//! Deterministic channel and notifier for the tests.
//!
//! They replace the network and the delivery thread, so routing and delivery are tested
//! without a socket (INV-3).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use super::channel::{Channel, Payload, SendError};
use super::message::{Kind, Message};
use super::Notifier;

/// Replays a scripted sequence of outcomes and records every payload it was asked to send.
///
/// Cloning shares the state, so the test keeps a handle after the channel moved into the
/// delivery thread.
#[derive(Debug, Clone)]
pub struct FakeChannel {
    name: String,
    kind: Kind,
    fail_open: bool,
    fail_verify: bool,
    inner: Arc<Mutex<Inner>>,
}

#[derive(Debug, Default)]
struct Inner {
    payloads: usize,
    attempts: usize,
    opened: usize,
    verified: usize,
    closed: usize,
    bodies: Vec<String>,
    outcomes: VecDeque<Result<(), SendError>>,
}

impl FakeChannel {
    pub fn new(name: &str, kind: Kind) -> Self {
        FakeChannel {
            name: name.to_string(),
            kind,
            fail_open: false,
            fail_verify: false,
            inner: Arc::new(Mutex::new(Inner::default())),
        }
    }

    /// Succeeds on every attempt.
    pub fn always_ok(name: &str, kind: Kind) -> Self {
        FakeChannel::new(name, kind)
    }

    /// Refuses to open, so a start error can be exercised.
    pub fn failing_open(name: &str, kind: Kind) -> Self {
        let mut channel = FakeChannel::new(name, kind);
        channel.fail_open = true;
        channel
    }

    /// Refuses to verify, so a check failure can be exercised.
    pub fn failing_verify(name: &str, kind: Kind) -> Self {
        let mut channel = FakeChannel::new(name, kind);
        channel.fail_verify = true;
        channel
    }

    /// Fails with the scripted outcomes, oldest first, then succeeds forever.
    pub fn with_outcomes(name: &str, kind: Kind, outcomes: Vec<Result<(), SendError>>) -> Self {
        let channel = FakeChannel::new(name, kind);
        channel
            .inner
            .lock()
            .expect("the fake channel lock is not poisoned")
            .outcomes = outcomes.into();
        channel
    }

    /// Every body rendered, in order.
    pub fn sent(&self) -> Vec<String> {
        self.lock().bodies.clone()
    }

    /// How many payloads were rendered.
    pub fn payloads(&self) -> usize {
        self.lock().payloads
    }

    /// How many attempts the channel received.
    pub fn attempts(&self) -> usize {
        self.lock().attempts
    }

    pub fn opened(&self) -> usize {
        self.lock().opened
    }

    pub fn verified(&self) -> usize {
        self.lock().verified
    }

    pub fn closed(&self) -> usize {
        self.lock().closed
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .expect("the fake channel lock is not poisoned")
    }
}

impl Channel for FakeChannel {
    fn name(&self) -> &str {
        &self.name
    }

    fn kind(&self) -> Kind {
        self.kind
    }

    fn open(&mut self) -> Result<(), String> {
        if self.fail_open {
            return Err("the endpoint is unreachable".to_string());
        }
        self.lock().opened += 1;
        Ok(())
    }

    fn verify(&mut self) -> Result<(), String> {
        self.lock().verified += 1;
        if self.fail_verify {
            return Err("the endpoint rejected the token".to_string());
        }
        Ok(())
    }

    fn payload(&self, message: &Message) -> Option<Payload> {
        let mut inner = self.lock();
        inner.payloads += 1;
        let body = message.text();
        inner.bodies.push(body.clone());
        Some(Payload {
            body,
            media_type: "text/plain; charset=utf-8",
        })
    }

    fn send(&mut self, _payload: &Payload) -> Result<(), SendError> {
        let mut inner = self.lock();
        inner.attempts += 1;
        inner.outcomes.pop_front().unwrap_or(Ok(()))
    }

    fn close(&mut self) {
        self.lock().closed += 1;
    }
}

/// Records every message the monitor hands it.
#[derive(Debug, Clone, Default)]
pub struct FakeNotifier {
    messages: Arc<Mutex<Vec<Message>>>,
}

impl FakeNotifier {
    pub fn new() -> Self {
        FakeNotifier::default()
    }

    /// Every message received, oldest first.
    pub fn messages(&self) -> Vec<Message> {
        self.messages
            .lock()
            .expect("the fake notifier lock is not poisoned")
            .clone()
    }

    pub fn len(&self) -> usize {
        self.messages
            .lock()
            .expect("the fake notifier lock is not poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Notifier for FakeNotifier {
    fn send(&self, message: Message) {
        self.messages
            .lock()
            .expect("the fake notifier lock is not poisoned")
            .push(message);
    }
}
