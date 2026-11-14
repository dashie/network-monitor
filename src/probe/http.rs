//! HTTP and HTTPS probing.
//!
//! One `GET`, one status line, nothing else: no redirects, no body, no client state. The URL
//! parser lives here with the request it feeds.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};

use super::{Endpoint, Probe, ProbeError, Scheme, Verdict};

/// The largest response head accepted, so a chatty endpoint cannot grow the buffer.
const MAX_HEAD: usize = 8 * 1024;

/// The parts of an HTTP URL the probe needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UrlParts {
    pub scheme: Scheme,
    pub host: String,
    pub port: u16,
    pub target: String,
}

/// Parse `http://host[:port][/path][?query]`.
///
/// No credentials, no fragment, no IPv6 literal: the URL becomes the identity of the
/// target, so only what the probe can act on is accepted.
pub fn parse_url(text: &str) -> Result<UrlParts, String> {
    let text = text.trim();
    if text.chars().any(char::is_whitespace) {
        return Err("the URL contains whitespace".to_string());
    }

    let (scheme, rest) = match text.strip_prefix("http://") {
        Some(rest) => (Scheme::Http, rest),
        None => match text.strip_prefix("https://") {
            Some(rest) => (Scheme::Https, rest),
            None => return Err("the URL must start with http:// or https://".to_string()),
        },
    };

    if rest.contains('#') {
        return Err("a fragment is not part of a request and is not supported".to_string());
    }

    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    let (authority, tail) = rest.split_at(end);
    if authority.is_empty() {
        return Err("the URL has no host".to_string());
    }
    if authority.contains('@') {
        return Err("credentials in the URL are not supported".to_string());
    }
    if !authority.is_ascii() {
        return Err("the host must be written in ASCII".to_string());
    }

    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (host, Some(port)),
        None => (authority, None),
    };
    if host.is_empty() {
        return Err("the URL has no host".to_string());
    }
    if host.contains(':') {
        return Err("IPv6 hosts are not supported in this version".to_string());
    }

    let port = match port {
        Some(text) => match text.parse::<u16>() {
            Ok(0) | Err(_) => return Err(format!("'{text}' is not a port between 1 and 65535")),
            Ok(port) => port,
        },
        None => scheme.default_port(),
    };

    let target = match tail {
        "" => "/".to_string(),
        tail if tail.starts_with('/') => tail.to_string(),
        tail => format!("/{tail}"),
    };

    Ok(UrlParts {
        scheme,
        host: host.to_string(),
        port,
        target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_scheme_host_port_and_target() {
        let url = parse_url("https://example.com/health?deep=1").unwrap();
        assert_eq!(url.scheme, Scheme::Https);
        assert_eq!(url.host, "example.com");
        assert_eq!(url.port, 443);
        assert_eq!(url.target, "/health?deep=1");
    }

    #[test]
    fn fills_the_default_port_and_the_root_path() {
        let url = parse_url("http://192.0.2.10").unwrap();
        assert_eq!(url.scheme, Scheme::Http);
        assert_eq!(url.host, "192.0.2.10");
        assert_eq!(url.port, 80);
        assert_eq!(url.target, "/");
    }

    #[test]
    fn keeps_an_explicit_port_and_a_query_only_target() {
        let url = parse_url("http://example.com:8080?ping=1").unwrap();
        assert_eq!(url.port, 8080);
        assert_eq!(url.target, "/?ping=1");
    }

    #[test]
    fn rejects_what_the_probe_cannot_use() {
        assert!(parse_url("ftp://example.com/").is_err());
        assert!(parse_url("http://").is_err());
        assert!(parse_url("http://user:secret@example.com/").is_err());
        assert!(parse_url("http://example.com/#top").is_err());
        assert!(parse_url("http://example.com:0/").is_err());
        assert!(parse_url("http://[::1]/").is_err());
        assert!(parse_url("http://exa mple.com/").is_err());
    }
}

/// Asks one HTTP endpoint per probe cycle.
#[derive(Default)]
pub struct HttpProbe;

impl HttpProbe {
    pub fn new() -> Self {
        HttpProbe
    }
}

impl Probe for HttpProbe {
    fn probe(
        &self,
        endpoint: &Endpoint,
        _interface: Option<&str>,
        timeout: Duration,
    ) -> Result<Verdict, ProbeError> {
        let Endpoint::Http {
            scheme,
            host,
            port,
            target,
            expect,
        } = endpoint
        else {
            return Err(ProbeError::local(format!(
                "the http prober cannot check a {} target",
                endpoint.method()
            )));
        };

        let started = Instant::now();
        match exchange(*scheme, host, *port, target, timeout) {
            Ok(status) if expect.matches(status) => Ok(Verdict::Up {
                rtt: started.elapsed(),
            }),
            Ok(_) => Ok(Verdict::Down),
            Err(Failure::Down) => Ok(Verdict::Down),
            Err(Failure::Local(reason)) => Err(ProbeError::local(reason)),
        }
    }
}

/// Why one exchange did not produce a status inside the expectation.
#[derive(Debug)]
enum Failure {
    /// The endpoint answered with a refusal, timed out, or failed the TLS handshake.
    Down,
    /// This host could not perform the check, or the answer is not an HTTP answer.
    Local(String),
}

fn exchange(
    scheme: Scheme,
    host: &str,
    port: u16,
    target: &str,
    timeout: Duration,
) -> Result<u16, Failure> {
    let address = resolve(host, port)?;
    let tcp = TcpStream::connect_timeout(&address, timeout).map_err(|error| failure(&error))?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(|error| failure(&error))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|error| failure(&error))?;

    let request = request(target, host, port, scheme);
    match scheme {
        Scheme::Http => {
            let mut stream = tcp;
            send(&mut stream, &request)?;
            read_status(&mut stream)
        }
        Scheme::Https => {
            let mut stream = tls(tcp, host)?;
            send(&mut stream, &request)?;
            read_status(&mut stream)
        }
    }
}

/// Write the request and push it out: a TLS stream buffers until it is flushed.
fn send(stream: &mut impl Write, request: &str) -> Result<(), Failure> {
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|error| failure(&error))
}

/// The address of the endpoint: a literal is used as written, a name is resolved.
fn resolve(host: &str, port: u16) -> Result<SocketAddr, Failure> {
    use std::net::{IpAddr, ToSocketAddrs};
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(address, port));
    }
    let mut addresses = (host, port)
        .to_socket_addrs()
        .map_err(|error| Failure::Local(format!("cannot resolve '{host}': {error}")))?;
    addresses
        .next()
        .ok_or_else(|| Failure::Local(format!("'{host}' resolves to no address")))
}

/// The request on the wire: no body, no cookies, no compression, connection closed after it.
fn request(target: &str, host: &str, port: u16, scheme: Scheme) -> String {
    let authority = if port == scheme.default_port() {
        host.to_string()
    } else {
        format!("{host}:{port}")
    };
    format!(
        "GET {target} HTTP/1.1\r\n\
         Host: {authority}\r\n\
         User-Agent: network-monitor/{}\r\n\
         Accept: */*\r\n\
         Connection: close\r\n\r\n",
        env!("CARGO_PKG_VERSION")
    )
}

/// Read the head and take the status out of its first line.
fn read_status(stream: &mut impl Read) -> Result<u16, Failure> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let read = stream.read(&mut chunk).map_err(|error| failure(&error))?;
        if read == 0 {
            return Err(Failure::Local(
                "the connection closed before a status line".to_string(),
            ));
        }
        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            return parse_status(&head);
        }
        if head.len() > MAX_HEAD {
            return Err(Failure::Local(format!(
                "the response head is larger than {MAX_HEAD} bytes"
            )));
        }
    }
}

/// Parse the status from `HTTP/1.1 200 OK` and the first line of a head.
fn parse_status(head: &[u8]) -> Result<u16, Failure> {
    let line_end = head
        .windows(2)
        .position(|window| window == b"\r\n")
        .ok_or_else(|| Failure::Local("the status line is unterminated".to_string()))?;
    let line = std::str::from_utf8(&head[..line_end])
        .map_err(|_| Failure::Local("the status line is not text".to_string()))?;
    let mut parts = line.split_whitespace();
    let protocol = parts.next().unwrap_or_default();
    if !protocol.starts_with("HTTP/") {
        return Err(Failure::Local(format!(
            "'{line}' is not an HTTP status line"
        )));
    }
    parts
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .filter(|status| (100..=599).contains(status))
        .ok_or_else(|| Failure::Local(format!("'{line}' carries no status code")))
}

/// A TLS stream over an already connected socket, with the handshake completed here so a
/// certificate problem is classified as `down` instead of surfacing mid-read.
fn tls(tcp: TcpStream, host: &str) -> Result<StreamOwned<ClientConnection, TcpStream>, Failure> {
    let config = tls_config()?;
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| Failure::Local(format!("'{host}' cannot be a TLS server name")))?;
    let mut connection = ClientConnection::new(Arc::clone(config), server_name)
        .map_err(|reason| Failure::Local(format!("cannot start TLS: {reason}")))?;
    let mut tcp = tcp;
    while connection.is_handshaking() {
        connection
            .complete_io(&mut tcp)
            .map_err(|error| failure(&error))?;
    }
    Ok(StreamOwned::new(connection, tcp))
}

/// The TLS configuration, built once: verification against the embedded roots, HTTP/1.1 only.
fn tls_config() -> Result<&'static Arc<ClientConfig>, Failure> {
    static CONFIG: OnceLock<Result<Arc<ClientConfig>, String>> = OnceLock::new();
    CONFIG
        .get_or_init(|| {
            let mut roots = RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let mut config = ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            Ok(Arc::new(config))
        })
        .as_ref()
        .map_err(|reason| Failure::Local(reason.clone()))
}

/// A refusal, a timeout, or a TLS alert is the answer of the endpoint; the rest is ours.
fn failure(error: &io::Error) -> Failure {
    match error.kind() {
        io::ErrorKind::ConnectionRefused
        | io::ErrorKind::TimedOut
        | // A socket read timeout surfaces as `WouldBlock` on Unix.
        io::ErrorKind::WouldBlock
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::UnexpectedEof
        | io::ErrorKind::InvalidData => Failure::Down,
        _ => Failure::Local(format!("cannot reach the endpoint: {error}")),
    }
}

#[cfg(test)]
mod exchange_tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc::{channel, Receiver};
    use std::thread;

    fn http(host: &str, port: u16, target: &str) -> Endpoint {
        Endpoint::Http {
            scheme: Scheme::Http,
            host: host.to_string(),
            port,
            target: target.to_string(),
            expect: crate::probe::StatusMatch::default(),
        }
    }

    fn expecting(endpoint: Endpoint, status: u16) -> Endpoint {
        let Endpoint::Http {
            scheme,
            host,
            port,
            target,
            ..
        } = endpoint
        else {
            unreachable!("the helper builds an HTTP endpoint")
        };
        Endpoint::Http {
            scheme,
            host,
            port,
            target,
            expect: crate::probe::StatusMatch {
                classes: Vec::new(),
                codes: vec![status],
            },
        }
    }

    /// A server answering one request, returning the request text it received.
    fn server(response: &'static [u8]) -> (u16, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().unwrap().port();
        let (sender, receiver) = channel();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = Vec::new();
                let mut chunk = [0u8; 512];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&chunk[..read]),
                    }
                }
                let _ = sender.send(String::from_utf8_lossy(&request).into_owned());
                let _ = stream.write_all(response);
            }
        });
        (port, receiver)
    }

    #[test]
    fn the_request_carries_the_path_and_the_authority() {
        let (port, request) = server(b"HTTP/1.1 204 No Content\r\n\r\n");
        let verdict = HttpProbe::new()
            .probe(
                &http("127.0.0.1", port, "/health?deep=1"),
                None,
                Duration::from_secs(1),
            )
            .expect("a local exchange");

        assert!(matches!(verdict, Verdict::Up { .. }), "got {verdict:?}");
        let request = request.recv().expect("the request was sent");
        assert!(
            request.starts_with("GET /health?deep=1 HTTP/1.1\r\n"),
            "unexpected request: {request}"
        );
        assert!(
            request.contains(&format!("Host: 127.0.0.1:{port}\r\n")),
            "unexpected request: {request}"
        );
        assert!(request.contains("Connection: close"));
        assert!(request.contains("User-Agent: network-monitor/"));
    }

    #[test]
    fn an_expected_status_is_up() {
        let (port, _) = server(b"HTTP/1.1 204 No Content\r\n\r\n");
        let endpoint = expecting(http("127.0.0.1", port, "/"), 204);

        let verdict = HttpProbe::new()
            .probe(&endpoint, None, Duration::from_secs(1))
            .expect("a local exchange");

        assert!(matches!(verdict, Verdict::Up { .. }));
    }

    #[test]
    fn a_status_outside_the_expectation_is_down() {
        let (port, _) = server(b"HTTP/1.1 500 Internal Server Error\r\n\r\n");

        let verdict = HttpProbe::new()
            .probe(&http("127.0.0.1", port, "/"), None, Duration::from_secs(1))
            .expect("a local exchange");

        assert_eq!(verdict, Verdict::Down);
    }

    #[test]
    fn an_explicit_expectation_accepts_the_status_it_names() {
        let (port, _) = server(b"HTTP/1.1 500 Internal Server Error\r\n\r\n");
        let endpoint = expecting(http("127.0.0.1", port, "/"), 500);

        let verdict = HttpProbe::new()
            .probe(&endpoint, None, Duration::from_secs(1))
            .expect("a local exchange");

        assert!(matches!(verdict, Verdict::Up { .. }));
    }

    #[test]
    fn a_refused_connection_is_down() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
            listener.local_addr().unwrap().port()
        };

        let verdict = HttpProbe::new()
            .probe(&http("127.0.0.1", port, "/"), None, Duration::from_secs(1))
            .expect("a refusal is a verdict");

        assert_eq!(verdict, Verdict::Down);
    }

    #[test]
    fn a_silent_server_times_out_as_down() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                thread::sleep(Duration::from_millis(300));
                drop(stream);
            }
        });

        let verdict = HttpProbe::new()
            .probe(
                &http("127.0.0.1", port, "/"),
                None,
                Duration::from_millis(50),
            )
            .expect("a timeout is a verdict");

        assert_eq!(verdict, Verdict::Down);
    }

    #[test]
    fn an_answer_that_is_not_http_is_a_local_failure() {
        let (port, _) = server(b"this is not http\r\n\r\n");

        let error = HttpProbe::new()
            .probe(&http("127.0.0.1", port, "/"), None, Duration::from_secs(1))
            .expect_err("a malformed answer cannot be a verdict");

        assert!(
            error.to_string().contains("status line"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn a_head_larger_than_the_cap_is_refused() {
        static HUGE: &[u8] = &[b'x'; MAX_HEAD + 1];
        let (port, _) = server(HUGE);

        let error = HttpProbe::new()
            .probe(&http("127.0.0.1", port, "/"), None, Duration::from_secs(1))
            .expect_err("an endless head cannot be a verdict");

        assert!(
            error.to_string().contains("larger than"),
            "unexpected: {error}"
        );
    }

    #[test]
    fn the_status_is_read_from_the_first_line() {
        assert_eq!(
            parse_status(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap(),
            204
        );
        assert_eq!(parse_status(b"HTTP/1.0 301 Moved\r\n\r\n").unwrap(), 301);
        assert!(parse_status(b"HTTP/1.1\r\n\r\n").is_err());
        assert!(parse_status(b"HTTP/1.1 999 Nope\r\n\r\n").is_err());
        assert!(parse_status(b"HTTP/1.1 200 OK").is_err());
    }

    #[test]
    fn another_method_is_refused() {
        let endpoint = Endpoint::Icmp {
            address: std::net::Ipv4Addr::LOCALHOST,
        };
        assert!(HttpProbe::new()
            .probe(&endpoint, None, Duration::from_millis(100))
            .is_err());
    }

    /// Manual check: a real certificate chain against the embedded roots.
    #[test]
    #[ignore = "needs outbound HTTPS"]
    fn a_public_https_endpoint_is_up() {
        let endpoint = Endpoint::Http {
            scheme: Scheme::Https,
            host: "example.com".to_string(),
            port: 443,
            target: "/".to_string(),
            expect: crate::probe::StatusMatch::default(),
        };

        let verdict = HttpProbe::new()
            .probe(&endpoint, None, Duration::from_secs(10))
            .expect("a network exchange");

        assert!(matches!(verdict, Verdict::Up { .. }), "got {verdict:?}");
    }
}
