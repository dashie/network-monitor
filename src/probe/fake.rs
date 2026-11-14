//! Deterministic prober for tests.
//!
//! Tests use it instead of raw sockets, so the suite runs without privileges and
//! without a network (INV-3).

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::Duration;

use super::{Code, Details, Endpoint, Failure, Probe, ProbeError, Verdict};

/// One request the fake received, so a test can assert what the monitor asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Call {
    pub endpoint: Endpoint,
    pub interface: Option<String>,
    pub timeout: Duration,
}

/// Replays a scripted sequence and falls back to a fixed verdict afterwards.
pub struct FakeProbe {
    script: Mutex<VecDeque<Result<Verdict, ProbeError>>>,
    fallback: Verdict,
    calls: Mutex<Vec<Call>>,
}

impl FakeProbe {
    /// Always returns the same verdict.
    pub fn always(verdict: Verdict) -> Self {
        FakeProbe {
            script: Mutex::new(VecDeque::new()),
            fallback: verdict,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Replays the script, then repeats the last scripted item.
    pub fn scripted(items: Vec<Result<Verdict, ProbeError>>) -> Self {
        let fallback = match items.last() {
            Some(Ok(verdict)) => verdict.clone(),
            _ => down(),
        };
        FakeProbe {
            script: Mutex::new(items.into()),
            fallback,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Replays verdicts, then repeats the last one.
    pub fn from_verdicts(verdicts: Vec<Verdict>) -> Self {
        let items = verdicts.into_iter().map(Ok).collect();
        FakeProbe::scripted(items)
    }
}

impl FakeProbe {
    /// Every request the fake received, oldest first.
    pub fn calls(&self) -> Vec<Call> {
        self.calls
            .lock()
            .expect("the call lock is not poisoned")
            .clone()
    }
}

impl Probe for FakeProbe {
    fn probe(
        &self,
        endpoint: &Endpoint,
        interface: Option<&str>,
        timeout: Duration,
    ) -> Result<Verdict, ProbeError> {
        self.calls
            .lock()
            .expect("the call lock is not poisoned")
            .push(Call {
                endpoint: endpoint.clone(),
                interface: interface.map(str::to_string),
                timeout,
            });

        let mut script = self.script.lock().expect("the script lock is not poisoned");
        match script.len() {
            0 => Ok(self.fallback.clone()),
            1 => script.front().cloned().expect("the script is not empty"),
            _ => script.pop_front().expect("the script is not empty"),
        }
    }
}

/// A convenient "up" verdict with a fixed round-trip time.
pub fn up() -> Verdict {
    Verdict::Up {
        rtt: Duration::from_micros(1_234),
    }
}

/// A "down" verdict whose cause is the one a silent target produces.
pub fn down() -> Verdict {
    down_with(Code::TcpTimeout)
}

/// A "down" verdict with a stated cause, for a test that asserts the code.
pub fn down_with(code: Code) -> Verdict {
    Verdict::Down(Failure::new(
        code,
        &Details {
            host: Some("1.1.1.1"),
            port: Some(443),
            ..Details::default()
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn endpoint() -> Endpoint {
        Endpoint::Icmp {
            address: Ipv4Addr::new(1, 1, 1, 1),
        }
    }

    fn timeout() -> Duration {
        Duration::from_millis(500)
    }

    #[test]
    fn replays_the_script_then_repeats_the_last_item() {
        let probe = FakeProbe::from_verdicts(vec![up(), down()]);
        assert_eq!(probe.probe(&endpoint(), None, timeout()).unwrap(), up());
        assert_eq!(probe.probe(&endpoint(), None, timeout()).unwrap(), down());
        assert_eq!(probe.probe(&endpoint(), None, timeout()).unwrap(), down());
    }

    #[test]
    fn always_returns_the_fallback() {
        let probe = FakeProbe::always(down());
        assert_eq!(probe.probe(&endpoint(), None, timeout()).unwrap(), down());
        assert_eq!(probe.probe(&endpoint(), None, timeout()).unwrap(), down());
    }

    #[test]
    fn records_every_request_it_received() {
        let probe = FakeProbe::always(up());
        probe
            .probe(&endpoint(), Some("eth1"), Duration::from_millis(1500))
            .unwrap();

        let calls = probe.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].endpoint, endpoint());
        assert_eq!(calls[0].interface.as_deref(), Some("eth1"));
        assert_eq!(calls[0].timeout, Duration::from_millis(1500));
    }
}
