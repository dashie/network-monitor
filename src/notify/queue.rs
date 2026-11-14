//! The bounded hand-off between the probe loops and the delivery thread.
//!
//! A probe never blocks on a notification: it pushes the message and moves on. When the
//! operator is not draining the queue — a Telegram outage, a slow link — the oldest
//! message is dropped so the newest state change is the one that survives.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use super::Notification;

/// How many messages may wait for delivery before the oldest is dropped.
pub const CAPACITY: usize = 32;

#[derive(Debug, Default)]
struct State {
    items: VecDeque<Notification>,
    closed: bool,
}

/// A multi-producer, single-consumer queue.
#[derive(Debug, Default)]
pub struct Queue {
    state: Mutex<State>,
    ready: Condvar,
}

impl Queue {
    pub fn new() -> Self {
        Queue::default()
    }

    /// Append a message, dropping the oldest when the queue is full.
    ///
    /// Returns `true` when a message was dropped to make room.
    pub fn push(&self, notification: Notification) -> bool {
        let mut state = self.lock();
        let dropped = state.items.len() >= CAPACITY;
        if dropped {
            state.items.pop_front();
        }
        state.items.push_back(notification);
        drop(state);
        self.ready.notify_one();
        dropped
    }

    /// Take the oldest message, waiting at most `wait` for one to arrive.
    pub fn pop_timeout(&self, wait: Duration) -> Option<Notification> {
        let mut state = self.lock();
        if state.items.is_empty() && !state.closed {
            let (next, _) = self
                .ready
                .wait_timeout(state, wait)
                .expect("the queue lock is not poisoned");
            state = next;
        }
        state.items.pop_front()
    }

    /// Stop waiting and let the consumer leave.
    pub fn close(&self) {
        self.lock().closed = true;
        self.ready.notify_all();
    }

    pub fn is_closed(&self) -> bool {
        self.lock().closed
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().expect("the queue lock is not poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TargetState;
    use std::time::Instant;

    fn notification(index: usize) -> Notification {
        Notification {
            target: format!("target-{index}"),
            key: format!("icmp://10.0.0.{index}"),
            state: TargetState::Down,
            cause: None,
            occurred_at_ms: index as i64,
            at: Instant::now(),
        }
    }

    #[test]
    fn messages_come_out_in_order() {
        let queue = Queue::new();
        assert!(!queue.push(notification(1)));
        assert!(!queue.push(notification(2)));
        assert_eq!(
            queue.pop_timeout(Duration::ZERO).unwrap().key,
            "icmp://10.0.0.1"
        );
        assert_eq!(
            queue.pop_timeout(Duration::ZERO).unwrap().key,
            "icmp://10.0.0.2"
        );
        assert!(queue.pop_timeout(Duration::ZERO).is_none());
    }

    #[test]
    fn a_full_queue_drops_the_oldest() {
        let queue = Queue::new();
        for index in 0..CAPACITY {
            assert!(!queue.push(notification(index)), "no drop below capacity");
        }
        assert!(queue.push(notification(CAPACITY)), "the oldest was dropped");

        let mut seen = Vec::new();
        while let Some(item) = queue.pop_timeout(Duration::ZERO) {
            seen.push(item.occurred_at_ms);
        }
        assert_eq!(seen.len(), CAPACITY);
        assert_eq!(seen[0], 1, "the first message was dropped");
        assert_eq!(seen[CAPACITY - 1], CAPACITY as i64, "the newest survived");
    }

    #[test]
    fn a_closed_queue_stops_the_consumer() {
        let queue = Queue::new();
        queue.close();
        assert!(queue.is_closed());
        assert!(queue.pop_timeout(Duration::from_millis(10)).is_none());
    }
}
