//! Deterministic sender and notifier for the tests.
//!
//! They replace the network and the delivery thread, so the notification rules are
//! tested without a socket (INV-3).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::{Notification, Notifier, SendError, Sender};

/// Replays a scripted sequence of outcomes and records every text it was asked to send.
///
/// Cloning shares the state, so the test keeps a handle after the sender moved into the
/// delivery thread.
#[derive(Debug, Clone, Default)]
pub struct FakeSender {
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    outcomes: VecDeque<Result<(), SendError>>,
    sent: Vec<String>,
}

impl FakeSender {
    /// Fails with the scripted outcomes, oldest first, then succeeds forever.
    pub fn new(outcomes: Vec<Result<(), SendError>>) -> Self {
        FakeSender {
            inner: Arc::new(Mutex::new(Inner {
                outcomes: outcomes.into(),
                sent: Vec::new(),
            })),
        }
    }

    /// Succeeds on every attempt.
    pub fn always_ok() -> Self {
        FakeSender::default()
    }

    /// Every text offered to the sender, oldest first, attempts included.
    pub fn sent(&self) -> Vec<String> {
        self.inner
            .lock()
            .expect("the fake sender lock is not poisoned")
            .sent
            .clone()
    }

    /// How many attempts the sender received.
    pub fn attempts(&self) -> usize {
        self.inner
            .lock()
            .expect("the fake sender lock is not poisoned")
            .sent
            .len()
    }
}

impl Sender for FakeSender {
    fn send(&mut self, text: &str) -> Result<(), SendError> {
        let mut inner = self
            .inner
            .lock()
            .expect("the fake sender lock is not poisoned");
        inner.sent.push(text.to_string());
        inner.outcomes.pop_front().unwrap_or(Ok(()))
    }
}

/// Records every notification the monitor hands it.
#[derive(Debug, Clone, Default)]
pub struct FakeNotifier {
    notifications: Arc<Mutex<Vec<Notification>>>,
}

impl FakeNotifier {
    pub fn new() -> Self {
        FakeNotifier::default()
    }

    /// Every notification received, oldest first.
    pub fn notifications(&self) -> Vec<Notification> {
        self.notifications
            .lock()
            .expect("the fake notifier lock is not poisoned")
            .clone()
    }

    pub fn len(&self) -> usize {
        self.notifications
            .lock()
            .expect("the fake notifier lock is not poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Notifier for FakeNotifier {
    fn notify(&self, notification: Notification) {
        self.notifications
            .lock()
            .expect("the fake notifier lock is not poisoned")
            .push(notification);
    }
}
