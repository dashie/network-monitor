//! The transition decision.
//!
//! Kept free of I/O so the rule of INV-1 and INV-5 is testable in isolation.

use crate::probe::{ProbeError, Verdict};
use crate::store::TargetState;

/// A recorded change of state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Transition {
    pub from: TargetState,
    pub to: TargetState,
    pub rtt_us: Option<i64>,
}

/// Decide whether a probe outcome is a transition from the previous state.
///
/// A local probe failure never produces a transition (INV-5); a repeated verdict does
/// not either (INV-1, FR-5).
pub fn transition(
    previous: TargetState,
    outcome: &Result<Verdict, ProbeError>,
) -> Option<Transition> {
    let (state, rtt_us) = match outcome {
        Ok(Verdict::Up { rtt }) => (TargetState::Up, Some(rtt.as_micros() as i64)),
        Ok(Verdict::Down) => (TargetState::Down, None),
        Err(_) => return None,
    };

    if previous == state {
        return None;
    }

    Some(Transition {
        from: previous,
        to: state,
        rtt_us,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::probe::fake::up;
    use std::time::Duration;

    #[test]
    fn first_verdict_leaves_the_unknown_state() {
        let transition = transition(TargetState::Unknown, &Ok(up())).unwrap();
        assert_eq!(transition.from, TargetState::Unknown);
        assert_eq!(transition.to, TargetState::Up);
        assert_eq!(transition.rtt_us, Some(1_234));
    }

    #[test]
    fn a_repeated_verdict_is_not_a_transition() {
        assert!(transition(TargetState::Up, &Ok(up())).is_none());
        assert!(transition(TargetState::Down, &Ok(Verdict::Down)).is_none());
    }

    #[test]
    fn a_change_of_verdict_is_a_transition() {
        let down = transition(TargetState::Up, &Ok(Verdict::Down)).unwrap();
        assert_eq!((down.from, down.to), (TargetState::Up, TargetState::Down));
        assert_eq!(down.rtt_us, None);

        let recovered = transition(
            TargetState::Down,
            &Ok(Verdict::Up {
                rtt: Duration::from_micros(900),
            }),
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
        let failure = ProbeError::local("permission denied");
        assert!(transition(TargetState::Up, &Err(failure.clone())).is_none());
        assert!(transition(TargetState::Down, &Err(failure)).is_none());
    }
}
