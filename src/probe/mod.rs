//! Probe contract and the transports that implement it.
//!
//! [`Probe`] is the only boundary the monitor knows: a transport can be replaced by
//! [`fake::FakeProbe`], so the decision logic is tested without raw sockets or privileges
//! (INV-3). [`Endpoint`] states what has to be checked — one variant per method — and the
//! behaviour of each method lives in its own module (`icmp`, `tcp`, `dns`, `http`).

pub mod dns;
pub mod failure;
pub mod fake;
pub mod http;
pub mod icmp;
pub mod interface;
pub mod tcp;

use std::fmt;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use thiserror::Error;

pub use failure::{Code, Details, Failure};

/// The kind of reachability check a target asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Icmp,
    Tcp,
    Dns,
    Http,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Icmp => "icmp",
            Method::Tcp => "tcp",
            Method::Dns => "dns",
            Method::Http => "http",
        }
    }
}

impl fmt::Display for Method {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The scheme of an HTTP target: it selects the TLS layer, nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    Http,
    Https,
}

impl Scheme {
    pub fn as_str(self) -> &'static str {
        match self {
            Scheme::Http => "http",
            Scheme::Https => "https",
        }
    }

    pub fn default_port(self) -> u16 {
        match self {
            Scheme::Http => 80,
            Scheme::Https => 443,
        }
    }
}

impl fmt::Display for Scheme {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The DNS record type a target asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordType {
    A,
    Aaaa,
}

impl RecordType {
    pub fn as_str(self) -> &'static str {
        match self {
            RecordType::A => "A",
            RecordType::Aaaa => "AAAA",
        }
    }
}

impl fmt::Display for RecordType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Which HTTP statuses count as `up`: a status matches when its class or its code is
/// listed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMatch {
    /// Classes such as `2` for `2xx`.
    pub classes: Vec<u8>,
    /// Exact status codes such as `204`.
    pub codes: Vec<u16>,
}

impl StatusMatch {
    pub fn matches(&self, status: u16) -> bool {
        self.classes.contains(&((status / 100) as u8)) || self.codes.contains(&status)
    }

    /// Parse the configuration forms: a class (`2xx`) or an exact code (`204`).
    pub fn parse(parts: &[String]) -> Result<StatusMatch, String> {
        let mut classes = Vec::new();
        let mut codes = Vec::new();
        for part in parts {
            let text = part.trim();
            if text.is_empty() {
                continue;
            }
            match text.strip_suffix("xx").or_else(|| text.strip_suffix("XX")) {
                Some(digit) => {
                    let class: u8 = digit
                        .parse()
                        .map_err(|_| format!("'{text}' is not a class such as 2xx"))?;
                    if !(1..=5).contains(&class) {
                        return Err(format!("'{text}' is not a class between 1xx and 5xx"));
                    }
                    classes.push(class);
                }
                None => {
                    let code: u16 = text
                        .parse()
                        .map_err(|_| format!("'{text}' is not an HTTP status code"))?;
                    if !(100..=599).contains(&code) {
                        return Err(format!("'{text}' is not a status code between 100 and 599"));
                    }
                    codes.push(code);
                }
            }
        }

        if classes.is_empty() && codes.is_empty() {
            return Err("no status is expected".to_string());
        }
        Ok(StatusMatch { classes, codes })
    }

    /// The configuration form, for `--show-config`.
    pub fn specs(&self) -> Vec<String> {
        let mut specs: Vec<String> = self
            .classes
            .iter()
            .map(|class| format!("{class}xx"))
            .collect();
        specs.extend(self.codes.iter().map(u16::to_string));
        specs
    }
}

impl Default for StatusMatch {
    /// A redirect or a successful answer both mean "the endpoint replied".
    fn default() -> Self {
        StatusMatch {
            classes: vec![2, 3],
            codes: Vec::new(),
        }
    }
}

/// One reachability check: the method and its parameters.
///
/// It is also the identity of a target in the event database: [`Endpoint`] renders as the
/// canonical key (`tcp://1.1.1.1:443`), so two targets differ exactly when their endpoints
/// differ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Endpoint {
    /// One ICMP echo request, IPv4 only, optionally through an interface.
    Icmp { address: Ipv4Addr },
    /// One TCP connection to a port; the handshake completing is the answer.
    Tcp { host: String, port: u16 },
    /// One DNS question: a name that must resolve, through the system resolver or a server.
    Dns {
        name: String,
        resolver: Option<IpAddr>,
        record: RecordType,
    },
    /// One HTTP request whose status must fall inside the expectation.
    Http {
        scheme: Scheme,
        host: String,
        port: u16,
        target: String,
        expect: StatusMatch,
    },
}

impl Endpoint {
    pub fn method(&self) -> Method {
        match self {
            Endpoint::Icmp { .. } => Method::Icmp,
            Endpoint::Tcp { .. } => Method::Tcp,
            Endpoint::Dns { .. } => Method::Dns,
            Endpoint::Http { .. } => Method::Http,
        }
    }
}

impl fmt::Display for Endpoint {
    /// The canonical key stored in the database and shown in the page.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Endpoint::Icmp { address } => write!(formatter, "icmp://{address}"),
            Endpoint::Tcp { host, port } => write!(formatter, "tcp://{host}:{port}"),
            Endpoint::Dns {
                name,
                resolver: Some(resolver),
                ..
            } => write!(formatter, "dns://{name}@{resolver}"),
            Endpoint::Dns {
                name,
                resolver: None,
                ..
            } => write!(formatter, "dns://{name}"),
            Endpoint::Http {
                scheme,
                host,
                port,
                target,
                ..
            } => {
                write!(formatter, "{scheme}://{host}")?;
                if *port != scheme.default_port() {
                    write!(formatter, ":{port}")?;
                }
                write!(formatter, "{target}")
            }
        }
    }
}

/// The simulated day of one target: the answer of the simulator's probe.
///
/// One stretch of the day, exactly as [`crate::sim::day`] cuts it: whether the target
/// answered, when, and — when it did not — the failure a real probe would have reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulatedEpisode {
    pub state: SimulatedState,
    pub from_ms: i64,
    pub to_ms: i64,
    /// How long the answer took, in microseconds; `None` when the target did not answer.
    pub rtt_us: Option<i64>,
    /// Why the target did not answer; `None` when it did.
    pub cause: Option<Failure>,
}

/// Whether a simulated stretch is the target answering or failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatedState {
    Up,
    Down,
}

/// The timeout a simulated failure states: the one a target runs with unless it configures
/// its own, spelled here because a simulator has no configuration to read.
pub const SIMULATED_TIMEOUT: Duration = Duration::from_millis(2_000);

/// The simulated day of one target, in the vocabulary of its method.
///
/// This is the simulated half of the [`Probe`] contract: the real half asks the network,
/// this one asks nothing, and both answer in the same terms. The schedule of the day lives
/// in [`crate::sim`]; the method contributes the latencies and the failures it can state.
pub fn simulated_day(
    endpoint: &Endpoint,
    seed: u64,
    day_start_ms: i64,
    fault_rate: f64,
) -> Vec<SimulatedEpisode> {
    match endpoint {
        Endpoint::Icmp { .. } => icmp::simulated_day(endpoint, seed, day_start_ms, fault_rate),
        Endpoint::Tcp { .. } => tcp::simulated_day(endpoint, seed, day_start_ms, fault_rate),
        Endpoint::Dns { .. } => dns::simulated_day(endpoint, seed, day_start_ms, fault_rate),
        Endpoint::Http { .. } => http::simulated_day(endpoint, seed, day_start_ms, fault_rate),
    }
}

/// The schedule of one day, dressed with the latency and the failure of one method.
///
/// `answer` is called once for every stretch the target is up, `fail` once for every stretch
/// it is down, and the two draw from a stream of their own — seeded by the method's name, so
/// an ICMP target and a TCP target of the same schedule do not share their draws.
pub(crate) fn simulated_episodes(
    endpoint: &Endpoint,
    seed: u64,
    day_start_ms: i64,
    fault_rate: f64,
    purpose: &str,
    mut answer: impl FnMut(&mut crate::sim::rng::Rng) -> i64,
    mut fail: impl FnMut(&mut crate::sim::rng::Rng) -> Failure,
) -> Vec<SimulatedEpisode> {
    let key = endpoint.to_string();
    let mut rng = crate::sim::rng::Rng::for_draw(seed, &key, day_start_ms, purpose);
    crate::sim::day(seed, &key, day_start_ms, fault_rate)
        .into_iter()
        .map(|stretch| SimulatedEpisode {
            state: if stretch.up {
                SimulatedState::Up
            } else {
                SimulatedState::Down
            },
            from_ms: stretch.from_ms,
            to_ms: stretch.to_ms,
            rtt_us: stretch.up.then(|| answer(&mut rng)),
            cause: (!stretch.up).then(|| fail(&mut rng)),
        })
        .collect()
}

/// The outcome of one probe cycle.
///
/// A local failure is *not* a verdict: it is a [`ProbeError`], so the monitor can
/// never record "the target is down" when the host merely could not probe (INV-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The check succeeded, with the time it took.
    Up { rtt: Duration },
    /// The check failed, as a definitive answer of the target, with the reason it states.
    Down(Failure),
}

/// A failure of this host to perform the probe.
///
/// It speaks the same language as a verdict cause — a code and one standard message — so an
/// operator reads one vocabulary everywhere. It is still never persisted (INV-5): only a
/// verdict is, and a local failure appears in the log.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{failure}")]
pub struct ProbeError {
    failure: Failure,
}

impl ProbeError {
    pub fn of(code: Code, details: &Details<'_>) -> Self {
        ProbeError {
            failure: Failure::new(code, details),
        }
    }

    /// The short token, for a log line that can be filtered by it.
    pub fn code(&self) -> &'static str {
        self.failure.code()
    }

    /// The standard message of this occurrence.
    pub fn message(&self) -> &str {
        self.failure.message()
    }
}

/// One reachability check, executed through an optional interface.
///
/// The timeout belongs to the request, not to the prober: each target may carry its own,
/// and two targets of the same process may use different ones.
pub trait Probe: Send + Sync {
    fn probe(
        &self,
        endpoint: &Endpoint,
        interface: Option<&str>,
        timeout: Duration,
    ) -> Result<Verdict, ProbeError>;
}

/// The prober of the running process: one stateless transport per method.
#[derive(Default)]
pub struct Prober {
    icmp: icmp::IcmpProbe,
    tcp: tcp::TcpProbe,
    dns: dns::DnsProbe,
    http: http::HttpProbe,
}

impl Prober {
    pub fn new() -> Self {
        Prober::default()
    }
}

impl Probe for Prober {
    fn probe(
        &self,
        endpoint: &Endpoint,
        interface: Option<&str>,
        timeout: Duration,
    ) -> Result<Verdict, ProbeError> {
        match endpoint {
            Endpoint::Icmp { .. } => self.icmp.probe(endpoint, interface, timeout),
            Endpoint::Tcp { .. } => self.tcp.probe(endpoint, interface, timeout),
            Endpoint::Dns { .. } => self.dns.probe(endpoint, interface, timeout),
            Endpoint::Http { .. } => self.http.probe(endpoint, interface, timeout),
        }
    }
}

pub use icmp::IcmpProbe;
