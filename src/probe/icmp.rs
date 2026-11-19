//! Raw ICMP echo probing.
//!
//! One call opens a raw socket, optionally binds it to an interface, sends one echo
//! request, waits for the matching reply up to the configured timeout, and closes the
//! socket. That is the semantics of `ping -I <interface> <address>` without parsing a
//! localised `ping` output (ADR-0003).

use std::io;
use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::time::{Duration, Instant};

use super::{Code, Details, Endpoint, Failure, Probe, ProbeError, SimulatedEpisode, Verdict};

/// Identifier carried in every echo request of this process.
fn identifier() -> u16 {
    (std::process::id() & 0xffff) as u16
}

/// Raw ICMP echo prober.
///
/// Stateless apart from the sequence number: the timeout arrives with every request, so
/// two targets of the same process may use different ones.
#[derive(Default)]
pub struct IcmpProbe {
    sequence: AtomicU16,
}

impl IcmpProbe {
    pub fn new() -> Self {
        IcmpProbe {
            sequence: AtomicU16::new(1),
        }
    }
}

impl Probe for IcmpProbe {
    fn probe(
        &self,
        endpoint: &Endpoint,
        interface: Option<&str>,
        timeout: Duration,
    ) -> Result<Verdict, ProbeError> {
        let Endpoint::Icmp { address } = endpoint else {
            return Err(ProbeError::of(
                Code::ProbeWrongMethod,
                &Details {
                    method: Some("icmp"),
                    other: Some(endpoint.method().as_str()),
                    ..Details::default()
                },
            ));
        };

        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        exchange(*address, interface, timeout, identifier(), sequence)
    }
}

/// Build an ICMP echo request: type 8, code 0, identifier, sequence, 24 bytes payload.
pub(crate) fn build_echo_request(identifier: u16, sequence: u16) -> Vec<u8> {
    let mut packet = vec![0u8; 8 + 24];
    packet[0] = 8; // echo request
    packet[1] = 0; // code
    packet[4..6].copy_from_slice(&identifier.to_be_bytes());
    packet[6..8].copy_from_slice(&sequence.to_be_bytes());
    packet[8] = b'N';
    packet[9] = b'M';

    let sum = checksum(&packet);
    packet[2..4].copy_from_slice(&sum.to_be_bytes());
    packet
}

/// RFC 1071 one's-complement checksum.
pub(crate) fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut chunks = data.chunks_exact(2);
    for chunk in &mut chunks {
        sum += u16::from_be_bytes([chunk[0], chunk[1]]) as u32;
    }
    if let Some(last) = chunks.remainder().first() {
        sum += (*last as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

/// Skip the IPv4 header when the platform delivers it with the ICMP message.
pub(crate) fn icmp_message(buffer: &[u8]) -> Option<&[u8]> {
    if buffer.len() >= 20 && buffer[0] >> 4 == 4 {
        let header_length = (buffer[0] & 0x0f) as usize * 4;
        if header_length < 20 || buffer.len() < header_length {
            return None;
        }
        Some(&buffer[header_length..])
    } else {
        Some(buffer)
    }
}

/// True when the message is an echo reply carrying our identifier and sequence.
pub(crate) fn is_expected_reply(message: &[u8], identifier: u16, sequence: u16) -> bool {
    if message.len() < 8 || message[0] != 0 || message[1] != 0 {
        return false;
    }
    let reply_identifier = u16::from_be_bytes([message[4], message[5]]);
    let reply_sequence = u16::from_be_bytes([message[6], message[7]]);
    reply_identifier == identifier && reply_sequence == sequence
}

/// Wait for the reply to the request just sent, with one budget for the whole probe.
///
/// A raw ICMP socket receives **every** echo reply of the host, not only the one this
/// probe waits for: the other targets of this process answer into the same socket family,
/// and any other ping on the machine lands here too. `receive` is therefore called once
/// per packet and answers `None` for a packet that is not a candidate and for a wait that
/// expired. Each wait is shortened to what is left of `timeout`, so foreign traffic can
/// delay the verdict by the gap between two packets and can never postpone it: a probe
/// against a dead target still concludes in `timeout`, however busy the host is.
/// The time the reply took, or `None` when the budget expired without one.
fn await_reply<F>(
    identifier: u16,
    sequence: u16,
    timeout: Duration,
    mut receive: F,
) -> Result<Option<Duration>, ProbeError>
where
    F: FnMut(Duration, &mut [u8]) -> Result<Option<usize>, ProbeError>,
{
    let started = Instant::now();
    let mut buffer = [0u8; 1500];

    loop {
        let wait = match timeout.checked_sub(started.elapsed()) {
            Some(wait) if !wait.is_zero() => wait,
            _ => return Ok(None),
        };

        let Some(received) = receive(wait, &mut buffer)? else {
            continue;
        };

        if let Some(message) = icmp_message(&buffer[..received]) {
            if is_expected_reply(message, identifier, sequence) {
                return Ok(Some(started.elapsed()));
            }
        }
    }
}

/// The verdict of one exchange: a reply is `up`, an expired budget is `down` without one.
fn conclusion(target: Ipv4Addr, timeout: Duration, reply: Option<Duration>) -> Verdict {
    match reply {
        Some(rtt) => Verdict::Up { rtt },
        None => {
            let address = target.to_string();
            Verdict::Down(Failure::new(
                Code::IcmpNoReply,
                &Details {
                    address: Some(&address),
                    timeout: Some(timeout),
                    ..Details::default()
                },
            ))
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::io;
    use std::mem;
    use std::time::Duration;

    use super::super::{ProbeError, Verdict};
    use super::socket_failure;
    use super::{await_reply, build_echo_request, conclusion, interface_failure, io_failure};

    /// Closes the socket on every return path.
    struct Socket(libc::c_int);

    impl Drop for Socket {
        fn drop(&mut self) {
            unsafe {
                libc::close(self.0);
            }
        }
    }

    pub fn exchange(
        target: std::net::Ipv4Addr,
        interface: Option<&str>,
        timeout: Duration,
        identifier: u16,
        sequence: u16,
    ) -> Result<Verdict, ProbeError> {
        let raw = unsafe { libc::socket(libc::AF_INET, libc::SOCK_RAW, libc::IPPROTO_ICMP) };
        if raw < 0 {
            return Err(socket_failure(&io::Error::last_os_error()));
        }
        let socket = Socket(raw);

        if let Some(name) = interface {
            crate::probe::interface::bind(socket.0, name)
                .map_err(|reason| interface_failure(name, &reason))?;
        }

        let packet = build_echo_request(identifier, sequence);
        let destination = socket_address(target);
        let sent = unsafe {
            libc::sendto(
                socket.0,
                packet.as_ptr() as *const libc::c_void,
                packet.len(),
                0,
                &destination as *const libc::sockaddr_in as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            )
        };
        if sent < 0 {
            return Err(io_failure(&io::Error::last_os_error()));
        }

        await_reply(identifier, sequence, timeout, |wait, buffer| {
            receive_one(socket.0, target, wait, buffer)
        })
        .map(|reply| conclusion(target, timeout, reply))
    }

    /// One wait for one packet: `None` when it was not ours or the wait expired.
    fn receive_one(
        socket: libc::c_int,
        target: std::net::Ipv4Addr,
        wait: std::time::Duration,
        buffer: &mut [u8],
    ) -> Result<Option<usize>, ProbeError> {
        set_receive_timeout(socket, wait)?;

        let mut from: libc::sockaddr_storage = unsafe { mem::zeroed() };
        let mut from_length = mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
        let received = unsafe {
            libc::recvfrom(
                socket,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
                0,
                &mut from as *mut libc::sockaddr_storage as *mut libc::sockaddr,
                &mut from_length,
            )
        };

        if received < 0 {
            let error = io::Error::last_os_error();
            return match error.kind() {
                io::ErrorKind::WouldBlock
                | io::ErrorKind::TimedOut
                | io::ErrorKind::Interrupted => Ok(None),
                _ => Err(io_failure(&error)),
            };
        }

        let received = received as usize;
        if !source_matches(&from, target) {
            return Ok(None);
        }
        Ok(Some(received))
    }

    fn set_receive_timeout(fd: libc::c_int, timeout: Duration) -> Result<(), ProbeError> {
        // A zeroed `timeval` means "wait forever" on POSIX, so a remainder under a
        // microsecond is rounded up instead of disabling the timeout.
        let timeout = timeout.max(Duration::from_millis(1));
        let value = libc::timeval {
            tv_sec: timeout.as_secs() as _,
            tv_usec: timeout.subsec_micros() as _,
        };
        let result = unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &value as *const libc::timeval as *const libc::c_void,
                mem::size_of::<libc::timeval>() as libc::socklen_t,
            )
        };
        if result != 0 {
            return Err(io_failure(&io::Error::last_os_error()));
        }
        Ok(())
    }

    fn socket_address(target: std::net::Ipv4Addr) -> libc::sockaddr_in {
        let mut address: libc::sockaddr_in = unsafe { mem::zeroed() };
        address.sin_family = libc::AF_INET as _;
        address.sin_addr.s_addr = u32::from_ne_bytes(target.octets());
        address
    }

    fn source_matches(from: &libc::sockaddr_storage, target: std::net::Ipv4Addr) -> bool {
        // SAFETY: recvfrom filled a storage large enough for a sockaddr_in, and the
        // socket is AF_INET, so reinterpreting it as sockaddr_in is valid.
        let address =
            unsafe { &*(from as *const libc::sockaddr_storage as *const libc::sockaddr_in) };
        address.sin_addr.s_addr == u32::from_ne_bytes(target.octets())
    }
}

#[cfg(windows)]
mod platform {
    use std::net::Ipv4Addr;
    use std::sync::Once;
    use std::time::Duration;

    use super::super::{ProbeError, Verdict};
    use super::socket_failure;
    use super::{await_reply, build_echo_request, conclusion, interface_failure, io_failure};

    const AF_INET: i32 = 2;
    const SOCK_RAW: i32 = 3;
    const IPPROTO_ICMP: i32 = 1;
    const SOL_SOCKET: i32 = 0xffff;
    const SO_RCVTIMEO: i32 = 0x1006;
    const INVALID_SOCKET: usize = usize::MAX;
    const WSAETIMEDOUT: i32 = 10060;
    const WSAEWOULDBLOCK: i32 = 10035;

    #[link(name = "ws2_32")]
    extern "system" {
        fn WSAStartup(version: u16, data: *mut u8) -> i32;
        fn socket(af: i32, kind: i32, protocol: i32) -> usize;
        fn setsockopt(socket: usize, level: i32, name: i32, value: *const u8, length: i32) -> i32;
        fn sendto(
            socket: usize,
            buffer: *const u8,
            length: i32,
            flags: i32,
            to: *const u8,
            to_length: i32,
        ) -> i32;
        fn recvfrom(
            socket: usize,
            buffer: *mut u8,
            length: i32,
            flags: i32,
            from: *mut u8,
            from_length: *mut i32,
        ) -> i32;
        fn closesocket(socket: usize) -> i32;
        fn WSAGetLastError() -> i32;
    }

    struct Socket(usize);

    impl Drop for Socket {
        fn drop(&mut self) {
            unsafe {
                closesocket(self.0);
            }
        }
    }

    /// The error the Windows socket layer reported.
    fn windows_error(code: i32) -> std::io::Error {
        std::io::Error::from_raw_os_error(code)
    }

    fn start_winsock() {
        static START: Once = Once::new();
        START.call_once(|| unsafe {
            let mut data = [0u8; 512];
            WSAStartup(0x0202, data.as_mut_ptr());
        });
    }

    pub fn exchange(
        target: Ipv4Addr,
        interface: Option<&str>,
        timeout: Duration,
        identifier: u16,
        sequence: u16,
    ) -> Result<Verdict, ProbeError> {
        start_winsock();

        let raw = unsafe { socket(AF_INET, SOCK_RAW, IPPROTO_ICMP) };
        if raw == INVALID_SOCKET {
            return Err(socket_failure(&std::io::Error::other(
                "run the tool from an elevated shell",
            )));
        }
        let socket = Socket(raw);

        if let Some(name) = interface {
            crate::probe::interface::bind(socket.0, name)
                .map_err(|reason| interface_failure(name, &reason))?;
        }

        let packet = build_echo_request(identifier, sequence);
        let destination = socket_address(target);
        let sent = unsafe {
            sendto(
                socket.0,
                packet.as_ptr(),
                packet.len() as i32,
                0,
                destination.as_ptr(),
                destination.len() as i32,
            )
        };
        if sent < 0 {
            let code = unsafe { WSAGetLastError() };
            return Err(io_failure(&windows_error(code)));
        }

        await_reply(identifier, sequence, timeout, |wait, buffer| {
            receive_one(socket.0, target, wait, buffer)
        })
        .map(|reply| conclusion(target, timeout, reply))
    }

    /// One wait for one packet: `None` when it was not ours or the wait expired.
    fn receive_one(
        socket: usize,
        target: Ipv4Addr,
        wait: Duration,
        buffer: &mut [u8],
    ) -> Result<Option<usize>, ProbeError> {
        let milliseconds = wait.as_millis().clamp(1, u32::MAX as u128) as u32;
        let result = unsafe {
            setsockopt(
                socket,
                SOL_SOCKET,
                SO_RCVTIMEO,
                &milliseconds as *const u32 as *const u8,
                4,
            )
        };
        if result != 0 {
            return Err(io_failure(&std::io::Error::other(
                "cannot set the receive timeout",
            )));
        }

        let mut from = [0u8; 16];
        let mut from_length: i32 = from.len() as i32;
        let received = unsafe {
            recvfrom(
                socket,
                buffer.as_mut_ptr(),
                buffer.len() as i32,
                0,
                from.as_mut_ptr(),
                &mut from_length,
            )
        };

        if received < 0 {
            let code = unsafe { WSAGetLastError() };
            return if code == WSAETIMEDOUT || code == WSAEWOULDBLOCK {
                Ok(None)
            } else {
                Err(io_failure(&windows_error(code)))
            };
        }

        let received = received as usize;
        if from[4..8] != target.octets() {
            return Ok(None);
        }
        Ok(Some(received))
    }

    /// `sockaddr_in` as raw bytes: family, port, address, padding.
    fn socket_address(target: Ipv4Addr) -> [u8; 16] {
        let mut address = [0u8; 16];
        address[0] = AF_INET as u8;
        address[4..8].copy_from_slice(&target.octets());
        address
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use super::super::{ProbeError, Verdict};
    use super::socket_failure;

    pub fn exchange(
        _target: Ipv4Addr,
        _interface: Option<&str>,
        _timeout: Duration,
        _identifier: u16,
        _sequence: u16,
    ) -> Result<Verdict, ProbeError> {
        Err(socket_failure(&std::io::Error::other(
            "raw ICMP probing is not implemented for this operating system",
        )))
    }
}

/// A raw socket that could not be opened; the privilege is the usual reason.
fn socket_failure(error: &io::Error) -> ProbeError {
    ProbeError::of(
        Code::IcmpSocket,
        &Details {
            // The template already states the privilege: repeating it here would say it twice.
            reason: Some(&error.to_string()),
            ..Details::default()
        },
    )
}

/// Sending or receiving on the socket failed for a reason that is not the target's silence.
fn io_failure(error: &io::Error) -> ProbeError {
    ProbeError::of(
        Code::IcmpIo,
        &Details {
            reason: Some(&error.to_string()),
            ..Details::default()
        },
    )
}

/// The socket exists but cannot be bound to the configured device.
fn interface_failure(interface: &str, reason: &str) -> ProbeError {
    ProbeError::of(
        Code::IcmpInterface,
        &Details {
            interface: Some(interface),
            reason: Some(reason),
            ..Details::default()
        },
    )
}

fn exchange(
    target: Ipv4Addr,
    interface: Option<&str>,
    timeout: Duration,
    identifier: u16,
    sequence: u16,
) -> Result<Verdict, ProbeError> {
    platform::exchange(target, interface, timeout, identifier, sequence)
}

/// The simulated day of one ICMP target.
///
/// An echo request that gets no reply is the one thing an ICMP probe can report, so every
/// fault of a simulated ICMP target states it. The latency of an answer spans a local link
/// and a distant one.
pub fn simulated_day(
    endpoint: &Endpoint,
    seed: u64,
    day_start_ms: i64,
    fault_rate: f64,
    rules: super::SimulatedRules,
) -> Vec<SimulatedEpisode> {
    let Endpoint::Icmp { address } = endpoint else {
        // The dispatcher chooses by method; this line cannot be reached from it.
        return Vec::new();
    };
    let address = address.to_string();
    super::simulated_episodes(
        endpoint,
        super::SimulatedSchedule {
            seed,
            day_start_ms,
            fault_rate,
            rules,
            purpose: "icmp",
        },
        |rng| rng.between(300, 40_000),
        |_rng| {
            Failure::new(
                Code::IcmpNoReply,
                &Details {
                    address: Some(&address),
                    timeout: Some(super::SIMULATED_TIMEOUT),
                    ..Details::default()
                },
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_socket_helpers_state_their_code() {
        // The privileged probe cannot run without root, so the mapping of the two helpers it
        // reports through is pinned here.
        let denied = std::io::Error::from_raw_os_error(0o1);
        assert_eq!(socket_failure(&denied).code(), "ICMP.SOCKET");
        assert_eq!(
            socket_failure(&denied).message(),
            "cannot open a raw ICMP socket: Operation not permitted (os error 1) \
             (root or CAP_NET_RAW is required)"
        );
        assert_eq!(io_failure(&denied).code(), "ICMP.IO");
        let bind = interface_failure("eth9", "no such device");
        assert_eq!(bind.code(), "ICMP.INTERFACE");
        assert_eq!(
            bind.message(),
            "cannot bind the probe to eth9: no such device"
        );
    }

    #[test]
    fn checksum_of_an_empty_buffer_is_all_ones() {
        assert_eq!(checksum(&[]), 0xffff);
    }

    #[test]
    fn echo_request_is_well_formed() {
        let packet = build_echo_request(0x1234, 7);
        assert_eq!(packet.len(), 32);
        assert_eq!(packet[0], 8);
        assert_eq!(packet[1], 0);
        assert_eq!(u16::from_be_bytes([packet[4], packet[5]]), 0x1234);
        assert_eq!(u16::from_be_bytes([packet[6], packet[7]]), 7);
        assert_eq!(checksum(&packet), 0);
    }

    #[test]
    fn strips_the_ipv4_header_when_present() {
        let mut packet = build_echo_request(1, 1);
        let mut datagram = vec![0u8; 20];
        datagram[0] = 0x45;
        datagram[9] = 1;
        datagram.append(&mut packet);
        let message = icmp_message(&datagram).unwrap();
        assert_eq!(message[0], 8);
    }

    #[test]
    fn keeps_the_message_when_no_ipv4_header_is_present() {
        let packet = build_echo_request(1, 1);
        let message = icmp_message(&packet).unwrap();
        assert_eq!(message.len(), packet.len());
    }

    #[test]
    fn matches_only_our_reply() {
        let mut reply = vec![0u8; 8];
        reply[0] = 0;
        reply[4..6].copy_from_slice(&42u16.to_be_bytes());
        reply[6..8].copy_from_slice(&9u16.to_be_bytes());
        assert!(is_expected_reply(&reply, 42, 9));
        assert!(!is_expected_reply(&reply, 42, 10));
        assert!(!is_expected_reply(&reply, 43, 9));
    }

    /// An echo reply for the given identifier and sequence, as the socket would deliver it.
    fn reply(identifier: u16, sequence: u16) -> Vec<u8> {
        let mut packet = build_echo_request(identifier, sequence);
        packet[0] = 0;
        packet
    }

    #[test]
    fn traffic_from_other_hosts_cannot_postpone_the_verdict() {
        // The bug this guards: a raw socket receives the replies of every target of this
        // process, and a `continue` that skipped the deadline let a dead target stay
        // "pending" for as long as foreign packets kept arriving.
        let timeout = Duration::from_millis(40);
        let mut waits: Vec<Duration> = Vec::new();
        let started = Instant::now();

        let verdict = await_reply(42, 9, timeout, |wait, _buffer| {
            waits.push(wait);
            Ok(None)
        });

        assert_eq!(verdict, Ok(None), "no reply before the budget expires");
        assert!(
            started.elapsed() < timeout + Duration::from_millis(500),
            "the probe concludes on its own budget, not on the traffic around it"
        );
        assert!(waits.len() > 1, "the wait is attempted again");
        assert!(
            waits.last() < waits.first(),
            "each wait shrinks with the budget instead of restarting it: {waits:?}"
        );
    }

    #[test]
    fn the_reply_to_our_request_ends_the_wait_at_once() {
        let verdict = await_reply(42, 9, Duration::from_secs(5), |_wait, buffer| {
            let packet = reply(42, 9);
            buffer[..packet.len()].copy_from_slice(&packet);
            Ok(Some(packet.len()))
        });
        assert!(
            matches!(verdict, Ok(Some(_))),
            "the reply is taken on the first packet"
        );
    }

    #[test]
    fn a_reply_to_another_request_is_not_ours() {
        let timeout = Duration::from_millis(30);
        let verdict = await_reply(42, 9, timeout, |_wait, buffer| {
            let packet = reply(42, 10);
            buffer[..packet.len()].copy_from_slice(&packet);
            Ok(Some(packet.len()))
        });
        assert_eq!(
            verdict,
            Ok(None),
            "another sequence is ignored and the budget still ends the probe"
        );
    }

    #[test]
    fn a_socket_failure_is_reported_instead_of_looping() {
        let verdict = await_reply(42, 9, Duration::from_secs(1), |_wait, _buffer| {
            Err(io_failure(&io::Error::other("the socket went away")))
        });
        let error = verdict.expect_err("an error ends the probe, it does not retry");
        assert_eq!(error.code(), "ICMP.IO");
    }

    #[test]
    fn ignores_echo_requests_and_errors() {
        let request = build_echo_request(42, 9);
        assert!(!is_expected_reply(&request, 42, 9));

        let mut unreachable = vec![0u8; 8];
        unreachable[0] = 3;
        assert!(!is_expected_reply(&unreachable, 42, 9));
    }
}
