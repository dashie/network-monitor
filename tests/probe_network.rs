//! Probes that leave the machine: the transports against public endpoints.
//!
//! They are ignored by default because they need a route to the internet, and DNS queries
//! leave the host:
//!
//! ```text
//! cargo test --test probe_network -- --ignored
//! ```
//!
//! Everything else in the suite reaches the same code through local servers, and the
//! decision logic through `probe::fake`.

use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use uptime_monitor::probe::dns::DnsProbe;
use uptime_monitor::probe::http::HttpProbe;
use uptime_monitor::probe::tcp::TcpProbe;
use uptime_monitor::probe::{Endpoint, Probe, RecordType, Scheme, StatusMatch, Verdict};

fn timeout() -> Duration {
    Duration::from_secs(5)
}

fn is_up(verdict: &Verdict) -> bool {
    matches!(verdict, Verdict::Up { .. })
}

fn tcp(host: &str, port: u16) -> Endpoint {
    Endpoint::Tcp {
        host: host.to_string(),
        port,
    }
}

fn dns(name: &str, resolver: Option<Ipv4Addr>) -> Endpoint {
    Endpoint::Dns {
        name: name.to_string(),
        resolver: resolver.map(IpAddr::V4),
        record: RecordType::A,
    }
}

fn https(host: &str, target: &str) -> Endpoint {
    Endpoint::Http {
        scheme: Scheme::Https,
        host: host.to_string(),
        port: Scheme::Https.default_port(),
        target: target.to_string(),
        expect: StatusMatch::default(),
    }
}

#[test]
#[ignore = "needs a route to the public internet"]
fn a_public_port_answers() {
    let verdict = TcpProbe::new()
        .probe(&tcp("1.1.1.1", 443), None, timeout())
        .expect("a local socket and a route can be had");

    assert!(
        is_up(&verdict),
        "expected 1.1.1.1:443 to answer, got {verdict:?}"
    );
}

#[test]
#[ignore = "needs a route to the public internet"]
fn a_public_name_resolves_through_a_given_server() {
    let server = Ipv4Addr::new(1, 1, 1, 1);
    let verdict = DnsProbe::new()
        .probe(&dns("one.one.one.one", Some(server)), None, timeout())
        .expect("a UDP socket and a route can be had");

    assert!(
        is_up(&verdict),
        "expected {server} to resolve the name, got {verdict:?}"
    );
}

#[test]
#[ignore = "needs a route to the public internet and a system resolver"]
fn a_public_name_resolves_through_the_system_resolver() {
    let verdict = DnsProbe::new()
        .probe(&dns("example.com", None), None, timeout())
        .expect("the system resolver answers or fails, it never errors");

    assert!(
        is_up(&verdict),
        "expected example.com to resolve, got {verdict:?}"
    );
}

#[test]
#[ignore = "needs a route to the public internet"]
fn a_name_that_does_not_exist_is_down_without_a_verdict_error() {
    let server = Ipv4Addr::new(1, 1, 1, 1);
    let verdict = DnsProbe::new()
        .probe(
            &dns("uptime-monitor.invalid", Some(server)),
            None,
            timeout(),
        )
        .expect("a definitive answer is a verdict, not an error");

    match &verdict {
        Verdict::Down(failure) => assert_eq!(failure.code(), "DNS.NXDOMAIN"),
        other => panic!("expected NXDOMAIN to be down, got {other:?}"),
    }
}

#[test]
#[ignore = "needs a route to the public internet"]
fn a_public_https_endpoint_answers_over_the_embedded_roots() {
    let verdict = HttpProbe::new()
        .probe(&https("example.com", "/"), None, timeout())
        .expect("a TLS handshake against the embedded roots");

    assert!(
        is_up(&verdict),
        "expected example.com to answer, got {verdict:?}"
    );
}
