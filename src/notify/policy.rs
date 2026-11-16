//! The notification decision: which state changes are reported, and when.
//!
//! Kept free of I/O, so the rules of FR-25 are testable without a network or a clock:
//! a `down` is reported as soon as it is recorded, an `up` only after it has held for
//! the stabilisation window (a **stable `up` window**), and a `down` that repeats an
//! already-notified `down` is suppressed.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::store::TargetState;

use super::message::{Event, Message};

/// The state a target event reports, or `None` for a global event.
fn state(message: &Message) -> Option<TargetState> {
    match message.event {
        Event::TargetDown { .. } => Some(TargetState::Down),
        Event::TargetUp { .. } => Some(TargetState::Up),
        _ => None,
    }
}

/// The reporting state of one target.
#[derive(Debug, Clone)]
pub struct Policy {
    stabilize: Duration,
    /// The last state actually reported.
    notified: Option<TargetState>,
    /// An `up` that has not yet held long enough, with the instant it may be reported.
    pending: Option<(Instant, Message)>,
}

impl Policy {
    pub fn new(stabilize: Duration) -> Self {
        Policy {
            stabilize,
            notified: None,
            pending: None,
        }
    }

    /// Feed an observed change and return the message to send now, if any.
    pub fn observe(&mut self, message: Message) -> Option<Message> {
        match state(&message)? {
            TargetState::Down => {
                // A disconnection cancels a recovery that has not been announced.
                self.pending = None;
                if self.notified == Some(TargetState::Down) {
                    return None;
                }
                self.notified = Some(TargetState::Down);
                Some(message)
            }
            TargetState::Up => {
                if self.notified == Some(TargetState::Up) {
                    return None;
                }
                if self.stabilize.is_zero() {
                    self.pending = None;
                    self.notified = Some(TargetState::Up);
                    return Some(message);
                }
                let deadline = message.at + self.stabilize;
                // The window runs from the first `up` of the episode; a later one
                // neither shortens nor restarts it.
                if let Some((existing, _)) = &self.pending {
                    if *existing <= deadline {
                        return None;
                    }
                }
                self.pending = Some((deadline, message));
                None
            }
            TargetState::Unknown => None,
        }
    }

    /// When the pending `up`, if any, becomes reportable.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending.as_ref().map(|(deadline, _)| *deadline)
    }

    /// Report the pending `up` once its window has closed.
    pub fn due(&mut self, now: Instant) -> Option<Message> {
        match &self.pending {
            Some((deadline, _)) if *deadline <= now => {}
            _ => return None,
        }
        let (_, message) = self.pending.take()?;
        self.notified = Some(TargetState::Up);
        Some(message)
    }
}

/// One policy per target, keyed by the target key.
#[derive(Debug)]
pub struct Engine {
    stabilize: Duration,
    policies: HashMap<String, Policy>,
}

impl Engine {
    pub fn new(stabilize: Duration) -> Self {
        Engine {
            stabilize,
            policies: HashMap::new(),
        }
    }

    /// Feed an observed change and return the message to send now, if any.
    ///
    /// A global event is never given to a policy: it bypasses the window and the
    /// per-target state.
    pub fn observe(&mut self, message: Message) -> Option<Message> {
        let key = message.target_key()?.to_string();
        self.policies
            .entry(key)
            .or_insert_with(|| Policy::new(self.stabilize))
            .observe(message)
    }

    /// The earliest instant at which [`Engine::due_all`] has something to return.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.policies
            .values()
            .filter_map(Policy::next_deadline)
            .min()
    }

    /// Every message whose window has closed, in a deterministic order.
    pub fn due_all(&mut self, now: Instant) -> Vec<Message> {
        let mut due: Vec<Message> = self
            .policies
            .values_mut()
            .filter_map(|policy| policy.due(now))
            .collect();
        due.sort_by(|left, right| {
            left.occurred_at_ms
                .cmp(&right.occurred_at_ms)
                .then_with(|| left.target_key().cmp(&right.target_key()))
        });
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notify::message::{Cause, TargetRef};

    fn down(key: &str, at: Instant) -> Message {
        let mut message = Message::target_down(
            TargetRef::new(key, None),
            Cause::new("ICMP.NO_REPLY", "no reply"),
            0,
        );
        message.at = at;
        message
    }

    fn up(key: &str, at: Instant) -> Message {
        let mut message = Message::target_up(TargetRef::new(key, None), 0);
        message.at = at;
        message
    }

    fn is_down(message: &Message) -> bool {
        matches!(message.event, Event::TargetDown { .. })
    }

    fn is_up(message: &Message) -> bool {
        matches!(message.event, Event::TargetUp { .. })
    }

    fn window() -> Duration {
        Duration::from_secs(60)
    }

    #[test]
    fn a_disconnection_is_reported_at_once() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        let sent = policy
            .observe(down("icmp://1.1.1.1", now))
            .expect("a disconnection needs no window");
        assert!(is_down(&sent));
        assert_eq!(policy.next_deadline(), None);
    }

    #[test]
    fn a_recovery_is_reported_only_after_the_window() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        assert!(policy.observe(down("icmp://1.1.1.1", now)).is_some());

        let up_at = now + Duration::from_secs(5);
        assert!(policy.observe(up("icmp://1.1.1.1", up_at)).is_none());
        assert_eq!(policy.next_deadline(), Some(up_at + window()));

        assert!(policy
            .due(up_at + window() - Duration::from_millis(1))
            .is_none());
        let sent = policy.due(up_at + window()).expect("the window closed");
        assert!(is_up(&sent));
        assert_eq!(policy.next_deadline(), None);
    }

    #[test]
    fn a_disconnection_inside_the_window_cancels_the_recovery() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        policy.observe(down("icmp://1.1.1.1", now));

        let up_at = now + Duration::from_secs(5);
        policy.observe(up("icmp://1.1.1.1", up_at));
        assert!(
            policy
                .observe(down("icmp://1.1.1.1", up_at + Duration::from_secs(10)))
                .is_none(),
            "the operator already knows the target is down"
        );
        assert!(policy.due(up_at + window()).is_none());
    }

    #[test]
    fn a_flap_reports_one_disconnection_per_incident() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        assert!(policy.observe(down("icmp://1.1.1.1", now)).is_some());
        assert!(policy.observe(up("icmp://1.1.1.1", now)).is_none());
        assert!(
            policy.observe(down("icmp://1.1.1.1", now)).is_none(),
            "a second down in the same incident is silent"
        );
    }

    #[test]
    fn a_disconnection_after_a_notified_recovery_is_reported_again() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        policy.observe(down("icmp://1.1.1.1", now));
        let up_at = now + Duration::from_secs(10);
        policy.observe(up("icmp://1.1.1.1", up_at));
        let recovered = policy
            .due(up_at + window())
            .expect("the recovery is reported");
        assert!(is_up(&recovered));

        let again = policy
            .observe(down("icmp://1.1.1.1", up_at + window()))
            .expect("a new incident is a new message");
        assert!(is_down(&again));
    }

    #[test]
    fn a_zero_window_reports_a_recovery_at_once() {
        let now = Instant::now();
        let mut policy = Policy::new(Duration::ZERO);
        policy.observe(down("icmp://1.1.1.1", now));

        let sent = policy
            .observe(up("icmp://1.1.1.1", now))
            .expect("no window means no waiting");
        assert!(is_up(&sent));
        assert!(policy.observe(up("icmp://1.1.1.1", now)).is_none());
    }

    #[test]
    fn the_first_verdict_reports_the_observed_state() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        assert!(policy.observe(down("icmp://1.1.1.1", now)).is_some());
        assert!(policy.observe(up("icmp://1.1.1.1", now)).is_none());

        let mut fresh = Policy::new(window());
        assert!(fresh.observe(up("icmp://1.1.1.1", now)).is_none());
        assert_eq!(fresh.next_deadline(), Some(now + window()));
    }

    #[test]
    fn the_engine_keeps_one_policy_per_key() {
        let now = Instant::now();
        let mut engine = Engine::new(window());
        assert!(engine.observe(down("icmp://1.1.1.1", now)).is_some());
        assert!(engine.observe(down("icmp://1.1.1.1", now)).is_none());

        assert!(engine.observe(down("tcp://1.1.1.1:443", now)).is_some());
        assert_eq!(engine.next_deadline(), None);
        assert!(engine.due_all(now).is_empty());
    }

    #[test]
    fn due_all_returns_one_recovery_per_target() {
        let now = Instant::now();
        let mut engine = Engine::new(window());
        for key in ["icmp://1.1.1.1", "icmp://1.0.0.1"] {
            engine.observe(down(key, now));
            engine.observe(up(key, now));
        }

        assert_eq!(engine.next_deadline(), Some(now + window()));
        assert!(engine
            .due_all(now + window() - Duration::from_secs(1))
            .is_empty());

        let due = engine.due_all(now + window());
        assert_eq!(due.len(), 2);
        assert!(due.iter().all(is_up));
        assert!(
            engine.due_all(now + window()).is_empty(),
            "one message per window"
        );
    }

    #[test]
    fn a_global_event_is_never_given_to_a_policy() {
        let mut engine = Engine::new(window());
        assert!(engine.observe(Message::shutdown(0)).is_none());
        assert_eq!(engine.next_deadline(), None);
        assert!(engine.due_all(Instant::now()).is_empty());
    }
}
