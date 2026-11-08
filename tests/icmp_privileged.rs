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

use network_monitor::probe::{interface, IcmpProbe, Probe, Verdict};

fn probe() -> IcmpProbe {
    IcmpProbe::new(Duration::from_secs(2))
}

#[test]
#[ignore = "requires root or CAP_NET_RAW"]
fn loopback_answers_an_echo_request() {
    let verdict = probe()
        .probe(Ipv4Addr::LOCALHOST, None)
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
        .probe(Ipv4Addr::LOCALHOST, Some(name))
        .expect("a raw ICMP socket can be bound to the interface");

    assert!(
        matches!(verdict, Verdict::Up { .. }),
        "expected an echo reply on {name}, got {verdict:?}"
    );
}

#[test]
#[cfg(unix)]
fn a_missing_interface_is_rejected() {
    let error = interface::validate("network-monitor-no-such-interface").unwrap_err();
    assert!(
        error.contains("does not exist"),
        "unexpected message: {error}"
    );
}
