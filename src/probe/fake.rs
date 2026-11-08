//! Deterministic prober for tests.
//!
//! Tests use it instead of raw sockets, so the suite runs without privileges and
//! without a network (INV-3).

use std::collections::VecDeque;
use std::net::Ipv4Addr;
use std::sync::Mutex;
use std::time::Duration;

use super::{Probe, ProbeError, Verdict};

/// Replays a scripted sequence and falls back to a fixed verdict afterwards.
pub struct FakeProbe {
    script: Mutex<VecDeque<Result<Verdict, ProbeError>>>,
    fallback: Verdict,
}

impl FakeProbe {
    /// Always returns the same verdict.
    pub fn always(verdict: Verdict) -> Self {
        FakeProbe {
            script: Mutex::new(VecDeque::new()),
            fallback: verdict,
        }
    }

    /// Replays the script, then repeats the last scripted item.
    pub fn scripted(items: Vec<Result<Verdict, ProbeError>>) -> Self {
        let fallback = match items.last() {
            Some(Ok(verdict)) => *verdict,
            _ => Verdict::Down,
        };
        FakeProbe {
            script: Mutex::new(items.into()),
            fallback,
        }
    }

    /// Replays verdicts, then repeats the last one.
    pub fn from_verdicts(verdicts: Vec<Verdict>) -> Self {
        let items = verdicts.into_iter().map(Ok).collect();
        FakeProbe::scripted(items)
    }
}

impl Probe for FakeProbe {
    fn probe(&self, _target: Ipv4Addr, _interface: Option<&str>) -> Result<Verdict, ProbeError> {
        let mut script = self.script.lock().expect("the script lock is not poisoned");
        match script.len() {
            0 => Ok(self.fallback),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replays_the_script_then_repeats_the_last_item() {
        let probe = FakeProbe::from_verdicts(vec![up(), Verdict::Down]);
        let target = Ipv4Addr::new(1, 1, 1, 1);
        assert_eq!(probe.probe(target, None).unwrap(), up());
        assert_eq!(probe.probe(target, None).unwrap(), Verdict::Down);
        assert_eq!(probe.probe(target, None).unwrap(), Verdict::Down);
    }

    #[test]
    fn always_returns_the_fallback() {
        let probe = FakeProbe::always(Verdict::Down);
        let target = Ipv4Addr::new(1, 1, 1, 1);
        assert_eq!(probe.probe(target, None).unwrap(), Verdict::Down);
        assert_eq!(probe.probe(target, None).unwrap(), Verdict::Down);
    }
}
