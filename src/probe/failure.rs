//! The vocabulary of a failure: one code, and one standard message.
//!
//! Every failure of a probe cycle is a [`Failure`]. Its **code** names what happened in a
//! short token that can be grepped in a log, filtered in a table, and stored in the database;
//! its **message** is one sentence, written once here as a template, with the details of the
//! occurrence inserted when it is rendered. No other module words a failure: a call site picks
//! a [`Code`] and fills a [`Details`], and the sentence comes from this file.
//!
//! The prefix of a code names the layer that reported the failure, not the method of the
//! target: an `http` target whose port is closed states `TCP.REFUSED`, which is what happened.

use std::fmt;
use std::time::Duration;

/// The values a template may insert.
///
/// Every field is optional because every code uses a handful of them; a template that asks for
/// a value the code did not supply renders `unknown` rather than a raw placeholder.
#[derive(Debug, Default, Clone, Copy)]
pub struct Details<'a> {
    pub host: Option<&'a str>,
    pub port: Option<u16>,
    pub name: Option<&'a str>,
    pub server: Option<&'a str>,
    pub address: Option<&'a str>,
    pub url: Option<&'a str>,
    pub interface: Option<&'a str>,
    pub status: Option<u16>,
    pub expect: Option<&'a str>,
    pub timeout: Option<Duration>,
    /// How many consecutive probes the condition held for.
    pub count: Option<u64>,
    pub method: Option<&'a str>,
    pub other: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// What went wrong, as a closed set.
///
/// A code is added here together with its template and with the details its prober supplies;
/// the tests fail if any of the three is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Code {
    IcmpNoReply,
    IcmpSocket,
    IcmpIo,
    IcmpInterface,
    TcpRefused,
    TcpReset,
    TcpTimeout,
    TcpSocket,
    TcpConnect,
    TcpResolve,
    TcpInterface,
    DnsNxDomain,
    DnsNoRecord,
    DnsTimeout,
    DnsUnresolved,
    DnsBuild,
    DnsSocket,
    DnsServerFailure,
    DnsUnusable,
    HttpUnexpectedStatus,
    HttpTls,
    HttpResolve,
    HttpTlsInit,
    HttpMalformed,
    HttpHeadLimit,
    ProbeWrongMethod,
    ProbeSlow,
}

impl Code {
    /// Every code, so a test can check that none is missing a code or a template.
    pub const ALL: [Code; 27] = [
        Code::IcmpNoReply,
        Code::IcmpSocket,
        Code::IcmpIo,
        Code::IcmpInterface,
        Code::TcpRefused,
        Code::TcpReset,
        Code::TcpTimeout,
        Code::TcpSocket,
        Code::TcpConnect,
        Code::TcpResolve,
        Code::TcpInterface,
        Code::DnsNxDomain,
        Code::DnsNoRecord,
        Code::DnsTimeout,
        Code::DnsUnresolved,
        Code::DnsBuild,
        Code::DnsSocket,
        Code::DnsServerFailure,
        Code::DnsUnusable,
        Code::HttpUnexpectedStatus,
        Code::HttpTls,
        Code::HttpResolve,
        Code::HttpTlsInit,
        Code::HttpMalformed,
        Code::HttpHeadLimit,
        Code::ProbeWrongMethod,
        Code::ProbeSlow,
    ];

    /// The short token stored with an event and shown in a table.
    pub fn code(self) -> &'static str {
        match self {
            Code::IcmpNoReply => "ICMP.NO_REPLY",
            Code::IcmpSocket => "ICMP.SOCKET",
            Code::IcmpIo => "ICMP.IO",
            Code::IcmpInterface => "ICMP.INTERFACE",
            Code::TcpRefused => "TCP.REFUSED",
            Code::TcpReset => "TCP.RESET",
            Code::TcpTimeout => "TCP.TIMEOUT",
            Code::TcpSocket => "TCP.SOCKET",
            Code::TcpConnect => "TCP.CONNECT",
            Code::TcpResolve => "TCP.RESOLVE",
            Code::TcpInterface => "TCP.INTERFACE",
            Code::DnsNxDomain => "DNS.NXDOMAIN",
            Code::DnsNoRecord => "DNS.NO_RECORD",
            Code::DnsTimeout => "DNS.TIMEOUT",
            Code::DnsUnresolved => "DNS.UNRESOLVED",
            Code::DnsBuild => "DNS.BUILD",
            Code::DnsSocket => "DNS.SOCKET",
            Code::DnsServerFailure => "DNS.SERVER_FAILURE",
            Code::DnsUnusable => "DNS.UNUSABLE",
            Code::HttpUnexpectedStatus => "HTTP.UNEXPECTED_STATUS",
            Code::HttpTls => "HTTP.TLS",
            Code::HttpResolve => "HTTP.RESOLVE",
            Code::HttpTlsInit => "HTTP.TLS_INIT",
            Code::HttpMalformed => "HTTP.MALFORMED",
            Code::HttpHeadLimit => "HTTP.HEAD_LIMIT",
            Code::ProbeWrongMethod => "PROBE.WRONG_METHOD",
            Code::ProbeSlow => "PROBE.SLOW",
        }
    }

    /// The standard message, with the placeholders the details fill.
    pub fn template(self) -> &'static str {
        match self {
            Code::IcmpNoReply => "no echo reply from {address} within {timeout}",
            Code::IcmpSocket => {
                "cannot open a raw ICMP socket: {reason} (root or CAP_NET_RAW is required)"
            }
            Code::IcmpIo => "cannot send or receive on the ICMP socket: {reason}",
            Code::IcmpInterface => "cannot bind the probe to {interface}: {reason}",
            Code::TcpRefused => "the connection to {host}:{port} was refused",
            Code::TcpReset => "the connection to {host}:{port} was reset",
            Code::TcpTimeout => "no answer from {host}:{port} within {timeout}",
            Code::TcpSocket => "cannot create the socket for {host}:{port}: {reason}",
            Code::TcpConnect => "cannot connect to {host}:{port}: {reason}",
            Code::TcpResolve => "cannot resolve {host}: {reason}",
            Code::TcpInterface => "cannot bind the probe to {interface}: {reason}",
            Code::DnsNxDomain => "{name} does not exist, as answered by {server}",
            Code::DnsNoRecord => {
                "{name} has no record of the requested type, as answered by {server}"
            }
            Code::DnsTimeout => "{server} did not answer the question for {name} within {timeout}",
            Code::DnsUnresolved => "{name} does not resolve through the system resolver",
            Code::DnsBuild => "{name} cannot be written as a DNS question: {reason}",
            Code::DnsSocket => "cannot ask {server}: {reason}",
            Code::DnsServerFailure => "{server} could not answer the question for {name}: {reason}",
            Code::DnsUnusable => {
                "{server} answered something that is not a usable answer: {reason}"
            }
            Code::HttpUnexpectedStatus => "{url} answered {status}, outside {expect}",
            Code::HttpTls => "the TLS handshake with {host}:{port} failed: {reason}",
            Code::HttpResolve => "cannot resolve {host}: {reason}",
            Code::HttpTlsInit => "cannot build the TLS client: {reason}",
            Code::HttpMalformed => "{url} answered something that is not an HTTP answer: {reason}",
            Code::HttpHeadLimit => "{url} sent a head larger than the {expect} limit",
            Code::ProbeWrongMethod => "the {method} prober cannot check a {other} target",
            Code::ProbeSlow => {
                "the round-trip time stayed above {timeout} for {count} consecutive probes"
            }
        }
    }
}

impl fmt::Display for Code {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// One failure: its code, and its message rendered from the template of that code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    code: &'static str,
    message: String,
}

impl Failure {
    pub fn new(code: Code, details: &Details<'_>) -> Self {
        Failure {
            code: code.code(),
            message: render(code.template(), details),
        }
    }

    /// The short token, as stored and as shown in a table.
    pub fn code(&self) -> &'static str {
        self.code
    }

    /// The standard message of this occurrence, details included.
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for Failure {
    /// Both halves, so a log line carries the token that identifies the failure.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} — {}", self.code, self.message)
    }
}

/// Fill the `{placeholders}` of one template.
fn render(template: &str, details: &Details<'_>) -> String {
    let mut message = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        message.push_str(&rest[..start]);
        let Some(offset) = rest[start..].find('}') else {
            message.push_str(&rest[start..]);
            return message;
        };
        message.push_str(&value(&rest[start + 1..start + offset], details));
        rest = &rest[start + offset + 1..];
    }
    message.push_str(rest);
    message
}

fn value(name: &str, details: &Details<'_>) -> String {
    let found = match name {
        "host" => details.host.map(str::to_string),
        "port" => details.port.map(|port| port.to_string()),
        "name" => details.name.map(str::to_string),
        "server" => details.server.map(str::to_string),
        "address" => details.address.map(str::to_string),
        "url" => details.url.map(str::to_string),
        "interface" => details.interface.map(str::to_string),
        "status" => details.status.map(|status| status.to_string()),
        "expect" => details.expect.map(str::to_string),
        "timeout" => details
            .timeout
            .map(|timeout| format!("{} ms", timeout.as_millis())),
        "count" => details.count.map(|count| count.to_string()),
        "method" => details.method.map(str::to_string),
        "other" => details.other.map(str::to_string),
        "reason" => details.reason.map(str::to_string),
        _ => None,
    };
    // A message must never show a raw placeholder to a reader.
    found.unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn everything() -> Details<'static> {
        Details {
            host: Some("1.1.1.1"),
            port: Some(443),
            name: Some("example.com"),
            server: Some("9.9.9.9"),
            address: Some("1.1.1.1"),
            url: Some("https://example.com/health"),
            interface: Some("eth1"),
            status: Some(503),
            expect: Some("200-399"),
            timeout: Some(Duration::from_millis(2_000)),
            count: Some(3),
            method: Some("tcp"),
            other: Some("dns"),
            reason: Some("the socket went away"),
        }
    }

    #[test]
    fn every_code_states_a_token_and_a_template() {
        for code in Code::ALL {
            assert!(!code.code().is_empty(), "{code:?} has no code");
            assert!(
                code.code().contains('.'),
                "{} must carry its layer prefix",
                code.code()
            );
            assert_eq!(
                code.code(),
                code.code().to_uppercase(),
                "a code is uppercase"
            );
            assert!(!code.template().is_empty(), "{code:?} has no template");
        }
    }

    #[test]
    fn no_two_codes_share_a_token() {
        let mut seen: Vec<&str> = Code::ALL.iter().map(|code| code.code()).collect();
        seen.sort_unstable();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "two codes collide");
    }

    #[test]
    fn every_template_renders_with_the_details_its_code_supplies() {
        let details = everything();
        for code in Code::ALL {
            let failure = Failure::new(code, &details);
            assert!(
                !failure.message().contains('{'),
                "{} left a placeholder: {}",
                code.code(),
                failure.message()
            );
            assert!(
                !failure.message().contains("unknown"),
                "{} asked for a detail no code supplies: {}",
                code.code(),
                failure.message()
            );
        }
    }

    #[test]
    fn a_detail_the_code_did_not_supply_reads_unknown() {
        let failure = Failure::new(Code::TcpRefused, &Details::default());
        assert!(
            failure.message().contains("unknown:unknown"),
            "got {}",
            failure.message()
        );
    }

    #[test]
    fn the_message_states_the_details_of_the_occurrence() {
        let failure = Failure::new(Code::HttpUnexpectedStatus, &everything());
        assert_eq!(failure.code(), "HTTP.UNEXPECTED_STATUS");
        assert_eq!(
            failure.message(),
            "https://example.com/health answered 503, outside 200-399"
        );

        let timeout = Failure::new(
            Code::TcpTimeout,
            &Details {
                host: Some("192.0.2.1"),
                port: Some(81),
                timeout: Some(Duration::from_millis(1_500)),
                ..Details::default()
            },
        );
        assert_eq!(
            timeout.message(),
            "no answer from 192.0.2.1:81 within 1500 ms"
        );
    }

    #[test]
    fn a_log_line_carries_the_token_and_the_message() {
        let failure = Failure::new(
            Code::TcpResolve,
            &Details {
                host: Some("example.invalid"),
                reason: Some("no address"),
                ..Details::default()
            },
        );
        assert_eq!(
            failure.to_string(),
            "TCP.RESOLVE — cannot resolve example.invalid: no address"
        );
    }
}
