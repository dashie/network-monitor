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

/// Whether a simulated stretch is the target answering, answering slowly, or failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SimulatedState {
    Up,
    Degraded,
    Down,
}

/// What the simulator needs to know to produce a `degraded` stretch.
///
/// The monitor's `degraded_after_ms` and `degraded_after` decide whether a target can be
/// degraded at all and how long the stretch has to last to be plausible at its interval;
/// a threshold of `0` disables it, exactly as it does in the monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SimulatedRules {
    pub degraded_after_ms: u64,
    pub degraded_after: u32,
    pub interval: Duration,
}

impl Default for SimulatedRules {
    fn default() -> Self {
        SimulatedRules {
            degraded_after_ms: 0,
            degraded_after: 3,
            interval: Duration::from_secs(5),
        }
    }
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
    rules: SimulatedRules,
) -> Vec<SimulatedEpisode> {
    match endpoint {
        Endpoint::Icmp { .. } => {
            icmp::simulated_day(endpoint, seed, day_start_ms, fault_rate, rules)
        }
        Endpoint::Tcp { .. } => tcp::simulated_day(endpoint, seed, day_start_ms, fault_rate, rules),
        Endpoint::Dns { .. } => dns::simulated_day(endpoint, seed, day_start_ms, fault_rate, rules),
        Endpoint::Http { .. } => {
            http::simulated_day(endpoint, seed, day_start_ms, fault_rate, rules)
        }
    }
}

/// The schedule of one day, dressed with the latency and the failure of one method.
///
/// `answer` is called once for every stretch the target is up, `fail` once for every stretch
/// it is down, and the two draw from a stream of their own — seeded by the method's name, so
/// an ICMP target and a TCP target of the same schedule do not share their draws.
/// The schedule one simulated day is drawn from: the same tuple for the answers, the
/// failures and the slow windows.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SimulatedSchedule<'a> {
    pub seed: u64,
    pub day_start_ms: i64,
    pub fault_rate: f64,
    pub rules: SimulatedRules,
    pub purpose: &'a str,
}

pub(crate) fn simulated_episodes(
    endpoint: &Endpoint,
    schedule: SimulatedSchedule<'_>,
    mut answer: impl FnMut(&mut crate::sim::rng::Rng) -> i64,
    mut fail: impl FnMut(&mut crate::sim::rng::Rng) -> Failure,
) -> Vec<SimulatedEpisode> {
    let SimulatedSchedule {
        seed,
        day_start_ms,
        fault_rate,
        rules,
        purpose,
    } = schedule;
    let key = endpoint.to_string();
    let mut rng = crate::sim::rng::Rng::for_draw(seed, &key, day_start_ms, purpose);
    // Slow stretches draw from a stream of their own: they neither consume the draws of the
    // answers and failures nor depend on them.
    let mut slow_rng = crate::sim::rng::Rng::for_draw(seed, &key, day_start_ms, "slow");
    let mut remaining = slow_spells(rules.degraded_after_ms, crate::sim::life(seed, &key));

    let mut episodes = Vec::new();
    for stretch in crate::sim::day(seed, &key, day_start_ms, fault_rate) {
        if !stretch.up {
            episodes.push(SimulatedEpisode {
                state: SimulatedState::Down,
                from_ms: stretch.from_ms,
                to_ms: stretch.to_ms,
                rtt_us: None,
                cause: Some(fail(&mut rng)),
            });
            continue;
        }

        let windows = if remaining == 0 {
            Vec::new()
        } else {
            slow_windows(&stretch, rules, &mut slow_rng, remaining)
        };
        remaining = remaining.saturating_sub(windows.len());

        let mut cursor = stretch.from_ms;
        for (from_ms, to_ms) in windows {
            if from_ms > cursor {
                episodes.push(SimulatedEpisode {
                    state: SimulatedState::Up,
                    from_ms: cursor,
                    to_ms: from_ms,
                    rtt_us: Some(answer(&mut rng)),
                    cause: None,
                });
            }
            episodes.push(SimulatedEpisode {
                state: SimulatedState::Degraded,
                from_ms,
                to_ms,
                rtt_us: Some(slow_rtt(rules.degraded_after_ms, &mut slow_rng)),
                cause: Some(Failure::new(
                    Code::ProbeSlow,
                    &Details {
                        timeout: Some(Duration::from_millis(rules.degraded_after_ms)),
                        count: Some(u64::from(rules.degraded_after)),
                        ..Details::default()
                    },
                )),
            });
            cursor = to_ms;
        }
        if cursor < stretch.to_ms {
            episodes.push(SimulatedEpisode {
                state: SimulatedState::Up,
                from_ms: cursor,
                to_ms: stretch.to_ms,
                rtt_us: Some(answer(&mut rng)),
                cause: None,
            });
        }
    }
    episodes
}

/// How many slow spells a day a target that can degrade holds.
///
/// The stricter the threshold, the rarer a spell that crosses it: six a day at 250 ms or
/// less, three up to a second, one above. A flapping target holds twice as many, and a
/// target that never fails, or a threshold of `0`, holds none.
fn slow_spells(threshold_ms: u64, life: crate::sim::Life) -> usize {
    if threshold_ms == 0 {
        return 0;
    }
    let base = if threshold_ms <= 250 {
        6
    } else if threshold_ms <= 1_000 {
        3
    } else {
        1
    };
    match life {
        crate::sim::Life::AlwaysUp => 0,
        crate::sim::Life::Occasional => base,
        crate::sim::Life::Flapping => base * 2,
    }
}

/// The shortest slow window: even a fast interval must show a stretch a reader can see.
const MIN_SLOW_MS: i64 = 60_000;
/// The longest slow window: a degradation is a stretch, not a second outage.
const MAX_SLOW_MS: i64 = 3_600_000;

/// The slow windows inside one `up` stretch, in order and separated by an `up` piece.
///
/// Each lasts at least `degraded_after` probes at the target's interval, so it is plausible
/// at that interval, and the first and the last leave an `up` stretch on both sides: a first
/// verdict is never `degraded`, exactly as in the monitor (FR-40). A stretch with no room
/// holds none, and the day keeps the spells that fit.
fn slow_windows(
    stretch: &crate::sim::Stretch,
    rules: SimulatedRules,
    rng: &mut crate::sim::rng::Rng,
    limit: usize,
) -> Vec<(i64, i64)> {
    let needed = (i64::from(rules.degraded_after))
        .saturating_mul(rules.interval.as_millis() as i64)
        .max(MIN_SLOW_MS);
    let duration = needed.min(MAX_SLOW_MS);
    let lead = (rules.interval.as_millis() as i64).max(1);

    let mut windows = Vec::new();
    let mut cursor = stretch.from_ms;
    for _ in 0..limit {
        let low = cursor.saturating_add(lead);
        let high = stretch.to_ms.saturating_sub(duration).saturating_sub(lead);
        if high <= low {
            break;
        }
        let start = rng.between(low, high);
        let end = start.saturating_add(duration);
        windows.push((start, end));
        cursor = end;
    }
    windows
}

/// The round-trip time of a slow answer, in microseconds: at or above the threshold in
/// milliseconds, drawn from the stream.
fn slow_rtt(threshold_ms: u64, rng: &mut crate::sim::rng::Rng) -> i64 {
    let threshold = threshold_ms.saturating_mul(1_000).min(i64::MAX as u64) as i64;
    let spread = threshold.max(50_000);
    threshold.saturating_add(rng.between(0, spread))
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
