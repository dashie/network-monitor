//! TCP connect probing.
//!
//! A target answers when the handshake completes: something listens on the port. Nothing
//! is sent, so the check never depends on an application protocol, and no data reaches the
//! service.

use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::time::{Duration, Instant};

use socket2::{Domain, Protocol, Socket, Type};

use super::interface as net_interface;
use super::{Code, Details, Endpoint, Failure, Probe, ProbeError, SimulatedEpisode, Verdict};

/// Opens one TCP connection per probe cycle.
#[derive(Default)]
pub struct TcpProbe;

impl TcpProbe {
    pub fn new() -> Self {
        TcpProbe
    }
}

impl Probe for TcpProbe {
    fn probe(
        &self,
        endpoint: &Endpoint,
        interface: Option<&str>,
        timeout: Duration,
    ) -> Result<Verdict, ProbeError> {
        let Endpoint::Tcp { host, port } = endpoint else {
            return Err(ProbeError::of(
                Code::ProbeWrongMethod,
                &Details {
                    method: Some("tcp"),
                    other: Some(endpoint.method().as_str()),
                    ..Details::default()
                },
            ));
        };

        let address = resolve(host, *port)?;
        let started = Instant::now();
        match connect(address, interface, timeout) {
            Ok(()) => Ok(Verdict::Up {
                rtt: started.elapsed(),
            }),
            Err(Unreachable::Down(failure)) => Ok(Verdict::Down(failure)),
            Err(Unreachable::Local(error)) => Err(error),
        }
    }
}

/// An address literal is used as written; a name is resolved once per probe cycle.
fn resolve(host: &str, port: u16) -> Result<SocketAddr, ProbeError> {
    if let Ok(address) = host.parse::<IpAddr>() {
        return Ok(SocketAddr::new(address, port));
    }

    let mut addresses = (host, port).to_socket_addrs().map_err(|error| {
        ProbeError::of(
            Code::TcpResolve,
            &Details {
                host: Some(host),
                reason: Some(&error.to_string()),
                ..Details::default()
            },
        )
    })?;
    addresses.next().ok_or_else(|| {
        ProbeError::of(
            Code::TcpResolve,
            &Details {
                host: Some(host),
                reason: Some("it resolves to no address"),
                ..Details::default()
            },
        )
    })
}

/// Why a connection did not open.
#[derive(Debug)]
enum Unreachable {
    /// The target answered with a refusal, a reset, or stayed silent until the timeout.
    Down(Failure),
    /// This host could not perform the check at all.
    Local(ProbeError),
}

/// One socket per probe cycle, so the interface can be bound before connecting and the
/// socket itself can arbitrate an ambiguous failure.
fn connect(
    address: SocketAddr,
    interface: Option<&str>,
    timeout: Duration,
) -> Result<(), Unreachable> {
    let socket = socket(address)?;
    if let Some(name) = interface {
        net_interface::bind_socket(&socket, name).map_err(|reason| {
            Unreachable::Local(ProbeError::of(
                Code::TcpInterface,
                &Details {
                    interface: Some(name),
                    reason: Some(&reason),
                    ..Details::default()
                },
            ))
        })?;
    }
    match socket.connect_timeout(&address.into(), timeout) {
        Ok(()) => Ok(()),
        Err(error) => outcome(&socket, &error, address, timeout),
    }
}

/// A socket whose address family follows the address it will connect to.
fn socket(address: SocketAddr) -> Result<Socket, Unreachable> {
    let domain = if address.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    Socket::new(domain, Type::STREAM, Some(Protocol::TCP)).map_err(|error| {
        let host = address.ip().to_string();
        Unreachable::Local(ProbeError::of(
            Code::TcpSocket,
            &Details {
                host: Some(&host),
                port: Some(address.port()),
                reason: Some(&error.to_string()),
                ..Details::default()
            },
        ))
    })
}

/// A refusal or a timeout is the answer of the target; every other error is ours.
///
/// The platform reports a peer that closed right after accepting as a hang-up with no error
/// set at all, which is not a failure of this host: the socket decides, and a peer it is
/// connected to is the answer `up`.
///
/// The split is what INV-5 rests on: a missing capability or an exhausted descriptor table
/// must never be recorded as "the target is down". A connect interrupted by a signal stays a
/// local failure for the same reason: retrying it needs a fresh socket, because the state of
/// an interrupted one is unspecified, and a transient condition is never a verdict.
fn outcome(
    socket: &Socket,
    error: &io::Error,
    address: SocketAddr,
    timeout: Duration,
) -> Result<(), Unreachable> {
    if socket.peer_addr().is_ok() {
        return Ok(());
    }
    let code = match error.kind() {
        io::ErrorKind::ConnectionRefused => Code::TcpRefused,
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => Code::TcpReset,
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Code::TcpTimeout,
        _ => {
            let host = address.ip().to_string();
            return Err(Unreachable::Local(ProbeError::of(
                Code::TcpConnect,
                &Details {
                    host: Some(&host),
                    port: Some(address.port()),
                    reason: Some(&error.to_string()),
                    ..Details::default()
                },
            )));
        }
    };
    let host = address.ip().to_string();
    Err(Unreachable::Down(Failure::new(
        code,
        &Details {
            host: Some(&host),
            port: Some(address.port()),
            timeout: Some(timeout),
            ..Details::default()
        },
    )))
}

/// The simulated day of one TCP target.
///
/// The three answers a handshake gives when it does not complete: the port is closed, the
/// peer is not there, or it drops the connection once it has been made.
pub fn simulated_day(
    endpoint: &Endpoint,
    seed: u64,
    day_start_ms: i64,
    fault_rate: f64,
) -> Vec<SimulatedEpisode> {
    let Endpoint::Tcp { host, port } = endpoint else {
        // The dispatcher chooses by method; this line cannot be reached from it.
        return Vec::new();
    };
    let host = host.clone();
    let port = *port;
    super::simulated_episodes(
        endpoint,
        seed,
        day_start_ms,
        fault_rate,
        "tcp",
        |rng| rng.between(1_000, 80_000),
        |rng| {
            let stated = Details {
                host: Some(&host),
                port: Some(port),
                ..Details::default()
            };
            match rng.below(10) {
                0..=5 => Failure::new(Code::TcpRefused, &stated),
                6..=8 => Failure::new(
                    Code::TcpTimeout,
                    &Details {
                        timeout: Some(super::SIMULATED_TIMEOUT),
                        ..stated
                    },
                ),
                _ => Failure::new(Code::TcpReset, &stated),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    fn tcp(host: &str, port: u16) -> Endpoint {
        Endpoint::Tcp {
            host: host.to_string(),
            port,
        }
    }

    fn timeout() -> Duration {
        Duration::from_millis(1_000)
    }

    /// The code of a `down` verdict, so an assertion states what the target answered.
    fn code_of(verdict: &Verdict) -> &'static str {
        match verdict {
            Verdict::Down(failure) => failure.code(),
            other => panic!("expected a down verdict, got {other:?}"),
        }
    }

    /// A listener that keeps the accepted connection open until the probe has finished: a
    /// listener dropped first would close the peer mid-handshake and make the test a race.
    fn serve_until_released(listener: TcpListener) -> (mpsc::Sender<()>, thread::JoinHandle<()>) {
        let (release, released) = mpsc::channel();
        let handle = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("the connection arrives");
            let _ = released.recv();
            drop(stream);
        });
        (release, handle)
    }

    #[test]
    fn an_open_port_is_up() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().unwrap().port();
        let (release, accepting) = serve_until_released(listener);

        let verdict = TcpProbe::new()
            .probe(&tcp("127.0.0.1", port), None, timeout())
            .expect("a refused connection is a verdict, not an error");

        assert!(
            matches!(verdict, Verdict::Up { rtt } if rtt > Duration::ZERO),
            "expected an open port, got {verdict:?}"
        );
        release.send(()).unwrap();
        accepting.join().unwrap();
    }

    #[test]
    fn a_closed_port_is_down() {
        let port = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
            listener.local_addr().unwrap().port()
        };

        let verdict = TcpProbe::new()
            .probe(&tcp("127.0.0.1", port), None, timeout())
            .expect("a refusal is a verdict");

        assert_eq!(code_of(&verdict), "TCP.REFUSED");
    }

    #[test]
    fn a_name_that_does_not_resolve_is_a_local_error() {
        let error = TcpProbe::new()
            .probe(&tcp("uptime-monitor.invalid", 80), None, timeout())
            .expect_err("a name that does not resolve is our failure");

        assert_eq!(error.code(), "TCP.RESOLVE");
        assert!(error.message().contains("resolve"), "unexpected: {error}");
    }

    #[test]
    fn another_method_is_refused() {
        let endpoint = Endpoint::Icmp {
            address: std::net::Ipv4Addr::LOCALHOST,
        };
        let error = TcpProbe::new()
            .probe(&endpoint, None, timeout())
            .expect_err("a prober checks its own method");
        assert_eq!(error.code(), "PROBE.WRONG_METHOD");
        assert_eq!(error.message(), "the tcp prober cannot check a icmp target");
    }

    #[test]
    fn a_refusal_is_the_answer_of_the_target_and_the_rest_is_ours() {
        let address: SocketAddr = ([127, 0, 0, 1], 9).into();
        let failed = socket(address).expect("a socket that never connected");

        for (kind, code) in [
            (io::ErrorKind::ConnectionRefused, "TCP.REFUSED"),
            (io::ErrorKind::TimedOut, "TCP.TIMEOUT"),
            (io::ErrorKind::WouldBlock, "TCP.TIMEOUT"),
            (io::ErrorKind::ConnectionReset, "TCP.RESET"),
            (io::ErrorKind::ConnectionAborted, "TCP.RESET"),
        ] {
            match outcome(
                &failed,
                &io::Error::from(kind),
                SocketAddr::from(([127, 0, 0, 1], 443)),
                timeout(),
            ) {
                Err(Unreachable::Down(failure)) => assert_eq!(failure.code(), code, "{kind:?}"),
                other => panic!("{kind:?} must be a verdict, got {other:?}"),
            }
        }
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::AddrNotAvailable,
            io::ErrorKind::Interrupted,
            io::ErrorKind::Other,
        ] {
            match outcome(
                &failed,
                &io::Error::from(kind),
                SocketAddr::from(([127, 0, 0, 1], 443)),
                timeout(),
            ) {
                Err(Unreachable::Local(error)) => {
                    assert_eq!(error.code(), "TCP.CONNECT", "{kind:?}");
                }
                other => panic!("{kind:?} must be a local failure, got {other:?}"),
            }
        }
    }

    #[test]
    fn an_interface_that_does_not_exist_states_its_code() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let address = listener.local_addr().unwrap();

        let error = connect(address, Some("no-such-device0"), timeout())
            .expect_err("a device this host does not have cannot be bound");

        match error {
            Unreachable::Local(error) => {
                assert_eq!(error.code(), "TCP.INTERFACE");
                assert!(
                    error.message().contains("no-such-device0"),
                    "unexpected: {error}"
                );
            }
            other => panic!("expected a local failure, got {other:?}"),
        }
    }

    /// Regression: a peer that accepts and closes at once leaves the platform reporting a
    /// hang-up with no error set. The handshake completed, so the socket answers the probe.
    #[test]
    fn a_socket_whose_peer_closed_after_the_handshake_is_up() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let address = listener.local_addr().unwrap();
        let connected = socket(address).expect("a socket");
        connected.connect(&address.into()).expect("the handshake");

        let hang_up = io::Error::other("no error set after POLLHUP");

        assert!(
            outcome(&connected, &hang_up, address, timeout()).is_ok(),
            "a connected socket answers even when the platform raises a nameless error"
        );
    }

    /// The interface-bound path: the socket is created by hand and bound before connecting.
    #[test]
    #[cfg(unix)]
    fn a_socket_bound_to_an_interface_still_connects() {
        let name = if cfg!(target_os = "macos") {
            "lo0"
        } else {
            "lo"
        };
        let listener = TcpListener::bind("127.0.0.1:0").expect("a free port");
        let port = listener.local_addr().unwrap().port();
        let (release, accepting) = serve_until_released(listener);

        let verdict = TcpProbe::new()
            .probe(&tcp("127.0.0.1", port), Some(name), timeout())
            .expect("the loopback interface can be bound");

        assert!(
            matches!(verdict, Verdict::Up { .. }),
            "expected the local listener through {name}, got {verdict:?}"
        );
        release.send(()).unwrap();
        accepting.join().unwrap();
    }

    /// Manual check: a black-holed address must come back `down` after the timeout, not hang.
    #[test]
    #[ignore = "needs a route that black-holes TEST-NET-1"]
    fn a_silent_address_times_out_as_down() {
        let verdict = TcpProbe::new()
            .probe(&tcp("192.0.2.1", 81), None, Duration::from_millis(150))
            .expect("a timeout is a verdict");

        assert_eq!(code_of(&verdict), "TCP.TIMEOUT");
    }
}
