//! The transition decision.
//!
//! Kept free of I/O so the rule of INV-1 and INV-5 is testable in isolation.

use std::time::Duration;

use crate::probe::{Code, Details, Failure, ProbeError, Verdict};
use crate::store::TargetState;

/// A recorded change of state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transition {
    pub from: TargetState,
    pub to: TargetState,
    pub rtt_us: Option<i64>,
    /// Why the target went down; `None` on the way back up.
    pub cause: Option<Failure>,
}

/// The damping rules the tracker applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rules {
    /// Consecutive failed probes before a target is recorded `down`; `1` is immediate.
    pub down_after: u32,
    /// Round-trip time at or above which a reachable probe is slow; `0` disables `degraded`.
    pub degraded_after_ms: u64,
    /// Consecutive slow probes needed to enter `degraded` (and fast ones to leave it).
    pub degraded_after: u32,
}

/// The per-target state machine: the last recorded state and the streaks that feed it.
///
/// A local probe failure never produces a transition and never touches a counter
/// (INV-5); a repeated verdict does not either (INV-1, FR-5).
#[derive(Debug, Clone)]
pub struct Tracker {
    state: TargetState,
    down_streak: u32,
    slow_streak: u32,
    fast_streak: u32,
}

impl Tracker {
    /// Start from the last state the store recorded; a target seen for the first time
    /// starts `unknown`.
    pub fn start(state: TargetState) -> Self {
        Tracker {
            state,
            down_streak: 0,
            slow_streak: 0,
            fast_streak: 0,
        }
    }

    /// Feed one probe outcome and return the transition it produced, if any.
    pub fn observe(
        &mut self,
        outcome: &Result<Verdict, ProbeError>,
        rules: &Rules,
    ) -> Option<Transition> {
        match outcome {
            Err(_) => None,
            Ok(Verdict::Down(failure)) => {
                self.down_streak += 1;
                self.slow_streak = 0;
                self.fast_streak = 0;
                if self.state == TargetState::Down {
                    return None;
                }
                if self.state != TargetState::Unknown && self.down_streak < rules.down_after {
                    return None;
                }
                self.down_streak = 0;
                self.transition_to(TargetState::Down, None, Some(failure.clone()))
            }
            Ok(Verdict::Up { rtt }) => {
                self.down_streak = 0;
                let rtt_us = rtt.as_micros() as i64;
                match self.state {
                    TargetState::Up | TargetState::Degraded => {
                        let slow = rules.degraded_after_ms > 0
                            && *rtt >= Duration::from_millis(rules.degraded_after_ms);
                        if slow {
                            self.slow_streak += 1;
                            self.fast_streak = 0;
                            if self.state == TargetState::Up
                                && self.slow_streak >= rules.degraded_after
                            {
                                self.slow_streak = 0;
                                return self.transition_to(
                                    TargetState::Degraded,
                                    Some(rtt_us),
                                    Some(slow_cause(rules)),
                                );
                            }
                            return None;
                        }

                        self.slow_streak = 0;
                        self.fast_streak += 1;
                        if self.state == TargetState::Degraded
                            && self.fast_streak >= rules.degraded_after
                        {
                            self.fast_streak = 0;
                            return self.transition_to(TargetState::Up, Some(rtt_us), None);
                        }
                        return None;
                    }
                    _ => {}
                }
                // A first verdict or a recovery is immediate, whatever the damping says.
                self.slow_streak = 0;
                self.fast_streak = 0;
                self.transition_to(TargetState::Up, Some(rtt_us), None)
            }
        }
    }

    fn transition_to(
        &mut self,
        to: TargetState,
        rtt_us: Option<i64>,
        cause: Option<Failure>,
    ) -> Option<Transition> {
        if self.state == to {
            return None;
        }
        let from = std::mem::replace(&mut self.state, to);
        Some(Transition {
            from,
            to,
            rtt_us,
            cause,
        })
    }
}

/// Why a reachable target became `degraded`: the threshold it crossed, and for how long.
fn slow_cause(rules: &Rules) -> Failure {
    Failure::new(
        Code::ProbeSlow,
        &Details {
            timeout: Some(Duration::from_millis(rules.degraded_after_ms)),
            count: Some(rules.degraded_after as u64),
            ..Details::default()
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::fake::{down, down_with, up};
    use crate::probe::{Code, Details};
    use std::time::Duration;

    fn rules(down_after: u32) -> Rules {
        Rules {
            down_after,
            degraded_after_ms: 0,
            degraded_after: 3,
        }
    }

    fn degrading(down_after: u32) -> Rules {
        Rules {
            down_after,
            degraded_after_ms: 200,
            degraded_after: 3,
        }
    }

    fn slow() -> Verdict {
        Verdict::Up {
            rtt: Duration::from_millis(250),
        }
    }

    fn fast() -> Verdict {
        Verdict::Up {
            rtt: Duration::from_millis(20),
        }
    }

    #[test]
    fn first_verdict_leaves_the_unknown_state() {
        let mut tracker = Tracker::start(TargetState::Unknown);
        let transition = tracker.observe(&Ok(up()), &rules(1)).unwrap();
        assert_eq!(transition.from, TargetState::Unknown);
        assert_eq!(transition.to, TargetState::Up);
        assert_eq!(transition.rtt_us, Some(1_234));
    }

    #[test]
    fn a_first_verdict_is_down_at_once_whatever_the_damping_says() {
        let mut tracker = Tracker::start(TargetState::Unknown);
        let transition = tracker.observe(&Ok(down()), &rules(3)).unwrap();
        assert_eq!(transition.from, TargetState::Unknown);
        assert_eq!(transition.to, TargetState::Down);
    }

    #[test]
    fn a_repeated_verdict_is_not_a_transition() {
        let mut tracker = Tracker::start(TargetState::Up);
        assert!(tracker.observe(&Ok(up()), &rules(1)).is_none());
        let mut tracker = Tracker::start(TargetState::Down);
        assert!(tracker.observe(&Ok(down()), &rules(1)).is_none());
    }

    #[test]
    fn the_cause_of_a_disconnection_reaches_the_transition() {
        let mut tracker = Tracker::start(TargetState::Up);
        let recorded = tracker
            .observe(&Ok(down_with(Code::TcpRefused)), &rules(1))
            .unwrap();
        let cause = recorded.cause.expect("a disconnection states why");
        assert_eq!(cause.code(), "TCP.REFUSED");
        assert_eq!(cause.message(), "the connection to 1.1.1.1:443 was refused");

        let back = tracker.observe(&Ok(up()), &rules(1)).unwrap();
        assert_eq!(back.cause, None, "a return to up carries no cause");
    }

    #[test]
    fn a_change_of_verdict_is_a_transition() {
        let mut tracker = Tracker::start(TargetState::Up);
        let down = tracker.observe(&Ok(down()), &rules(1)).unwrap();
        assert_eq!((down.from, down.to), (TargetState::Up, TargetState::Down));
        assert_eq!(down.rtt_us, None);

        let mut tracker = Tracker::start(TargetState::Down);
        let recovered = tracker
            .observe(
                &Ok(Verdict::Up {
                    rtt: Duration::from_micros(900),
                }),
                &rules(1),
            )
            .unwrap();
        assert_eq!(
            (recovered.from, recovered.to),
            (TargetState::Down, TargetState::Up)
        );
        assert_eq!(recovered.rtt_us, Some(900));
    }

    #[test]
    fn a_local_error_never_changes_the_state() {
        let failure = ProbeError::of(
            Code::IcmpSocket,
            &Details {
                reason: Some("permission denied"),
                ..Details::default()
            },
        );
        let mut tracker = Tracker::start(TargetState::Up);
        assert!(tracker.observe(&Err(failure.clone()), &rules(1)).is_none());
        let mut tracker = Tracker::start(TargetState::Down);
        assert!(tracker.observe(&Err(failure), &rules(1)).is_none());
    }

    #[test]
    fn damping_waits_for_the_configured_number_of_failures() {
        let mut tracker = Tracker::start(TargetState::Up);

        assert!(tracker.observe(&Ok(down()), &rules(3)).is_none());
        assert!(tracker.observe(&Ok(down()), &rules(3)).is_none());
        // A success in between resets the streak, so the following two record nothing
        // and the third records the transition.
        assert!(tracker.observe(&Ok(up()), &rules(3)).is_none());
        assert!(tracker.observe(&Ok(down()), &rules(3)).is_none());
        assert!(tracker.observe(&Ok(down()), &rules(3)).is_none());

        let down = tracker.observe(&Ok(down()), &rules(3)).unwrap();
        assert_eq!((down.from, down.to), (TargetState::Up, TargetState::Down));
    }

    #[test]
    fn a_local_error_between_failures_does_not_reset_the_streak() {
        let failure = ProbeError::of(
            Code::IcmpSocket,
            &Details {
                reason: Some("permission denied"),
                ..Details::default()
            },
        );
        let mut tracker = Tracker::start(TargetState::Up);

        assert!(tracker.observe(&Ok(down()), &rules(3)).is_none());
        assert!(tracker.observe(&Err(failure), &rules(3)).is_none());
        assert!(tracker.observe(&Ok(down()), &rules(3)).is_none());

        let down = tracker.observe(&Ok(down()), &rules(3)).unwrap();
        assert_eq!((down.from, down.to), (TargetState::Up, TargetState::Down));
    }

    #[test]
    fn one_failure_is_an_immediate_transition_by_default() {
        let mut tracker = Tracker::start(TargetState::Up);
        let down = tracker.observe(&Ok(down()), &rules(1)).unwrap();
        assert_eq!((down.from, down.to), (TargetState::Up, TargetState::Down));
    }

    #[test]
    fn a_slow_target_becomes_degraded_after_the_configured_probes() {
        let mut tracker = Tracker::start(TargetState::Up);
        assert!(tracker.observe(&Ok(slow()), &degrading(1)).is_none());
        assert!(tracker.observe(&Ok(slow()), &degrading(1)).is_none());
        let entered = tracker.observe(&Ok(slow()), &degrading(1)).unwrap();
        assert_eq!(
            (entered.from, entered.to),
            (TargetState::Up, TargetState::Degraded)
        );
        assert_eq!(entered.rtt_us, Some(250_000));
        let cause = entered.cause.expect("a degradation states why");
        assert_eq!(cause.code(), "PROBE.SLOW");
        assert_eq!(
            cause.message(),
            "the round-trip time stayed above 200 ms for 3 consecutive probes"
        );
    }

    #[test]
    fn a_degraded_target_returns_up_after_the_configured_fast_probes() {
        let mut tracker = Tracker::start(TargetState::Up);
        for _ in 0..3 {
            tracker.observe(&Ok(slow()), &degrading(1));
        }
        assert!(tracker.observe(&Ok(fast()), &degrading(1)).is_none());
        assert!(tracker.observe(&Ok(fast()), &degrading(1)).is_none());
        let back = tracker.observe(&Ok(fast()), &degrading(1)).unwrap();
        assert_eq!(
            (back.from, back.to),
            (TargetState::Degraded, TargetState::Up)
        );
        assert_eq!(back.cause, None, "a recovery carries no cause");
    }

    #[test]
    fn a_slow_probe_keeps_a_degraded_target_degraded_and_a_fast_one_resets_the_count() {
        let mut tracker = Tracker::start(TargetState::Degraded);
        assert!(tracker.observe(&Ok(slow()), &degrading(1)).is_none());
        assert!(tracker.observe(&Ok(fast()), &degrading(1)).is_none());
        assert!(tracker.observe(&Ok(fast()), &degrading(1)).is_none());
        // A slow probe in between resets the fast streak, so the third fast one is not
        // enough: hysteresis in both directions.
        assert!(tracker.observe(&Ok(slow()), &degrading(1)).is_none());
        assert!(tracker.observe(&Ok(fast()), &degrading(1)).is_none());
        assert!(tracker.observe(&Ok(fast()), &degrading(1)).is_none());
        let back = tracker.observe(&Ok(fast()), &degrading(1)).unwrap();
        assert_eq!(back.to, TargetState::Up);
    }

    #[test]
    fn a_zero_threshold_makes_degraded_unreachable() {
        let mut tracker = Tracker::start(TargetState::Up);
        for _ in 0..10 {
            assert!(tracker.observe(&Ok(slow()), &rules(1)).is_none());
        }
        let down = tracker.observe(&Ok(down()), &rules(1)).unwrap();
        assert_eq!(down.to, TargetState::Down);
    }

    #[test]
    fn a_down_verdict_from_degraded_follows_the_damping_rule() {
        let mut tracker = Tracker::start(TargetState::Degraded);
        assert!(tracker.observe(&Ok(down()), &degrading(3)).is_none());
        assert!(tracker.observe(&Ok(down()), &degrading(3)).is_none());
        let down = tracker.observe(&Ok(down()), &degrading(3)).unwrap();
        assert_eq!(
            (down.from, down.to),
            (TargetState::Degraded, TargetState::Down)
        );
    }

    #[test]
    fn a_recovery_from_down_is_up_even_when_the_probe_is_slow() {
        let mut tracker = Tracker::start(TargetState::Down);
        let first = tracker.observe(&Ok(slow()), &degrading(1)).unwrap();
        assert_eq!(first.to, TargetState::Up, "down never becomes degraded");
        assert_eq!(first.cause, None);
    }

    #[test]
    fn a_first_slow_verdict_after_a_gap_is_up() {
        let mut tracker = Tracker::start(TargetState::Unknown);
        let first = tracker.observe(&Ok(slow()), &degrading(1)).unwrap();
        assert_eq!(first.to, TargetState::Up, "FR-6 stays immediate");
        // The degradation count starts then, not at the first probe.
        assert!(tracker.observe(&Ok(slow()), &degrading(1)).is_none());
    }
}
