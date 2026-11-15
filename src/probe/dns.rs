//! DNS probing with a hand-written query.
//!
//! No resolver library: the question is encoded here, the answer is decoded here, and one
//! UDP exchange is performed for the target. A target with no resolver configured asks the
//! system resolver instead, which is the only portable way to reach `/etc/resolv.conf`,
//! the Windows registry, or whatever the platform uses.

use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};

use super::{
    Code, Details, Endpoint, Failure, Probe, ProbeError, RecordType, SimulatedEpisode, Verdict,
};

/// The largest response accepted, so a hostile server cannot grow the buffer.
const MAX_RESPONSE: usize = 4_096;
/// A compression pointer must point strictly backwards; the cap stops a crafted chain.
const MAX_POINTER_JUMPS: usize = 32;

/// Asks a name resolution per probe cycle.
#[derive(Default)]
pub struct DnsProbe;

impl DnsProbe {
    pub fn new() -> Self {
        DnsProbe
    }
}

impl Probe for DnsProbe {
    fn probe(
        &self,
        endpoint: &Endpoint,
        _interface: Option<&str>,
        timeout: Duration,
    ) -> Result<Verdict, ProbeError> {
        let Endpoint::Dns {
            name,
            resolver,
            record,
        } = endpoint
        else {
            return Err(ProbeError::of(
                Code::ProbeWrongMethod,
                &Details {
                    method: Some("dns"),
                    other: Some(endpoint.method().as_str()),
                    ..Details::default()
                },
            ));
        };

        match resolver {
            Some(server) => query(name, *record, SocketAddr::new(*server, 53), timeout),
            None => resolve_with_the_system(name, *record),
        }
    }
}

/// What a response says about the name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    /// At least one record of the requested type came back.
    Resolved,
    /// The server answered that the name does not exist.
    NoSuchName,
    /// The name exists and carries no record of the requested type.
    NoRecord,
    /// The server answered that it could not answer.
    Refused(&'static str),
    /// The bytes cannot be used as an answer of this query.
    Unusable(&'static str),
}

/// A name resolution through the system resolver, for a target without a server.
///
/// A name that does not exist is the answer of the target; a resolver that cannot be asked
/// at all is a local failure.
fn resolve_with_the_system(name: &str, record: RecordType) -> Result<Verdict, ProbeError> {
    let started = Instant::now();
    let addresses = match (name, 0).to_socket_addrs() {
        Ok(addresses) => addresses.collect::<Vec<SocketAddr>>(),
        // `getaddrinfo` exposes no portable way to tell "this name does not exist" from
        // "the resolver could not be asked": both arrive as an unclassified error. A name
        // this host cannot resolve is therefore `down`; a target that needs the
        // distinction names its resolver and gets the RCODE-based answer.
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => {
            return Err(ProbeError::of(
                Code::DnsSocket,
                &Details {
                    server: Some(SYSTEM_RESOLVER),
                    name: Some(name),
                    reason: Some(&error.to_string()),
                    ..Details::default()
                },
            ))
        }
        Err(_) => return Ok(Verdict::Down(unresolved(name))),
    };

    let found = addresses.iter().any(|address| match record {
        RecordType::A => address.is_ipv4(),
        RecordType::Aaaa => address.is_ipv6(),
    });

    Ok(if found {
        Verdict::Up {
            rtt: started.elapsed(),
        }
    } else {
        // The name resolves, but not to the family the question asked for.
        Verdict::Down(Failure::new(
            Code::DnsNoRecord,
            &Details {
                name: Some(name),
                server: Some(SYSTEM_RESOLVER),
                ..Details::default()
            },
        ))
    })
}

/// One UDP question to one server.
fn query(
    name: &str,
    record: RecordType,
    server: SocketAddr,
    timeout: Duration,
) -> Result<Verdict, ProbeError> {
    let message = build_query(name, record, identifier())?;
    let bind: SocketAddr = if server.is_ipv4() {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0)
    } else {
        SocketAddr::new(IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0)
    };
    let reach = |error: &io::Error| {
        ProbeError::of(
            Code::DnsSocket,
            &Details {
                server: Some(&server.to_string()),
                name: Some(name),
                reason: Some(&error.to_string()),
                ..Details::default()
            },
        )
    };
    let socket = UdpSocket::bind(bind).map_err(|error| reach(&error))?;
    socket.connect(server).map_err(|error| reach(&error))?;
    socket
        .set_read_timeout(Some(timeout))
        .map_err(|error| reach(&error))?;

    let started = Instant::now();
    socket.send(&message).map_err(|error| reach(&error))?;

    let mut buffer = [0u8; MAX_RESPONSE];
    let size = match socket.recv(&mut buffer) {
        Ok(size) => size,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Ok(Verdict::Down(Failure::new(
                Code::DnsTimeout,
                &Details {
                    name: Some(name),
                    server: Some(&server.to_string()),
                    timeout: Some(timeout),
                    ..Details::default()
                },
            )))
        }
        Err(error) => {
            return Err(ProbeError::of(
                Code::DnsSocket,
                &Details {
                    server: Some(&server.to_string()),
                    name: Some(name),
                    reason: Some(&error.to_string()),
                    ..Details::default()
                },
            ))
        }
    };

    match parse_response(&buffer[..size], record) {
        Response::Resolved => Ok(Verdict::Up {
            rtt: started.elapsed(),
        }),
        Response::NoSuchName => Ok(Verdict::Down(answer(name, Code::DnsNxDomain, server))),
        Response::NoRecord => Ok(Verdict::Down(answer(name, Code::DnsNoRecord, server))),
        Response::Refused(reason) => Err(server_failure(name, server, reason)),
        Response::Unusable(reason) => Err(ProbeError::of(
            Code::DnsUnusable,
            &Details {
                server: Some(&server.to_string()),
                name: Some(name),
                reason: Some(reason),
                ..Details::default()
            },
        )),
    }
}

/// What a name that does not resolve through the system resolver is called.
const SYSTEM_RESOLVER: &str = "the system resolver";

/// A name the system resolver cannot resolve.
fn unresolved(name: &str) -> Failure {
    Failure::new(
        Code::DnsUnresolved,
        &Details {
            name: Some(name),
            ..Details::default()
        },
    )
}

/// The answer of one server about one name: the name does not exist, or carries no record
/// of the requested type.
fn answer(name: &str, code: Code, server: SocketAddr) -> Failure {
    let server = server.to_string();
    Failure::new(
        code,
        &Details {
            name: Some(name),
            server: Some(&server),
            ..Details::default()
        },
    )
}

/// A server that answered that it could not answer.
fn server_failure(name: &str, server: SocketAddr, reason: &str) -> ProbeError {
    ProbeError::of(
        Code::DnsServerFailure,
        &Details {
            server: Some(&server.to_string()),
            name: Some(name),
            reason: Some(reason),
            ..Details::default()
        },
    )
}

/// A query identifier that changes with every call.
fn identifier() -> u16 {
    use std::sync::atomic::{AtomicU16, Ordering};
    static NEXT: AtomicU16 = AtomicU16::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Encode one question: header, QNAME, QTYPE, QCLASS.
pub fn build_query(name: &str, record: RecordType, identifier: u16) -> Result<Vec<u8>, ProbeError> {
    let mut message = Vec::with_capacity(32);
    message.extend_from_slice(&identifier.to_be_bytes());
    message.extend_from_slice(&0x0100u16.to_be_bytes()); // recursion desired
    message.extend_from_slice(&1u16.to_be_bytes()); // one question
    message.extend_from_slice(&0u16.to_be_bytes()); // no answers
    message.extend_from_slice(&0u16.to_be_bytes()); // no authority records
    message.extend_from_slice(&0u16.to_be_bytes()); // no additional records

    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() || label.len() > 63 {
            return Err(ProbeError::of(
                Code::DnsBuild,
                &Details {
                    name: Some(name),
                    reason: Some("a label is empty or longer than 63 bytes"),
                    ..Details::default()
                },
            ));
        }
        message.push(label.len() as u8);
        message.extend_from_slice(label.as_bytes());
    }
    message.push(0);

    message.extend_from_slice(&record_type(record).to_be_bytes());
    message.extend_from_slice(&1u16.to_be_bytes()); // class IN
    Ok(message)
}

/// Decode the answer of a query, ignoring everything but its outcome.
pub fn parse_response(message: &[u8], record: RecordType) -> Response {
    if message.len() < 12 {
        return Response::Unusable("shorter than a DNS header");
    }
    let flags = u16::from_be_bytes([message[2], message[3]]);
    if flags & 0x8000 == 0 {
        return Response::Unusable("not a response");
    }
    if flags & 0x0200 != 0 {
        return Response::Unusable("truncated by the server");
    }

    let questions = u16::from_be_bytes([message[4], message[5]]) as usize;
    let answers = u16::from_be_bytes([message[6], message[7]]) as usize;

    let mut at = 12;
    for _ in 0..questions {
        at = match read_name(message, at) {
            Ok((_, next)) => next,
            Err(reason) => return Response::Unusable(reason),
        } + 4; // QTYPE and QCLASS
        if at > message.len() {
            return Response::Unusable("the question runs past the message");
        }
    }

    // A record of the wanted type anywhere in the answer section resolves the name; a CNAME
    // chain is followed by the server itself, which puts the final record here too.
    let wanted = record_type(record);
    for _ in 0..answers {
        match read_record(message, at) {
            Ok((record_type, next)) => {
                at = next;
                if record_type == wanted {
                    return Response::Resolved;
                }
            }
            Err(reason) => return Response::Unusable(reason),
        }
    }

    match flags & 0x000f {
        0 => Response::NoRecord,
        3 => Response::NoSuchName,
        2 => Response::Refused("the server failed (SERVFAIL)"),
        4 => Response::Refused("the server does not implement the query (NOTIMP)"),
        5 => Response::Refused("the server refused the query (REFUSED)"),
        _ => Response::Refused("an unexpected error code"),
    }
}

fn record_type(record: RecordType) -> u16 {
    match record {
        RecordType::A => 1,
        RecordType::Aaaa => 28,
    }
}

/// Read a name at `at`, following compression pointers, and return it with the offset just
/// past the name as it appears in the record.
fn read_name(message: &[u8], at: usize) -> Result<(String, usize), &'static str> {
    let mut labels: Vec<&[u8]> = Vec::new();
    let mut cursor = at;
    let mut end = None;
    let mut jumps = 0;

    loop {
        let length = *message.get(cursor).ok_or("a name runs past the message")? as usize;
        match length {
            0 => {
                let next = end.unwrap_or(cursor + 1);
                return Ok((
                    String::from_utf8_lossy(&labels.join(&b'.')).into_owned(),
                    next,
                ));
            }
            // Two high bits set: a pointer to an earlier name.
            0xc0..=0xff => {
                let second = *message
                    .get(cursor + 1)
                    .ok_or("a pointer runs past the message")?;
                let target = ((length & 0x3f) << 8) | second as usize;
                if target >= cursor {
                    return Err("a compression pointer does not point backwards");
                }
                jumps += 1;
                if jumps > MAX_POINTER_JUMPS {
                    return Err("a compression pointer chain is too long");
                }
                end.get_or_insert(cursor + 2);
                cursor = target;
            }
            length if length > 63 => return Err("a label is longer than 63 bytes"),
            length => {
                let start = cursor + 1;
                let label = message
                    .get(start..start + length)
                    .ok_or("a label runs past the message")?;
                labels.push(label);
                cursor = start + length;
            }
        }
    }
}

/// Read one resource record and return its type with the offset past it.
fn read_record(message: &[u8], at: usize) -> Result<(u16, usize), &'static str> {
    let (_, after_name) = read_name(message, at)?;
    let header = message
        .get(after_name..after_name + 10)
        .ok_or("a record header runs past the message")?;
    let record_type = u16::from_be_bytes([header[0], header[1]]);
    let length = u16::from_be_bytes([header[8], header[9]]) as usize;
    let next = after_name + 10 + length;
    if next > message.len() {
        return Err("a record runs past the message");
    }
    Ok((record_type, next))
}

impl fmt::Display for Response {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Response::Resolved => formatter.write_str("resolved"),
            Response::NoSuchName => formatter.write_str("no such name"),
            Response::NoRecord => formatter.write_str("no record of that type"),
            Response::Refused(reason) | Response::Unusable(reason) => formatter.write_str(reason),
        }
    }
}

/// The simulated day of one DNS target.
///
/// The name stops existing, stops having the record that was asked for, or the server stops
/// answering. A target without a resolver of its own is answered by the system resolver,
/// which is what the message says.
pub fn simulated_day(
    endpoint: &Endpoint,
    seed: u64,
    day_start_ms: i64,
    fault_rate: f64,
) -> Vec<SimulatedEpisode> {
    let Endpoint::Dns { name, resolver, .. } = endpoint else {
        // The dispatcher chooses by method; this line cannot be reached from it.
        return Vec::new();
    };
    let name = name.clone();
    let server = resolver
        .map(|resolver| resolver.to_string())
        .unwrap_or_else(|| "the system resolver".to_string());
    super::simulated_episodes(
        endpoint,
        seed,
        day_start_ms,
        fault_rate,
        "dns",
        |rng| rng.between(500, 50_000),
        |rng| {
            let stated = Details {
                name: Some(&name),
                server: Some(&server),
                ..Details::default()
            };
            match rng.below(10) {
                0..=5 => Failure::new(Code::DnsNxDomain, &stated),
                6..=8 => Failure::new(Code::DnsNoRecord, &stated),
                _ => Failure::new(
                    Code::DnsTimeout,
                    &Details {
                        timeout: Some(super::SIMULATED_TIMEOUT),
                        ..stated
                    },
                ),
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// A response to `query`, with the given rcode and answer records.
    /// The code of a `down` verdict, so an assertion states what the server answered.
    fn code_of(verdict: &Verdict) -> &'static str {
        match verdict {
            Verdict::Down(failure) => failure.code(),
            other => panic!("expected a down verdict, got {other:?}"),
        }
    }

    fn response(query: &[u8], rcode: u8, answers: &[Vec<u8>]) -> Vec<u8> {
        let mut message = Vec::new();
        message.extend_from_slice(&query[0..2]);
        let flags = 0x8180u16 | u16::from(rcode & 0x0f);
        message.extend_from_slice(&flags.to_be_bytes());
        message.extend_from_slice(&1u16.to_be_bytes());
        message.extend_from_slice(&(answers.len() as u16).to_be_bytes());
        message.extend_from_slice(&0u16.to_be_bytes());
        message.extend_from_slice(&0u16.to_be_bytes());
        message.extend_from_slice(&query[12..]);
        for answer in answers {
            message.extend_from_slice(answer);
        }
        message
    }

    fn record(record_type: u16, data: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xc0, 0x0c]; // the name of the question, compressed
        bytes.extend_from_slice(&record_type.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&60u32.to_be_bytes());
        bytes.extend_from_slice(&(data.len() as u16).to_be_bytes());
        bytes.extend_from_slice(data);
        bytes
    }

    fn a_record() -> Vec<u8> {
        record(1, &[93, 184, 216, 34])
    }

    fn aaaa_record() -> Vec<u8> {
        let address: Ipv6Addr = "2001:db8::1".parse().unwrap();
        record(28, &address.octets())
    }

    fn cname_record() -> Vec<u8> {
        let mut data = Vec::new();
        for label in ["example", "net"] {
            data.push(label.len() as u8);
            data.extend_from_slice(label.as_bytes());
        }
        data.push(0);
        record(5, &data)
    }

    fn query_bytes(name: &str, record: RecordType) -> Vec<u8> {
        build_query(name, record, 0x1234).unwrap()
    }

    #[test]
    fn builds_a_question_on_the_wire() {
        let message = query_bytes("example.com", RecordType::A);

        assert_eq!(&message[0..2], &[0x12, 0x34]);
        assert_eq!(u16::from_be_bytes([message[2], message[3]]), 0x0100);
        assert_eq!(u16::from_be_bytes([message[4], message[5]]), 1);
        assert_eq!(&message[12..], b"\x07example\x03com\x00\x00\x01\x00\x01");
    }

    #[test]
    fn a_name_that_cannot_be_asked_states_its_code() {
        for name in ["", "a.b..c", &"x".repeat(64)] {
            let error = build_query(name, RecordType::A, 1)
                .expect_err("a name that cannot be encoded is our failure");
            assert_eq!(error.code(), "DNS.BUILD", "for {name:?}");
        }
    }

    #[test]
    fn an_answer_of_the_requested_type_resolves() {
        let query = query_bytes("example.com", RecordType::A);
        let answer = response(&query, 0, &[a_record()]);
        assert_eq!(parse_response(&answer, RecordType::A), Response::Resolved);
    }

    #[test]
    fn an_aaaa_answer_resolves_an_aaaa_question() {
        let query = query_bytes("example.com", RecordType::Aaaa);
        let answer = response(&query, 0, &[aaaa_record()]);
        assert_eq!(
            parse_response(&answer, RecordType::Aaaa),
            Response::Resolved
        );
    }

    #[test]
    fn a_record_of_the_wrong_family_does_not_resolve() {
        let query = query_bytes("example.com", RecordType::A);
        let answer = response(&query, 0, &[aaaa_record()]);
        assert_eq!(parse_response(&answer, RecordType::A), Response::NoRecord);
    }

    #[test]
    fn a_cname_chain_resolves_when_the_final_record_is_present() {
        let query = query_bytes("www.example.com", RecordType::A);
        let answer = response(&query, 0, &[cname_record(), a_record()]);
        assert_eq!(parse_response(&answer, RecordType::A), Response::Resolved);
    }

    #[test]
    fn a_missing_name_is_unresolved() {
        let query = query_bytes("missing.example.com", RecordType::A);
        let answer = response(&query, 3, &[]);
        assert_eq!(parse_response(&answer, RecordType::A), Response::NoSuchName);
    }

    #[test]
    fn a_server_failure_is_not_a_verdict() {
        let query = query_bytes("example.com", RecordType::A);
        for rcode in [2u8, 4, 5] {
            let answer = response(&query, rcode, &[]);
            assert!(
                matches!(parse_response(&answer, RecordType::A), Response::Refused(_)),
                "rcode {rcode} must be a server failure"
            );
        }
    }

    #[test]
    fn a_truncated_answer_is_unusable() {
        let query = query_bytes("example.com", RecordType::A);
        let mut answer = response(&query, 0, &[a_record()]);
        answer[2] |= 0x02; // TC
        assert!(matches!(
            parse_response(&answer, RecordType::A),
            Response::Unusable(_)
        ));
    }

    #[test]
    fn a_message_that_is_not_a_response_is_unusable() {
        let query = query_bytes("example.com", RecordType::A);
        assert!(matches!(
            parse_response(&query, RecordType::A),
            Response::Unusable(_)
        ));
        assert!(matches!(
            parse_response(&[0u8; 4], RecordType::A),
            Response::Unusable(_)
        ));
    }

    #[test]
    fn a_pointer_that_does_not_point_backwards_is_refused() {
        let query = query_bytes("example.com", RecordType::A);
        let mut broken = vec![0xc0, 0x0c];
        broken.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
        let answer = response(&query, 0, &[broken]);
        // The pointer targets the question, which is legal; a forward one is not.
        let mut forward = response(&query, 0, &[]);
        let at = forward.len();
        forward.push(0xc0);
        forward.push((at + 2) as u8);
        forward.extend_from_slice(&[0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 1, 2, 3, 4]);
        forward[7] = 1; // one answer
        assert!(matches!(
            parse_response(&forward, RecordType::A),
            Response::Unusable(_)
        ));
        // The legal backward pointer is read without complaint.
        assert_eq!(parse_response(&answer, RecordType::A), Response::Resolved);
    }

    fn local_server<F>(handler: F) -> (SocketAddr, thread::JoinHandle<()>)
    where
        F: FnOnce(&UdpSocket, &[u8], SocketAddr) + Send + 'static,
    {
        let socket = UdpSocket::bind("127.0.0.1:0").expect("a loopback socket");
        let address = socket.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let mut buffer = [0u8; 512];
            let (size, from) = socket.recv_from(&mut buffer).expect("a query arrives");
            handler(&socket, &buffer[..size], from);
        });
        (address, handle)
    }

    #[test]
    fn a_server_answering_resolves_the_name() {
        let (address, server) = local_server(|socket, query, from| {
            let answer = response(query, 0, &[a_record()]);
            socket.send_to(&answer, from).unwrap();
        });

        let verdict = query(
            "example.com",
            RecordType::A,
            address,
            Duration::from_millis(500),
        )
        .expect("a local exchange");

        assert!(matches!(verdict, Verdict::Up { .. }));
        let _ = server.join();
    }

    #[test]
    fn a_server_answering_nxdomain_is_down() {
        let (address, server) = local_server(|socket, query, from| {
            let answer = response(query, 3, &[]);
            socket.send_to(&answer, from).unwrap();
        });

        let verdict = query(
            "missing.example.com",
            RecordType::A,
            address,
            Duration::from_millis(500),
        )
        .expect("a local exchange");

        assert_eq!(code_of(&verdict), "DNS.NXDOMAIN");
        let _ = server.join();
    }

    /// The code of a local failure, so an assertion states what the server or this host did.
    fn code_of_error(error: &ProbeError) -> &'static str {
        error.code()
    }

    #[test]
    fn a_record_of_another_type_answers_no_record() {
        let (address, server) = local_server(|socket, query, from| {
            let answer = response(query, 0, &[aaaa_record()]);
            socket.send_to(&answer, from).unwrap();
        });

        let verdict = query(
            "example.com",
            RecordType::A,
            address,
            Duration::from_millis(500),
        )
        .expect("a local exchange");

        assert_eq!(code_of(&verdict), "DNS.NO_RECORD");
        let _ = server.join();
    }

    #[test]
    fn a_server_that_fails_states_its_code() {
        for rcode in [2u8, 4, 5] {
            let (address, server) = local_server(move |socket, query, from| {
                let answer = response(query, rcode, &[]);
                socket.send_to(&answer, from).unwrap();
            });

            let error = query(
                "example.com",
                RecordType::A,
                address,
                Duration::from_millis(500),
            )
            .expect_err("a server that cannot answer is a local failure");

            assert_eq!(code_of_error(&error), "DNS.SERVER_FAILURE", "rcode {rcode}");
            assert!(
                error.message().contains("could not answer"),
                "unexpected: {error}"
            );
            let _ = server.join();
        }
    }

    #[test]
    fn a_truncated_answer_over_the_wire_states_its_code() {
        let (address, server) = local_server(|socket, query, from| {
            let mut answer = response(query, 0, &[a_record()]);
            answer[2] |= 0x02; // TC
            socket.send_to(&answer, from).unwrap();
        });

        let error = query(
            "example.com",
            RecordType::A,
            address,
            Duration::from_millis(500),
        )
        .expect_err("a truncated answer is not an answer");

        assert_eq!(code_of_error(&error), "DNS.UNUSABLE");
        let _ = server.join();
    }

    #[test]
    fn a_socket_that_cannot_send_states_its_code() {
        // Sending to the broadcast address is refused without `SO_BROADCAST`: the cheapest
        // way to reach the socket failures of this transport without a network.
        let server = SocketAddr::from((Ipv4Addr::BROADCAST, 53));

        let error = query(
            "example.com",
            RecordType::A,
            server,
            Duration::from_millis(100),
        )
        .expect_err("a socket that cannot send is our failure");

        assert_eq!(code_of_error(&error), "DNS.SOCKET");
    }

    #[test]
    fn a_server_that_stays_silent_is_down() {
        let (address, server) = local_server(|_, _, _| {});

        let verdict = query(
            "example.com",
            RecordType::A,
            address,
            Duration::from_millis(50),
        )
        .expect("a timeout is a verdict");

        assert_eq!(code_of(&verdict), "DNS.TIMEOUT");
        let _ = server.join();
    }

    #[test]
    fn the_system_resolver_resolves_localhost() {
        let verdict = DnsProbe::new()
            .probe(
                &Endpoint::Dns {
                    name: "localhost".to_string(),
                    resolver: None,
                    record: RecordType::A,
                },
                None,
                Duration::from_millis(500),
            )
            .expect("the system resolver can be asked");

        assert!(matches!(verdict, Verdict::Up { .. }), "got {verdict:?}");
    }

    #[test]
    fn a_name_the_system_cannot_resolve_is_down() {
        let verdict = DnsProbe::new()
            .probe(
                &Endpoint::Dns {
                    name: "network-monitor.invalid".to_string(),
                    resolver: None,
                    record: RecordType::A,
                },
                None,
                Duration::from_millis(500),
            )
            .expect("a name that does not exist is a verdict");

        assert_eq!(code_of(&verdict), "DNS.UNRESOLVED");
    }

    #[test]
    fn another_method_is_refused() {
        let endpoint = Endpoint::Icmp {
            address: Ipv4Addr::LOCALHOST,
        };
        assert!(DnsProbe::new()
            .probe(&endpoint, None, Duration::from_millis(100))
            .is_err());
    }
}
