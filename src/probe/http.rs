//! HTTP and HTTPS probing.
//!
//! One `GET`, one status line, nothing else: no redirects, no body, no client state. The URL
//! parser lives here with the request it feeds.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use rustls::pki_types::ServerName;
use rustls::{ClientConnection, StreamOwned};

use super::{Code, Details, Endpoint, Failure, Probe, ProbeError, Scheme, Verdict};

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
            return Err(ProbeError::of(
                Code::ProbeWrongMethod,
                &Details {
                    method: Some("http"),
                    other: Some(endpoint.method().as_str()),
                    ..Details::default()
                },
            ));
        };

        let started = Instant::now();
        match exchange(*scheme, host, *port, target, timeout) {
            Ok(status) if expect.matches(status) => Ok(Verdict::Up {
                rtt: started.elapsed(),
            }),
            Ok(status) => {
                let url = endpoint.to_string();
                let expected = expect.specs().join(", ");
                Ok(Verdict::Down(Failure::new(
                    Code::HttpUnexpectedStatus,
                    &Details {
                        url: Some(&url),
                        status: Some(status),
                        expect: Some(&expected),
                        ..Details::default()
                    },
                )))
            }
            Err(Unreachable::Down(failure)) => Ok(Verdict::Down(failure)),
            Err(Unreachable::Local(error)) => Err(error),
        }
    }
}

/// Why one exchange did not produce a status inside the expectation.
#[derive(Debug)]
enum Unreachable {
    /// The endpoint answered with a refusal, a reset, a timeout, or a failed TLS handshake.
    Down(Failure),
    /// This host could not perform the check, or the answer is not an HTTP answer.
    Local(ProbeError),
}

/// Where the exchange is happening, so every failure it reports can name it.
#[derive(Debug, Clone, Copy)]
struct Venue<'a> {
    scheme: Scheme,
    host: &'a str,
    port: u16,
    target: &'a str,
}

impl Venue<'_> {
    /// The URL as the target key states it: the default port is left out.
    fn url(&self) -> String {
        let mut url = format!("{}://{}", self.scheme, self.host);
        if self.port != self.scheme.default_port() {
            url.push_str(&format!(":{}", self.port));
        }
        url.push_str(self.target);
        url
    }
}

fn exchange(
    scheme: Scheme,
    host: &str,
    port: u16,
    target: &str,
    timeout: Duration,
) -> Result<u16, Unreachable> {
    let venue = Venue {
        scheme,
        host,
        port,
        target,
    };
    let address = resolve(host, port)?;
    let tcp =
        TcpStream::connect_timeout(&address, timeout).map_err(|error| failure(&error, venue))?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(|error| failure(&error, venue))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|error| failure(&error, venue))?;

    let request = request(target, host, port, scheme);
    match scheme {
        Scheme::Http => {
            let mut stream = tcp;
            send(&mut stream, &request, venue)?;
            read_status(&mut stream, venue)
        }
        Scheme::Https => {
            let mut stream = tls(tcp, venue)?;
            send(&mut stream, &request, venue)?;
            read_status(&mut stream, venue)
        }
    }
}

/// Write the request and push it out: a TLS stream buffers until it is flushed.
fn send(stream: &mut impl Write, request: &str, venue: Venue<'_>) -> Result<(), Unreachable> {
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|error| failure(&error, venue))
}

/// The address of the endpoint: a literal is used as written, a name is resolved.
fn resolve(host: &str, port: u16) -> Result<SocketAddr, Unreachable> {
    use std::net::{IpAddr, ToSocketAddrs};
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(address, port));
    }
    let mut addresses = (host, port).to_socket_addrs().map_err(|error| {
        Unreachable::Local(ProbeError::of(
            Code::HttpResolve,
            &Details {
                host: Some(host),
                reason: Some(&error.to_string()),
                ..Details::default()
            },
        ))
    })?;
    addresses.next().ok_or_else(|| {
        Unreachable::Local(ProbeError::of(
            Code::HttpResolve,
            &Details {
                host: Some(host),
                reason: Some("it resolves to no address"),
                ..Details::default()
            },
        ))
    })
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
fn read_status(stream: &mut impl Read, venue: Venue<'_>) -> Result<u16, Unreachable> {
    let mut head = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        let read = stream
            .read(&mut chunk)
            .map_err(|error| failure(&error, venue))?;
        if read == 0 {
            return Err(malformed(
                venue,
                "the connection closed before a status line",
            ));
        }
        head.extend_from_slice(&chunk[..read]);
        if head.windows(4).any(|window| window == b"\r\n\r\n") {
            return parse_status(&head, venue);
        }
        if head.len() > MAX_HEAD {
            let url = venue.url();
            return Err(Unreachable::Local(ProbeError::of(
                Code::HttpHeadLimit,
                &Details {
                    url: Some(&url),
                    expect: Some("8 KiB"),
                    ..Details::default()
                },
            )));
        }
    }
}

/// Parse the status from `HTTP/1.1 200 OK` and the first line of a head.
fn parse_status(head: &[u8], venue: Venue<'_>) -> Result<u16, Unreachable> {
    let line_end = head
        .windows(2)
        .position(|window| window == b"\r\n")
        .ok_or_else(|| malformed(venue, "the status line is unterminated"))?;
    let line = std::str::from_utf8(&head[..line_end])
        .map_err(|_| malformed(venue, "the status line is not text"))?;
    let mut parts = line.split_whitespace();
    let protocol = parts.next().unwrap_or_default();
    if !protocol.starts_with("HTTP/") {
        return Err(malformed(
            venue,
            &format!("'{line}' is not an HTTP status line"),
        ));
    }
    parts
        .next()
        .and_then(|code| code.parse::<u16>().ok())
        .filter(|status| (100..=599).contains(status))
        .ok_or_else(|| malformed(venue, &format!("'{line}' carries no status code")))
}

/// A TLS stream over an already connected socket, with the handshake completed here so a
/// certificate problem is classified as `down` instead of surfacing mid-read.
fn tls(
    tcp: TcpStream,
    venue: Venue<'_>,
) -> Result<StreamOwned<ClientConnection, TcpStream>, Unreachable> {
    let config = crate::tls::config().map_err(|reason| {
        Unreachable::Local(ProbeError::of(
            Code::HttpTlsInit,
            &Details {
                reason: Some(&reason),
                ..Details::default()
            },
        ))
    })?;
    let server_name = ServerName::try_from(venue.host.to_string()).map_err(|_| {
        Unreachable::Local(ProbeError::of(
            Code::HttpTlsInit,
            &Details {
                reason: Some(&format!("'{}' is not a server name", venue.host)),
                ..Details::default()
            },
        ))
    })?;
    let mut connection =
        ClientConnection::new(Arc::clone(config), server_name).map_err(|reason| {
            Unreachable::Local(ProbeError::of(
                Code::HttpTlsInit,
                &Details {
                    reason: Some(&reason.to_string()),
                    ..Details::default()
                },
            ))
        })?;
    let mut tcp = tcp;
    while connection.is_handshaking() {
        connection
            .complete_io(&mut tcp)
            .map_err(|error| handshake(&error, venue))?;
    }
    Ok(StreamOwned::new(connection, tcp))
}

/// A refusal, a timeout, or a reset is the answer of the endpoint; the rest is ours.
fn failure(error: &io::Error, venue: Venue<'_>) -> Unreachable {
    let code = match error.kind() {
        io::ErrorKind::ConnectionRefused => Code::TcpRefused,
        io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::UnexpectedEof
        | io::ErrorKind::InvalidData => Code::TcpReset,
        // A socket read timeout surfaces as `WouldBlock` on Unix.
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Code::TcpTimeout,
        _ => {
            return Unreachable::Local(ProbeError::of(
                Code::TcpConnect,
                &Details {
                    host: Some(venue.host),
                    port: Some(venue.port),
                    reason: Some(&error.to_string()),
                    ..Details::default()
                },
            ))
        }
    };
    Unreachable::Down(at(code, venue, &error.to_string()))
}

/// A TLS handshake that did not complete is the answer of the endpoint, never ours.
fn handshake(error: &io::Error, venue: Venue<'_>) -> Unreachable {
    Unreachable::Down(at(Code::HttpTls, venue, &error.to_string()))
}

/// An answer that is not an HTTP answer.
fn malformed(venue: Venue<'_>, reason: &str) -> Unreachable {
    let url = venue.url();
    Unreachable::Local(ProbeError::of(
        Code::HttpMalformed,
        &Details {
            url: Some(&url),
            reason: Some(reason),
            ..Details::default()
        },
    ))
}

/// A failure that names where it happened.
fn at(code: Code, venue: Venue<'_>, reason: &str) -> Failure {
    Failure::new(
        code,
        &Details {
            host: Some(venue.host),
            port: Some(venue.port),
            reason: Some(reason),
            ..Details::default()
        },
    )
}

#[cfg(test)]
mod exchange_tests {
    use super::*;

    /// Where a parser test believes it is.
    fn venue() -> Venue<'static> {
        Venue {
            scheme: Scheme::Http,
            host: "127.0.0.1",
            port: 80,
            target: "/",
        }
    }

    /// The code of a `down` verdict, so an assertion states what the endpoint answered.
    fn code_of(verdict: &Verdict) -> &'static str {
        match verdict {
            Verdict::Down(failure) => failure.code(),
            other => panic!("expected a down verdict, got {other:?}"),
        }
    }
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

        assert_eq!(code_of(&verdict), "HTTP.UNEXPECTED_STATUS");
        match &verdict {
            Verdict::Down(failure) => assert_eq!(
                failure.message(),
                "http://127.0.0.1:PORT_NOW/ answered 500, outside 2xx, 3xx"
                    .replace("PORT_NOW", &port.to_string())
            ),
            other => panic!("expected a down verdict, got {other:?}"),
        }
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

        assert_eq!(code_of(&verdict), "TCP.REFUSED");
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

        assert_eq!(code_of(&verdict), "TCP.TIMEOUT");
    }

    /// A server that speaks plain bytes to a TLS client: the handshake cannot complete.
    fn silent_tls_server() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().unwrap().port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                // Not a TLS record: the client rejects the answer of the endpoint.
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
                thread::sleep(Duration::from_millis(200));
            }
        });
        port
    }

    #[test]
    fn a_name_that_does_not_resolve_states_its_code() {
        let error = HttpProbe::new()
            .probe(
                &http("network-monitor.invalid", 80, "/"),
                None,
                Duration::from_secs(1),
            )
            .expect_err("a name that does not resolve is our failure");

        assert_eq!(error.code(), "HTTP.RESOLVE");
    }

    #[test]
    fn a_handshake_that_fails_states_its_code() {
        let port = silent_tls_server();
        let endpoint = Endpoint::Http {
            scheme: Scheme::Https,
            host: "127.0.0.1".to_string(),
            port,
            target: "/".to_string(),
            expect: crate::probe::StatusMatch::default(),
        };

        let verdict = HttpProbe::new()
            .probe(&endpoint, None, Duration::from_secs(1))
            .expect("a failed handshake is the answer of the endpoint, not our failure");

        assert_eq!(code_of(&verdict), "HTTP.TLS");
        match &verdict {
            Verdict::Down(failure) => assert!(
                failure.message().contains("TLS handshake with 127.0.0.1"),
                "unexpected: {}",
                failure.message()
            ),
            other => panic!("expected a down verdict, got {other:?}"),
        }
    }

    #[test]
    fn an_answer_that_is_not_http_is_a_local_failure() {
        let (port, _) = server(b"this is not http\r\n\r\n");

        let error = HttpProbe::new()
            .probe(&http("127.0.0.1", port, "/"), None, Duration::from_secs(1))
            .expect_err("a malformed answer cannot be a verdict");

        assert_eq!(error.code(), "HTTP.MALFORMED");
        assert!(
            error
                .message()
                .starts_with(&format!("http://127.0.0.1:{port}/ answered")),
            "the message names the URL, not the host: {error}"
        );
    }

    #[test]
    fn a_head_larger_than_the_cap_is_refused() {
        static HUGE: &[u8] = &[b'x'; MAX_HEAD + 1];
        let (port, _) = server(HUGE);

        let error = HttpProbe::new()
            .probe(&http("127.0.0.1", port, "/"), None, Duration::from_secs(1))
            .expect_err("an endless head cannot be a verdict");

        assert_eq!(error.code(), "HTTP.HEAD_LIMIT");
        assert!(
            error
                .message()
                .starts_with(&format!("http://127.0.0.1:{port}/ sent a head")),
            "the message names the URL, not the host: {error}"
        );
    }

    #[test]
    fn the_status_is_read_from_the_first_line() {
        assert_eq!(
            parse_status(b"HTTP/1.1 204 No Content\r\n\r\n", venue()).unwrap(),
            204
        );
        assert_eq!(
            parse_status(b"HTTP/1.0 301 Moved\r\n\r\n", venue()).unwrap(),
            301
        );
        assert!(parse_status(b"HTTP/1.1\r\n\r\n", venue()).is_err());
        assert!(parse_status(b"HTTP/1.1 999 Nope\r\n\r\n", venue()).is_err());
        assert!(parse_status(b"HTTP/1.1 200 OK", venue()).is_err());
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
