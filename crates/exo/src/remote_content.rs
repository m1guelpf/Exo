//! The HTTP client message bodies load remote content with (images, and the CSS they import).
//! Senders pick those URLs, so it only reaches the public internet: never this Mac, its local
//! network or link-local services. Otherwise an `<img src="http://192.168.1.1/…">` has Exo ask
//! a router or printer on your network for something each time the message shows, from inside
//! it. Names are resolved before they're checked and the connection goes to the addresses that
//! passed, so DNS can't sneak a private address in; addresses in URLs, redirects included, are
//! checked the same way.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

/// How many redirects a load follows (reqwest's default).
const MAX_REDIRECTS: usize = 10;

/// Why a load was refused: it would have reached this Mac or its network.
#[derive(Debug, thiserror::Error)]
#[error("{0} is not on the public internet")]
pub(crate) struct Private(String);

/// Why a load failed.
#[derive(Debug, thiserror::Error)]
pub(crate) enum Error {
    /// The URL doesn't parse.
    #[error(transparent)]
    Url(#[from] url::ParseError),
    /// It names a local address.
    #[error(transparent)]
    Private(#[from] Private),
    /// The request failed (a name resolving only to local addresses, or a redirect to one,
    /// fails here too).
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    /// The client couldn't be built, so nothing loads rather than loading unchecked.
    #[error("the remote content client couldn't be built")]
    Unavailable,
}

/// The client: every URL is checked before it's asked for, and names as they resolve.
#[derive(Clone, Debug)]
pub(crate) struct Client {
    http: Option<reqwest::Client>,
    loopback: bool,
}

impl Client {
    /// Gives up on a load after `timeout`. With `loopback` it may also load from this Mac, which
    /// only a Gmail served from it (gmail-mock) asks for.
    pub(crate) fn new(timeout: Duration, loopback: bool) -> Self {
        Self {
            http: http(timeout, loopback),
            loopback,
        }
    }

    /// Asks for `url`, refusing without connecting when it names a local address.
    pub(crate) async fn get(&self, url: &str) -> Result<reqwest::Response, Error> {
        let url = url::Url::parse(url)?;
        check(&url, self.loopback)?;
        let http = self.http.as_ref().ok_or(Error::Unavailable)?;
        Ok(http.get(url).send().await?)
    }
}

/// Whether Gmail is served from this Mac (gmail-mock): only then may remote content come from
/// it too.
pub(crate) fn serves_gmail_locally(endpoints: &gmail::Endpoints) -> bool {
    endpoints.gmail.host().is_some_and(|host| match host {
        url::Host::Domain(name) => name == "localhost",
        url::Host::Ipv4(ip) => ip.is_loopback(),
        url::Host::Ipv6(ip) => ip.is_loopback(),
    })
}

/// The HTTP client, whose resolver and redirects only reach the public internet.
fn http(timeout: Duration, loopback: bool) -> Option<reqwest::Client> {
    let policy = reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if let Err(refused) = check(attempt.url(), loopback) {
            attempt.error(refused)
        } else {
            attempt.follow()
        }
    });
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(policy)
        .dns_resolver(Arc::new(PublicResolver { loopback }))
        // A proxy would resolve the name itself, past the resolver's check.
        .no_proxy()
        .build()
        .inspect_err(|error| log::error!("building the remote content client: {error}"))
        .ok()
}

/// Refuses `url` when its host is an address that isn't public, before anything connects;
/// names are left to [`PublicResolver`].
fn check(url: &url::Url, loopback: bool) -> Result<(), Private> {
    let ip = match url.host() {
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip),
        Some(url::Host::Domain(_)) => return Ok(()),
        None => return Err(Private(url.to_string())),
    };
    if allowed(ip, loopback) {
        Ok(())
    } else {
        Err(Private(ip.to_string()))
    }
}

/// Resolves names with the system's resolver, keeping only the addresses [`allowed`] lets
/// through; a name with none left fails to resolve.
struct PublicResolver {
    loopback: bool,
}

impl Resolve for PublicResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let loopback = self.loopback;
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host.as_str(), 0))
                .await?
                .filter(|address| allowed(address.ip(), loopback))
                .collect();
            if addresses.is_empty() {
                return Err(Box::new(Private(host)) as _);
            }
            Ok(Box::new(addresses.into_iter()) as Addrs)
        })
    }
}

/// Whether remote content may come from `ip`: a public address, or with `loopback` this Mac's.
fn allowed(ip: IpAddr, loopback: bool) -> bool {
    (loopback && ip.is_loopback()) || is_public(ip)
}

/// Whether `ip` is on the public internet: not this machine (loopback, `0.0.0.0`), a private or
/// carrier-grade NAT network, link-local, multicast, broadcast or reserved. An IPv6 address that
/// carries an IPv4 one (mapped, or NAT64's `64:ff9b::/96`) is judged by it.
fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => match embedded_v4(ip) {
            Some(v4) => is_public_v4(v4),
            None => {
                let [first, second, ..] = ip.segments();
                !(ip.is_loopback()
                    || ip.is_unspecified()
                    || ip.is_multicast()
                    || ip.is_unique_local()
                    || ip.is_unicast_link_local()
                    // Site-local (`fec0::/10`), deprecated but still routed by some networks.
                    || first & 0xffc0 == 0xfec0
                    // Discard-only (`100::/64`).
                    || (first == 0x100 && second == 0))
            }
        },
    }
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [first, second, ..] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_unspecified()
        || ip.is_broadcast()
        || ip.is_multicast()
        // "This network" (`0.0.0.0/8`), carrier-grade NAT (`100.64.0.0/10`) and reserved
        // (`240.0.0.0/4`).
        || first == 0
        || (first == 100 && second & 0xc0 == 64)
        || first >= 240)
}

/// The IPv4 address an IPv6 one stands for: IPv4-mapped (`::ffff:a.b.c.d`), or NAT64's
/// well-known prefix (`64:ff9b::a.b.c.d`), which a NAT64 network routes to that address.
fn embedded_v4(ip: Ipv6Addr) -> Option<Ipv4Addr> {
    ip.to_ipv4_mapped().or_else(|| {
        let segments = ip.segments();
        (segments[..6] == [0x64, 0xff9b, 0, 0, 0, 0])
            .then(|| Ipv4Addr::from((u32::from(segments[6]) << 16) | u32::from(segments[7])))
    })
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        sync::atomic::{AtomicUsize, Ordering},
    };

    use super::*;

    #[test]
    fn only_public_addresses_pass() {
        let public = [
            "8.8.8.8",
            "93.184.216.34",
            "100.128.0.1",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
            "64:ff9b::808:808",
        ];
        for ip in public {
            assert!(is_public(ip.parse().expect("an address")), "{ip} is public");
        }
        let private = [
            "127.0.0.1",
            "127.8.9.10",
            "10.1.2.3",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.1",
            "169.254.169.254",
            "0.0.0.0",
            "0.1.2.3",
            "100.64.0.1",
            "100.127.255.254",
            "224.0.0.1",
            "255.255.255.255",
            "240.0.0.1",
            "::1",
            "::",
            "fe80::1",
            "fc00::1",
            "fd12:3456::1",
            "fec0::1",
            "ff02::1",
            "100::1",
            "::ffff:192.168.1.1",
            "::ffff:127.0.0.1",
            "64:ff9b::a00:1",
        ];
        for ip in private {
            assert!(!is_public(ip.parse().expect("an address")), "{ip} is local");
        }
        assert!(
            allowed("127.0.0.1".parse().expect("an address"), true),
            "gmail-mock's own machine"
        );
        assert!(
            !allowed("192.168.1.1".parse().expect("an address"), true),
            "but never the network"
        );
    }

    #[test]
    fn urls_naming_local_addresses_are_refused_before_connecting() {
        let check = |url: &str, loopback| check(&url.parse().expect("a URL"), loopback).is_ok();
        assert!(
            check("https://example.com/a.png", false),
            "names are the resolver's"
        );
        assert!(!check("http://192.168.0.1/admin", false));
        assert!(!check("http://[::ffff:10.0.0.1]/", false));
        assert!(!check("http://127.0.0.1:8080/", false));
        assert!(check("http://127.0.0.1:8080/", true));
    }

    /// A server on this Mac that counts the connections it takes and answers each request with
    /// `answer`.
    struct Server {
        address: SocketAddr,
        connections: Arc<AtomicUsize>,
    }

    impl Server {
        fn start(answer: String) -> Self {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
            let address = listener.local_addr().expect("an address");
            let connections = Arc::new(AtomicUsize::new(0));
            let counted = connections.clone();
            std::thread::spawn(move || {
                for mut stream in listener.incoming().map_while(Result::ok) {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let mut request = [0; 1024];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(answer.as_bytes());
                }
            });
            Self {
                address,
                connections,
            }
        }
    }

    #[tokio::test]
    async fn loads_never_reach_this_mac_by_address_or_by_name() {
        let server = Server::start(
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_owned(),
        );
        let port = server.address.port();
        let client = Client::new(Duration::from_secs(5), false);
        for url in [
            format!("http://127.0.0.1:{port}/a.png"),
            format!("http://localhost:{port}/a.png"),
            format!("http://[::ffff:127.0.0.1]:{port}/a.png"),
        ] {
            assert!(client.get(&url).await.is_err(), "{url} is refused");
        }
        assert_eq!(
            server.connections.load(Ordering::SeqCst),
            0,
            "nothing connected"
        );

        let mock = client_for_mock()
            .get(&format!("http://127.0.0.1:{port}/a.png"))
            .await
            .expect("gmail-mock's machine is allowed");
        assert_eq!(mock.text().await.expect("a body"), "ok");
    }

    #[tokio::test]
    async fn redirects_to_local_addresses_are_refused() {
        for location in [
            "http://169.254.169.254/latest/meta-data/",
            "http://192.168.1.1/",
            "http://[fe80::1]/",
        ] {
            let answer = format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let server = Server::start(answer);
            let response = client_for_mock()
                .get(&format!("http://{}/a.png", server.address))
                .await;
            let error = response.expect_err("the redirect is refused");
            assert!(
                matches!(&error, Error::Http(error) if error.is_redirect()),
                "{location}: {error}"
            );
        }
    }

    fn client_for_mock() -> Client {
        Client::new(Duration::from_secs(5), true)
    }
}
