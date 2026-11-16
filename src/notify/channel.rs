//! The channel contract: what a transport must provide, and how a message reaches one.
//!
//! A channel renders one [`Message`] in its own payload, sends it, and is opened and
//! closed once. Delivery retries live here, so every transport fails the same way.

use std::collections::HashMap;
use std::time::Duration;

use crate::logging::{self, Level};
use crate::shutdown::{self, Shutdown};

use super::message::{Event, Kind, Message};

/// How many times one message is attempted before it is declared lost.
pub const ATTEMPTS: usize = 5;

/// How many messages may wait for delivery before the oldest is dropped.
pub const CAPACITY: usize = 32;

/// The waits between two attempts: 1, 2, 4, and 8 seconds.
pub const BACKOFF: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// Why a delivery attempt failed, and whether another attempt can help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendError {
    pub retryable: bool,
    pub reason: String,
}

/// What a channel hands to its transport: one rendered body and its media type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    pub body: String,
    pub media_type: &'static str,
}

/// One transport of one message.
pub trait Channel: Send {
    /// The name the configuration gave this channel.
    fn name(&self) -> &str;

    fn kind(&self) -> Kind;

    /// Prepare the transport; a failure stops the process at startup.
    fn open(&mut self) -> Result<(), String>;

    /// Render the message, or `None` when this channel is not interested in it.
    fn payload(&self, message: &Message) -> Option<Payload>;

    fn send(&mut self, payload: &Payload) -> Result<(), SendError>;

    /// Flush and release what `open` acquired.
    fn close(&mut self);
}

/// Which channel indices a message reaches, in declaration order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Routing {
    global: Vec<usize>,
    by_target: HashMap<String, Vec<usize>>,
}

impl Routing {
    pub fn new(global: Vec<usize>, by_target: HashMap<String, Vec<usize>>) -> Self {
        Routing { global, by_target }
    }

    /// The channels a message reaches: the root list for a global event, the
    /// effective list of the target for a target event.
    pub fn channels_for(&self, message: &Message) -> &[usize] {
        match &message.event {
            Event::TargetDown { .. } | Event::TargetUp { .. } => {
                match message.target_key().and_then(|key| self.by_target.get(key)) {
                    Some(indices) => indices.as_slice(),
                    None => &[],
                }
            }
            _ => &self.global,
        }
    }

    /// The channels a global event reaches.
    pub fn global(&self) -> &[usize] {
        &self.global
    }
}

/// Attempt one message five times, waiting 1, 2, 4, and 8 seconds between them.
///
/// The payload is rendered once, so a retry cannot change what is sent. A non-retryable
/// failure and a shutdown stop at once; every failure is one `WARN` line, and giving up
/// is one more.
pub fn deliver(
    channel: &mut dyn Channel,
    message: &Message,
    shutdown: &Shutdown,
) -> Result<(), SendError> {
    deliver_with(channel, message, shutdown, &BACKOFF)
}

/// [`deliver`] with an explicit backoff, so a test needs no real wait.
pub(crate) fn deliver_with(
    channel: &mut dyn Channel,
    message: &Message,
    shutdown: &Shutdown,
    waits: &[Duration],
) -> Result<(), SendError> {
    let Some(payload) = channel.payload(message) else {
        return Ok(());
    };

    let mut last = None;
    for attempt in 1..=ATTEMPTS {
        match channel.send(&payload) {
            Ok(()) => return Ok(()),
            Err(error) => {
                logging::notification(
                    Level::Warn,
                    message.target_key(),
                    &format!(
                        "notification attempt {attempt}/{ATTEMPTS} failed: {}",
                        error.reason
                    ),
                );
                let stop = !error.retryable || attempt == ATTEMPTS || shutdown::stopped(shutdown);
                last = Some(error);
                if stop {
                    break;
                }
                if let Some(wait) = waits.get(attempt - 1).copied() {
                    shutdown::pause(wait, shutdown);
                }
            }
        }
    }

    let error = last.expect("a failed attempt leaves an error");
    logging::notification(
        Level::Warn,
        message.target_key(),
        &format!("notification lost: {}", error.reason),
    );
    Err(error)
}

/// Attempt one message once, for a farewell that must not hold the process.
pub fn deliver_once(channel: &mut dyn Channel, message: &Message) -> Result<(), SendError> {
    let Some(payload) = channel.payload(message) else {
        return Ok(());
    };
    match channel.send(&payload) {
        Ok(()) => Ok(()),
        Err(error) => {
            logging::notification(
                Level::Warn,
                message.target_key(),
                &format!("notification lost: {}", error.reason),
            );
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::notify::message::{Cause, TargetRef};

    #[derive(Debug, Default)]
    struct Sink {
        payloads: AtomicUsize,
        sends: usize,
        interested: bool,
        outcomes: VecDeque<Result<(), SendError>>,
    }

    impl Sink {
        fn with(outcomes: Vec<Result<(), SendError>>) -> Self {
            Sink {
                interested: true,
                outcomes: outcomes.into(),
                ..Sink::default()
            }
        }
    }

    impl Channel for Sink {
        fn name(&self) -> &str {
            "sink"
        }

        fn kind(&self) -> Kind {
            Kind::Webhook
        }

        fn open(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn payload(&self, message: &Message) -> Option<Payload> {
            self.payloads.fetch_add(1, Ordering::Relaxed);
            if self.interested {
                Some(Payload {
                    body: message.event_name().to_string(),
                    media_type: "text/plain; charset=utf-8",
                })
            } else {
                None
            }
        }

        fn send(&mut self, _payload: &Payload) -> Result<(), SendError> {
            self.sends += 1;
            self.outcomes.pop_front().unwrap_or(Ok(()))
        }

        fn close(&mut self) {}
    }

    fn down() -> Message {
        let mut message = Message::target_down(
            TargetRef::new("icmp://1.1.1.1", None),
            Cause::new("ICMP.NO_REPLY", "no reply"),
            0,
        );
        message.at = std::time::Instant::now();
        message
    }

    fn failure(retryable: bool) -> Result<(), SendError> {
        Err(SendError {
            retryable,
            reason: "temporary".to_string(),
        })
    }

    #[test]
    fn a_global_event_reaches_the_root_list() {
        let routing = Routing::new(vec![0, 2], HashMap::new());
        assert_eq!(routing.channels_for(&Message::shutdown(0)), &[0, 2][..]);
        assert_eq!(routing.global(), &[0, 2][..]);
    }

    #[test]
    fn a_target_event_reaches_its_own_list() {
        let mut by_target = HashMap::new();
        by_target.insert("icmp://1.1.1.1".to_string(), vec![1_usize]);
        let routing = Routing::new(vec![0], by_target);
        assert_eq!(routing.channels_for(&down()), &[1][..]);
    }

    #[test]
    fn a_target_without_a_list_reaches_nothing() {
        let routing = Routing::new(vec![0], HashMap::new());
        assert!(routing.channels_for(&down()).is_empty());
    }

    #[test]
    fn the_payload_is_rendered_once_for_all_attempts() {
        let sink = Sink::with(vec![failure(true), failure(true)]);
        let mut channel = sink;
        let shutdown = Shutdown::new();

        deliver_with(&mut channel, &down(), &shutdown, &[]).expect("the third attempt succeeds");
        assert_eq!(
            channel.payloads.load(Ordering::Relaxed),
            1,
            "one payload, three sends"
        );
        assert_eq!(channel.sends, 3);
    }

    #[test]
    fn a_non_retryable_failure_is_not_retried() {
        let mut channel = Sink::with(vec![failure(false)]);
        let shutdown = Shutdown::new();

        let error = deliver_with(&mut channel, &down(), &shutdown, &[]).unwrap_err();
        assert_eq!(error.reason, "temporary");
        assert_eq!(channel.sends, 1);
    }

    #[test]
    fn a_channel_without_a_payload_delivers_nothing() {
        let mut channel = Sink::default();
        let shutdown = Shutdown::new();

        deliver_with(&mut channel, &down(), &shutdown, &[]).expect("nothing to do");
        assert_eq!(channel.payloads.load(Ordering::Relaxed), 1);
        assert_eq!(channel.sends, 0);
    }

    #[test]
    fn five_failures_lose_the_message() {
        let outcomes = std::iter::repeat(failure(true)).take(ATTEMPTS).collect();
        let mut channel = Sink::with(outcomes);
        let shutdown = Shutdown::new();

        assert!(deliver_with(&mut channel, &down(), &shutdown, &[]).is_err());
        assert_eq!(channel.sends, ATTEMPTS);
    }

    #[test]
    fn a_farewell_is_attempted_once() {
        let mut channel = Sink::with(vec![failure(true)]);
        assert!(deliver_once(&mut channel, &Message::shutdown(0)).is_err());
        assert_eq!(channel.sends, 1);
    }
}
