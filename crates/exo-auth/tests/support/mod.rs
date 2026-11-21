//! An `Auth` against an in-process gmail-mock, and a scripted browser.

#![allow(dead_code)] // Each test binary uses a different part of this module.

use std::io;
use std::sync::Arc;

use exo_auth::{Auth, ClientConfig, MemorySecrets};
use gmail_mock::{Config, MockServer};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{Notify, oneshot};
use url::Url;

pub const CLIENT_ID: &str = "exo-test.apps.example";
pub const CLIENT_SECRET: &str = "GOCSPX-test";
/// The mock's account.
pub const EMAIL: &str = "maya@brightmoor.co";

pub struct Harness {
    pub server: MockServer,
    pub secrets: Arc<MemorySecrets>,
    pub dir: tempfile::TempDir,
    pub auth: Auth,
}

impl Harness {
    pub async fn new() -> Self {
        let server = MockServer::start(Config::default()).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let secrets = Arc::new(MemorySecrets::default());
        let auth = open(&server, &dir, &secrets);
        Self {
            server,
            secrets,
            dir,
            auth,
        }
    }

    /// The same folder and secrets opened again, as on the next launch.
    pub fn relaunch(&self) -> Auth {
        open(&self.server, &self.dir, &self.secrets)
    }

    /// Signs the mock's account in through a browser that approves.
    pub async fn sign_in(&self) -> exo_auth::Account {
        let (open, page) = browser(Clone::clone);
        let account = self.auth.sign_in(None, open).await.unwrap();
        assert_eq!(page.await.unwrap().status, 200);
        account
    }

    /// The OAuth client the app uses, to act on its grants behind its back.
    pub fn oauth(&self) -> gmail::oauth::OAuthClient {
        gmail::oauth::OAuthClient::new(
            self.server.endpoints().oauth,
            CLIENT_ID,
            Some(CLIENT_SECRET.into()),
        )
    }

    /// Revokes the stored refresh token at the mock, as the user would from their Google account.
    pub async fn revoke_at_google(&self) {
        let grant = exo_auth::SecretStore::get(&*self.secrets, EMAIL)
            .unwrap()
            .unwrap();
        self.oauth().revoke(&grant).await.unwrap();
    }

    /// Moves the mock's clock `hours` ahead.
    pub async fn advance_clock(&self, hours: i64) {
        let http = reqwest::Client::new();
        let clock: serde_json::Value = http
            .get(self.server.url().join("__mock/clock").unwrap())
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let now: jiff::Timestamp = clock["now"].as_str().unwrap().parse().unwrap();
        let later = now
            .checked_add(jiff::SignedDuration::from_hours(hours))
            .unwrap();
        let response = http
            .post(self.server.url().join("__mock/clock").unwrap())
            .json(&serde_json::json!({ "now": later.to_string() }))
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }

    /// Makes the next request to `path` fail.
    pub async fn fault(&self, fault: serde_json::Value) {
        let response = reqwest::Client::new()
            .post(self.server.url().join("__mock/faults").unwrap())
            .json(&fault)
            .send()
            .await
            .unwrap();
        assert!(response.status().is_success());
    }
}

fn open(server: &MockServer, dir: &tempfile::TempDir, secrets: &Arc<MemorySecrets>) -> Auth {
    let client = ClientConfig::new(CLIENT_ID, CLIENT_SECRET, server.endpoints());
    Auth::open(client, dir.path(), secrets.clone()).unwrap()
}

/// Google's token endpoint, holding each request until [`answer`](Self::answer) lets it through
/// with a fixed body, so a test can act while a refresh is in flight. Revocations succeed at once.
pub struct HeldGoogle {
    pub endpoints: gmail::Endpoints,
    arrived: Arc<Notify>,
    release: Arc<Notify>,
}

impl HeldGoogle {
    /// Answers token requests with `body` (HTTP 200).
    pub async fn start(body: &'static str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let (arrived, release) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let (on_arrival, released) = (arrived.clone(), release.clone());
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (on_arrival, released) = (on_arrival.clone(), released.clone());
                tokio::spawn(async move {
                    let request = read_request(&mut stream).await;
                    let body = if request.starts_with("POST /revoke") {
                        "{}"
                    } else {
                        on_arrival.notify_one();
                        released.notified().await;
                        body
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        Self {
            endpoints: gmail::Endpoints::mock(&origin),
            arrived,
            release,
        }
    }

    /// The accounts in `harness`'s folder and secrets, talking to this endpoint.
    pub fn auth(&self, harness: &Harness) -> Auth {
        let client = ClientConfig::new(CLIENT_ID, CLIENT_SECRET, self.endpoints.clone());
        Auth::open(client, harness.dir.path(), harness.secrets.clone()).unwrap()
    }

    /// Waits for a token request.
    pub async fn arrived(&self) {
        self.arrived.notified().await;
    }

    /// Answers one token request.
    pub fn answer(&self) {
        self.release.notify_one();
    }
}

/// Reads a request's head and body, returning it as text.
async fn read_request(stream: &mut TcpStream) -> String {
    let mut request = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let read = stream.read(&mut chunk).await.unwrap();
        request.extend_from_slice(&chunk[..read]);
        let text = String::from_utf8_lossy(&request).into_owned();
        let complete = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
            let length = head
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")?
                        .parse()
                        .ok()
                })
                .unwrap_or(0);
            body.len() >= length
        });
        if complete || read == 0 {
            return text;
        }
    }
}

/// What the browser ended up showing.
#[derive(Debug)]
pub struct Page {
    /// The consent URL Exo opened.
    pub opened: Url,
    pub status: u16,
    pub text: String,
}

/// A browser for `Auth::sign_in`: given the consent URL, it loads the URL `visit` makes of it
/// and follows redirects (the mock's consent answers with one back to the loopback listener).
/// The page it lands on arrives on the receiver.
pub fn browser(
    visit: impl FnOnce(&Url) -> Url + Send + 'static,
) -> (
    impl FnOnce(&Url) -> io::Result<()> + Send,
    oneshot::Receiver<Page>,
) {
    let (sender, page) = oneshot::channel();
    let open = move |url: &Url| {
        let (opened, target) = (url.clone(), visit(url));
        tokio::spawn(async move {
            let response = reqwest::get(target).await.unwrap();
            let status = response.status().as_u16();
            let text = response.text().await.unwrap();
            let _ = sender.send(Page {
                opened,
                status,
                text,
            });
        });
        Ok(())
    };
    (open, page)
}

/// A query parameter of `url`.
pub fn param(url: &Url, name: &str) -> String {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .unwrap_or_else(|| panic!("{url} has no {name}"))
}

/// The loopback redirect Google would send for consent URL `url`, with `query` appended.
pub fn redirect(url: &Url, query: &str) -> Url {
    Url::parse(&format!("{}/?{query}", param(url, "redirect_uri"))).unwrap()
}
