//! Raw ICMP echo probing.
//!
//! One call opens a raw socket, optionally binds it to an interface, sends one echo
//! request, waits for the matching reply up to the configured timeout, and closes the
//! socket. That is the semantics of `ping -I <interface> <address>` without parsing a
//! localised `ping` output (ADR-0003).

use std::net::Ipv4Addr;
use std::sync::atomic::{AtomicU16, AtomicU64, Ordering};
use std::time::Duration;

use super::{Probe, ProbeError, Verdict};

/// Identifier carried in every echo request of this process.
fn identifier() -> u16 {
    (std::process::id() & 0xffff) as u16
}

/// Raw ICMP echo prober.
pub struct IcmpProbe {
    timeout_micros: AtomicU64,
    sequence: AtomicU16,
}

impl IcmpProbe {
    pub fn new(timeout: Duration) -> Self {
        IcmpProbe {
            timeout_micros: AtomicU64::new(timeout.as_micros() as u64),
            sequence: AtomicU16::new(1),
        }
    }

    fn timeout(&self) -> Duration {
        Duration::from_micros(self.timeout_micros.load(Ordering::Relaxed))
    }
}

impl Probe for IcmpProbe {
    fn probe(&self, target: Ipv4Addr, interface: Option<&str>) -> Result<Verdict, ProbeError> {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        exchange(target, interface, self.timeout(), identifier(), sequence)
    }

    fn set_timeout(&self, timeout: Duration) {
        self.timeout_micros
            .store(timeout.as_micros() as u64, Ordering::Relaxed);
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

#[cfg(unix)]
mod platform {
    use std::io;
    use std::mem;
    use std::time::{Duration, Instant};

    use super::super::{ProbeError, Verdict};
    use super::{build_echo_request, icmp_message, is_expected_reply};

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
            return Err(ProbeError::local(format!(
                "cannot open a raw ICMP socket: {} (root or CAP_NET_RAW is required)",
                io::Error::last_os_error()
            )));
        }
        let socket = Socket(raw);

        set_receive_timeout(socket.0, timeout)?;

        if let Some(name) = interface {
            crate::probe::interface::bind(socket.0, name).map_err(ProbeError::local)?;
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
            return Err(ProbeError::local(format!(
                "cannot send the echo request to {target}: {}",
                io::Error::last_os_error()
            )));
        }

        let started = Instant::now();
        let mut buffer = [0u8; 1500];
        loop {
            let mut from: libc::sockaddr_storage = unsafe { mem::zeroed() };
            let mut from_length = mem::size_of::<libc::sockaddr_storage>() as libc::socklen_t;
            let received = unsafe {
                libc::recvfrom(
                    socket.0,
                    buffer.as_mut_ptr() as *mut libc::c_void,
                    buffer.len(),
                    0,
                    &mut from as *mut libc::sockaddr_storage as *mut libc::sockaddr,
                    &mut from_length,
                )
            };

            if received < 0 {
                let error = io::Error::last_os_error();
                match error.kind() {
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut => {
                        return Ok(Verdict::Down)
                    }
                    io::ErrorKind::Interrupted => {
                        if started.elapsed() >= timeout {
                            return Ok(Verdict::Down);
                        }
                        continue;
                    }
                    _ => {
                        return Err(ProbeError::local(format!(
                            "cannot receive the echo reply: {error}"
                        )))
                    }
                }
            }

            let received = received as usize;
            if !source_matches(&from, target) {
                continue;
            }
            if let Some(message) = icmp_message(&buffer[..received]) {
                if is_expected_reply(message, identifier, sequence) {
                    return Ok(Verdict::Up {
                        rtt: started.elapsed(),
                    });
                }
            }

            if started.elapsed() >= timeout {
                return Ok(Verdict::Down);
            }
        }
    }

    fn set_receive_timeout(fd: libc::c_int, timeout: Duration) -> Result<(), ProbeError> {
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
            return Err(ProbeError::local(format!(
                "cannot set the receive timeout: {}",
                io::Error::last_os_error()
            )));
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
    use std::time::{Duration, Instant};

    use super::super::{ProbeError, Verdict};
    use super::{build_echo_request, icmp_message, is_expected_reply};

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
            return Err(ProbeError::local(
                "cannot open a raw ICMP socket: run the tool from an elevated shell",
            ));
        }
        let socket = Socket(raw);

        let milliseconds = timeout.as_millis().min(u32::MAX as u128) as u32;
        let result = unsafe {
            setsockopt(
                socket.0,
                SOL_SOCKET,
                SO_RCVTIMEO,
                &milliseconds as *const u32 as *const u8,
                4,
            )
        };
        if result != 0 {
            return Err(ProbeError::local("cannot set the receive timeout"));
        }

        if let Some(name) = interface {
            crate::probe::interface::bind(socket.0, name).map_err(ProbeError::local)?;
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
            return Err(ProbeError::local(format!(
                "cannot send the echo request to {target} (WSA error {code})"
            )));
        }

        let started = Instant::now();
        let mut buffer = [0u8; 1500];
        loop {
            let mut from = [0u8; 16];
            let mut from_length: i32 = from.len() as i32;
            let received = unsafe {
                recvfrom(
                    socket.0,
                    buffer.as_mut_ptr(),
                    buffer.len() as i32,
                    0,
                    from.as_mut_ptr(),
                    &mut from_length,
                )
            };

            if received < 0 {
                let code = unsafe { WSAGetLastError() };
                if code == WSAETIMEDOUT || code == WSAEWOULDBLOCK {
                    return Ok(Verdict::Down);
                }
                return Err(ProbeError::local(format!(
                    "cannot receive the echo reply (WSA error {code})"
                )));
            }

            let received = received as usize;
            if from[4..8] != target.octets() {
                continue;
            }
            if let Some(message) = icmp_message(&buffer[..received]) {
                if is_expected_reply(message, identifier, sequence) {
                    return Ok(Verdict::Up {
                        rtt: started.elapsed(),
                    });
                }
            }

            if started.elapsed() >= timeout {
                return Ok(Verdict::Down);
            }
        }
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

    pub fn exchange(
        _target: Ipv4Addr,
        _interface: Option<&str>,
        _timeout: Duration,
        _identifier: u16,
        _sequence: u16,
    ) -> Result<Verdict, ProbeError> {
        Err(ProbeError::local(
            "raw ICMP probing is not implemented for this operating system",
        ))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn ignores_echo_requests_and_errors() {
        let request = build_echo_request(42, 9);
        assert!(!is_expected_reply(&request, 42, 9));

        let mut unreachable = vec![0u8; 8];
        unreachable[0] = 3;
        assert!(!is_expected_reply(&unreachable, 42, 9));
    }
}
