//! The notification decision: which state changes are reported, and when.
//!
//! Kept free of I/O, so the rules of FR-25 are testable without a network or a clock:
//! a `down` is reported as soon as it is recorded, an `up` only after it has held for
//! the stabilisation window (a **stable `up` window**), and a `down` that repeats an
//! already-notified `down` is suppressed.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::store::TargetState;
use crate::time::format_rfc3339;

/// One state change worth telling the operator about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notification {
    /// The label shown to the operator, or the key when the target has no label.
    pub target: String,
    /// The identity of the target in the database.
    pub key: String,
    pub state: TargetState,
    /// The code and message of a failure; `None` on the way back up.
    pub cause: Option<String>,
    /// When the event was recorded, as epoch milliseconds.
    pub occurred_at_ms: i64,
    /// The instant the notification was created, which starts the stabilisation window.
    pub at: Instant,
}

impl Notification {
    /// The plain-text message, one fact per line.
    pub fn text(&self) -> String {
        let state = self.state.as_str().to_ascii_uppercase();
        let mut text = if self.target == self.key {
            format!("{state} {}", self.target)
        } else {
            format!("{state} {} ({})", self.target, self.key)
        };
        if let Some(cause) = &self.cause {
            text.push_str("\nCause: ");
            text.push_str(cause);
        }
        text.push('\n');
        text.push_str(&format_rfc3339(self.occurred_at_ms));
        text
    }
}

/// The reporting state of one target.
#[derive(Debug, Clone)]
pub struct Policy {
    stabilize: Duration,
    /// The last state actually reported.
    notified: Option<TargetState>,
    /// An `up` that has not yet held long enough, with the instant it may be reported.
    pending: Option<(Instant, Notification)>,
}

impl Policy {
    pub fn new(stabilize: Duration) -> Self {
        Policy {
            stabilize,
            notified: None,
            pending: None,
        }
    }

    /// Feed an observed change and return the notification to send now, if any.
    pub fn observe(&mut self, notification: Notification) -> Option<Notification> {
        match notification.state {
            TargetState::Down => {
                // A disconnection cancels a recovery that has not been announced.
                self.pending = None;
                if self.notified == Some(TargetState::Down) {
                    return None;
                }
                self.notified = Some(TargetState::Down);
                Some(notification)
            }
            TargetState::Up => {
                if self.notified == Some(TargetState::Up) {
                    return None;
                }
                if self.stabilize.is_zero() {
                    self.pending = None;
                    self.notified = Some(TargetState::Up);
                    return Some(notification);
                }
                let deadline = notification.at + self.stabilize;
                // The window runs from the first `up` of the episode; a later one
                // neither shortens nor restarts it.
                if let Some((existing, _)) = &self.pending {
                    if *existing <= deadline {
                        return None;
                    }
                }
                self.pending = Some((deadline, notification));
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
    pub fn due(&mut self, now: Instant) -> Option<Notification> {
        match &self.pending {
            Some((deadline, _)) if *deadline <= now => {}
            _ => return None,
        }
        let (_, notification) = self.pending.take()?;
        self.notified = Some(TargetState::Up);
        Some(notification)
    }
}

/// One policy per target, keyed by [`Notification::key`].
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

    /// Feed an observed change and return the notification to send now, if any.
    pub fn observe(&mut self, notification: Notification) -> Option<Notification> {
        self.policies
            .entry(notification.key.clone())
            .or_insert_with(|| Policy::new(self.stabilize))
            .observe(notification)
    }

    /// The earliest instant at which [`Engine::due_all`] has something to return.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.policies
            .values()
            .filter_map(Policy::next_deadline)
            .min()
    }

    /// Every notification whose window has closed, in a deterministic order.
    pub fn due_all(&mut self, now: Instant) -> Vec<Notification> {
        let mut due: Vec<Notification> = self
            .policies
            .values_mut()
            .filter_map(|policy| policy.due(now))
            .collect();
        due.sort_by(|left, right| {
            left.occurred_at_ms
                .cmp(&right.occurred_at_ms)
                .then_with(|| left.key.cmp(&right.key))
        });
        due
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(
        key: &str,
        state: TargetState,
        cause: Option<&str>,
        at: Instant,
    ) -> Notification {
        Notification {
            target: key.to_string(),
            key: key.to_string(),
            state,
            cause: cause.map(str::to_string),
            occurred_at_ms: 0,
            at,
        }
    }

    fn window() -> Duration {
        Duration::from_secs(60)
    }

    #[test]
    fn a_disconnection_is_reported_at_once() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        let sent = policy
            .observe(notification(
                "icmp://1.1.1.1",
                TargetState::Down,
                Some("ICMP.NO_REPLY — no reply"),
                now,
            ))
            .expect("a disconnection needs no window");
        assert_eq!(sent.state, TargetState::Down);
        assert_eq!(policy.next_deadline(), None);
    }

    #[test]
    fn a_recovery_is_reported_only_after_the_window() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        assert!(policy
            .observe(notification("icmp://1.1.1.1", TargetState::Down, None, now))
            .is_some());

        let up_at = now + Duration::from_secs(5);
        assert!(policy
            .observe(notification("icmp://1.1.1.1", TargetState::Up, None, up_at))
            .is_none());
        assert_eq!(policy.next_deadline(), Some(up_at + window()));

        assert!(policy
            .due(up_at + window() - Duration::from_millis(1))
            .is_none());
        let sent = policy.due(up_at + window()).expect("the window closed");
        assert_eq!(sent.state, TargetState::Up);
        assert_eq!(policy.next_deadline(), None);
    }

    #[test]
    fn a_disconnection_inside_the_window_cancels_the_recovery() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        policy.observe(notification("icmp://1.1.1.1", TargetState::Down, None, now));

        let up_at = now + Duration::from_secs(5);
        policy.observe(notification("icmp://1.1.1.1", TargetState::Up, None, up_at));
        assert!(
            policy
                .observe(notification(
                    "icmp://1.1.1.1",
                    TargetState::Down,
                    None,
                    up_at + Duration::from_secs(10)
                ))
                .is_none(),
            "the operator already knows the target is down"
        );
        assert!(policy.due(up_at + window()).is_none());
    }

    #[test]
    fn a_flap_reports_one_disconnection_per_incident() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        assert!(policy
            .observe(notification("icmp://1.1.1.1", TargetState::Down, None, now))
            .is_some());
        assert!(policy
            .observe(notification("icmp://1.1.1.1", TargetState::Up, None, now))
            .is_none());
        assert!(
            policy
                .observe(notification("icmp://1.1.1.1", TargetState::Down, None, now))
                .is_none(),
            "a second down in the same incident is silent"
        );
    }

    #[test]
    fn a_disconnection_after_a_notified_recovery_is_reported_again() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        policy.observe(notification("icmp://1.1.1.1", TargetState::Down, None, now));
        let up_at = now + Duration::from_secs(10);
        policy.observe(notification("icmp://1.1.1.1", TargetState::Up, None, up_at));
        let recovered = policy
            .due(up_at + window())
            .expect("the recovery is reported");
        assert_eq!(recovered.state, TargetState::Up);

        let again = policy
            .observe(notification(
                "icmp://1.1.1.1",
                TargetState::Down,
                None,
                up_at + window(),
            ))
            .expect("a new incident is a new message");
        assert_eq!(again.state, TargetState::Down);
    }

    #[test]
    fn a_zero_window_reports_a_recovery_at_once() {
        let now = Instant::now();
        let mut policy = Policy::new(Duration::ZERO);
        policy.observe(notification("icmp://1.1.1.1", TargetState::Down, None, now));

        let sent = policy
            .observe(notification("icmp://1.1.1.1", TargetState::Up, None, now))
            .expect("no window means no waiting");
        assert_eq!(sent.state, TargetState::Up);
        assert!(policy
            .observe(notification("icmp://1.1.1.1", TargetState::Up, None, now))
            .is_none());
    }

    #[test]
    fn the_first_verdict_reports_the_observed_state() {
        let now = Instant::now();
        let mut policy = Policy::new(window());
        assert!(policy
            .observe(notification("icmp://1.1.1.1", TargetState::Down, None, now))
            .is_some());
        assert!(policy
            .observe(notification("icmp://1.1.1.1", TargetState::Up, None, now))
            .is_none());

        let mut fresh = Policy::new(window());
        assert!(fresh
            .observe(notification("icmp://1.1.1.1", TargetState::Up, None, now))
            .is_none());
        assert_eq!(fresh.next_deadline(), Some(now + window()));
    }

    #[test]
    fn the_message_carries_the_cause_and_the_instant() {
        let now = Instant::now();
        let mut notification = notification(
            "icmp://1.1.1.1",
            TargetState::Down,
            Some("ICMP.NO_REPLY — no reply within 2000 ms"),
            now,
        );
        notification.target = "cloudflare-dns".to_string();
        notification.occurred_at_ms = 1_764_000_000_000;

        assert_eq!(
            notification.text(),
            "DOWN cloudflare-dns (icmp://1.1.1.1)\n\
             Cause: ICMP.NO_REPLY — no reply within 2000 ms\n\
             2025-11-24T16:00:00.000Z"
        );

        let up = Notification {
            target: "icmp://1.1.1.1".to_string(),
            key: "icmp://1.1.1.1".to_string(),
            state: TargetState::Up,
            cause: None,
            occurred_at_ms: 0,
            at: now,
        };
        assert_eq!(up.text(), "UP icmp://1.1.1.1\n1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn the_engine_keeps_one_policy_per_key() {
        let now = Instant::now();
        let mut engine = Engine::new(window());
        assert!(engine
            .observe(notification("icmp://1.1.1.1", TargetState::Down, None, now))
            .is_some());
        assert!(engine
            .observe(notification("icmp://1.1.1.1", TargetState::Down, None, now))
            .is_none());

        assert!(engine
            .observe(notification(
                "tcp://1.1.1.1:443",
                TargetState::Down,
                None,
                now
            ))
            .is_some());
        assert_eq!(engine.next_deadline(), None);
        assert!(engine.due_all(now).is_empty());
    }

    #[test]
    fn due_all_returns_one_recovery_per_target() {
        let now = Instant::now();
        let mut engine = Engine::new(window());
        for key in ["icmp://1.1.1.1", "icmp://1.0.0.1"] {
            engine.observe(notification(key, TargetState::Down, None, now));
            engine.observe(notification(key, TargetState::Up, None, now));
        }

        assert_eq!(engine.next_deadline(), Some(now + window()));
        assert!(engine
            .due_all(now + window() - Duration::from_secs(1))
            .is_empty());

        let due = engine.due_all(now + window());
        assert_eq!(due.len(), 2);
        assert!(due
            .iter()
            .all(|notification| notification.state == TargetState::Up));
        assert!(
            engine.due_all(now + window()).is_empty(),
            "one message per window"
        );
    }
}
