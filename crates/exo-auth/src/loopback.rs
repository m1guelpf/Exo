//! The loopback end of the redirect (RFC 8252, section 7.3): just enough HTTP/1.1 to read the
//! browser's `GET /?code=…&state=…` and answer it with a page.

use std::io;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinSet;
use url::Url;

use crate::Error;

/// How long a connection may take to send its request head. Browsers open spare connections
/// they never use, so each is read on its own and dropped when its time is up.
const READ_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest request head read. A redirect with a code and the scope list is about 1 KiB.
const MAX_HEAD: usize = 16 * 1024;

/// Copy for the browser page the sign-in ends on.
mod copy {
    /// Signed in.
    pub const SIGNED_IN: &str = "Thanks for allowing Exo!";
    /// The generic error.
    pub const FAILED: &str = "Sorry, something went wrong! Please try again later.";
    /// Fewer scopes granted than asked for.
    pub const MISSING_SCOPES: &str = "Please select all permissions when signing in with Google.\n\
                                      Exo requires these to manage your email.";
    /// The login attempt expired.
    pub const EXPIRED: &str = "Sorry, this login attempt has expired. Please try again.";
    /// No Gmail mailbox.
    pub const NO_MAILBOX: &str = "This Google Account does not appear to have a Gmail mailbox.";
}

/// Accepts connections until one carries this attempt's redirect: a request for `/` with
/// `code`, `error` or `state`, whose `state` is `state`. A redirect with another `state` (a
/// stale tab, or a page that found the port) is told the attempt expired, anything else
/// (`/favicon.ico`) gets a 404, and the wait goes on. Returns the redirect's URL and its
/// connection, to answer once sign-in finishes.
pub(crate) async fn receive(listener: &TcpListener, state: &str) -> io::Result<(Url, TcpStream)> {
    let mut reading = JoinSet::new();
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                reading.spawn(read_request(accepted?.0));
            }
            Some(read) = reading.join_next() => {
                let Ok(Ok((target, mut stream))) = read else { continue };
                if !is_redirect(&target) {
                    let _ = write(&mut stream, "404 Not Found", "").await;
                } else if param(&target, "state").as_deref() == Some(state) {
                    return Ok((target, stream));
                } else {
                    let _ = write(&mut stream, "400 Bad Request", &page(copy::EXPIRED)).await;
                }
            }
        }
    }
}

fn is_redirect(target: &Url) -> bool {
    target.path() == "/"
        && target
            .query_pairs()
            .any(|(name, _)| matches!(&*name, "code" | "error" | "state"))
}

/// The first value of `name` in `url`'s query.
pub(crate) fn param(url: &Url, name: &str) -> Option<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Reads a request head and returns its target, resolved against the loopback origin.
async fn read_request(mut stream: TcpStream) -> io::Result<(Url, TcpStream)> {
    let mut head = Vec::new();
    // The whole head, not just the request line: closing a socket with unread input resets the
    // connection, and the browser would show its own error instead of the page.
    let read_head = async {
        let mut chunk = [0; 2048];
        while !head.windows(4).any(|window| window == b"\r\n\r\n") {
            let read = stream.read(&mut chunk).await?;
            if read == 0 || head.len() + read > MAX_HEAD {
                return Err(io::ErrorKind::InvalidData.into());
            }
            head.extend_from_slice(&chunk[..read]);
        }
        Ok::<_, io::Error>(())
    };
    tokio::time::timeout(READ_TIMEOUT, read_head)
        .await
        .map_err(|_| io::ErrorKind::TimedOut)??;
    let line = head.split(|&byte| byte == b'\r').next().unwrap_or_default();
    let target = std::str::from_utf8(line)
        .ok()
        .and_then(|line| line.strip_prefix("GET "))
        .and_then(|rest| rest.split(' ').next())
        .and_then(|target| Url::parse("http://127.0.0.1/").ok()?.join(target).ok())
        .ok_or(io::ErrorKind::InvalidData)?;
    Ok((target, stream))
}

/// Answers the redirect with how sign-in went. The browser may have gone; that's fine.
pub(crate) async fn respond<T>(mut stream: TcpStream, result: &Result<T, Error>) {
    let (status, text) = match result {
        Ok(_) => ("200 OK", copy::SIGNED_IN),
        Err(error) => ("400 Bad Request", failure(error)),
    };
    if let Err(error) = write(&mut stream, status, &page(text)).await {
        log::debug!("answering the sign-in redirect: {error}");
    }
}

/// The browser page saying `text`, one of [`copy`]'s lines.
fn page(text: &str) -> String {
    format!(
        "<!DOCTYPE html><html lang=\"en\"><meta charset=\"utf-8\">\
         <meta name=\"color-scheme\" content=\"light dark\"><title>Exo</title>\
         <style>body{{margin:0 16px;min-height:90vh;display:grid;place-items:center;\
         font:16px/1.5 -apple-system,system-ui,sans-serif;text-align:center}}</style>\
         <p>{}</p></html>",
        text.replace('\n', "<br>")
    )
}

fn failure(error: &Error) -> &'static str {
    match error {
        Error::MissingScopes(_) => copy::MISSING_SCOPES,
        Error::NoMailbox => copy::NO_MAILBOX,
        _ => copy::FAILED,
    }
}

async fn write(stream: &mut TcpStream, status: &str, body: &str) -> io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.shutdown().await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(
        address: std::net::SocketAddr,
        request: &'static str,
    ) -> tokio::task::JoinHandle<String> {
        tokio::spawn(async move {
            let mut stream = TcpStream::connect(address).await.unwrap();
            stream.write_all(request.as_bytes()).await.unwrap();
            let mut response = String::new();
            stream.read_to_string(&mut response).await.unwrap();
            response
        })
    }

    #[tokio::test]
    async fn skips_idle_connections_and_other_paths() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let receiving = tokio::spawn(async move { receive(&listener, "s").await });
        // A spare connection that never sends anything mustn't hold up the redirect.
        let _idle = TcpStream::connect(address).await.unwrap();
        let favicon = send(address, "GET /favicon.ico HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        assert!(
            favicon.starts_with("HTTP/1.1 404 Not Found\r\n"),
            "{favicon}"
        );

        let redirect = send(
            address,
            "GET /?state=s&code=4%2F0A&scope=a+b HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: */*\r\n\r\n",
        );
        let (target, stream) = receiving.await.unwrap().unwrap();
        let query: Vec<_> = target.query_pairs().collect();
        assert_eq!(query[1], ("code".into(), "4/0A".into()));
        respond::<()>(stream, &Ok(())).await;
        let page = redirect.await.unwrap();
        assert!(page.starts_with("HTTP/1.1 200 OK\r\n"), "{page}");
        assert!(page.contains("Thanks for allowing Exo!"));
    }

    // Reads the accepted connection directly: the paused clock jumps ahead whenever the runtime
    // waits on the real socket, so timing a connection from before `accept` flakes under load.
    #[tokio::test(start_paused = true)]
    async fn a_request_trickling_in_is_dropped_when_its_time_is_up() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut writing = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        let (reading, _) = listener.accept().await.unwrap();
        // A byte every 4 s: no read waits long, but the head never ends.
        tokio::spawn(async move {
            for byte in b"GET /?state=s HTTP/1.1\r\nHost: x\r\n".iter().cycle() {
                if writing.write_all(&[*byte]).await.is_err() {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(4)).await;
            }
        });
        let started = tokio::time::Instant::now();
        let error = read_request(reading).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() <= READ_TIMEOUT + Duration::from_millis(1),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn each_failure_shows_its_copy() {
        assert_eq!(
            failure(&Error::MissingScopes(vec!["x".into()])),
            copy::MISSING_SCOPES
        );
        assert_eq!(failure(&Error::AccessDenied), copy::FAILED);
    }
}
