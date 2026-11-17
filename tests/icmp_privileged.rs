//! Probes that touch the real ICMP stack.
//!
//! They are ignored by default because a raw socket needs root or CAP_NET_RAW:
//!
//! ```text
//! sudo -E cargo test --test icmp_privileged -- --ignored
//! ```
//!
//! Everything else in the suite runs unprivileged against `probe::fake`.

use std::net::Ipv4Addr;
use std::time::Duration;

use uptime_monitor::probe::{interface, Endpoint, IcmpProbe, Probe, Verdict};

fn probe() -> IcmpProbe {
    IcmpProbe::new()
}

fn loopback() -> Endpoint {
    Endpoint::Icmp {
        address: Ipv4Addr::LOCALHOST,
    }
}

#[test]
#[ignore = "requires root or CAP_NET_RAW"]
fn loopback_answers_an_echo_request() {
    let verdict = probe()
        .probe(&loopback(), None, Duration::from_secs(2))
        .expect("a raw ICMP socket can be opened");

    assert!(
        matches!(verdict, Verdict::Up { .. }),
        "expected an echo reply from 127.0.0.1, got {verdict:?}"
    );
}

#[test]
#[ignore = "requires root or CAP_NET_RAW"]
fn an_existing_interface_can_be_bound() {
    let name = if cfg!(target_os = "macos") {
        "lo0"
    } else {
        "lo"
    };
    interface::validate(name).expect("the loopback interface exists");

    let verdict = probe()
        .probe(&loopback(), Some(name), Duration::from_secs(2))
        .expect("a raw ICMP socket can be bound to the interface");

    assert!(
        matches!(verdict, Verdict::Up { .. }),
        "expected an echo reply on {name}, got {verdict:?}"
    );
}

/// The timeout is a property of the request: this probe sends to a TEST-NET address that
/// does not answer, so the call must return `down` after the timeout it was given.
#[test]
#[ignore = "requires root or CAP_NET_RAW"]
fn the_timeout_of_the_request_bounds_the_wait() {
    let unreachable = Endpoint::Icmp {
        address: Ipv4Addr::new(192, 0, 2, 1),
    };

    let started = Duration::from_millis(120);
    let fast = std::time::Instant::now();
    let verdict = probe()
        .probe(&unreachable, None, started)
        .expect("a raw ICMP socket can be opened");
    let elapsed = fast.elapsed();

    assert!(
        matches!(&verdict, Verdict::Down(failure) if failure.code() == "ICMP.NO_REPLY"),
        "expected no reply to be reported, got {verdict:?}"
    );
    assert!(
        elapsed >= started,
        "the probe returned before its timeout: {elapsed:?}"
    );
    assert!(
        elapsed < started * 4,
        "the probe waited far beyond its timeout: {elapsed:?}"
    );
}

#[test]
#[cfg(unix)]
fn a_missing_interface_is_rejected() {
    let error = interface::validate("uptime-monitor-no-such-interface").unwrap_err();
    assert!(
        error.contains("does not exist"),
        "unexpected message: {error}"
    );
}
