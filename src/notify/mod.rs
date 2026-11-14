//! Notification of state changes: the decision ([`policy`]), the hand-off ([`queue`]),
//! and the delivery thread.
//!
//! The monitor depends only on the [`Notifier`] contract published here, so the
//! stabilisation rules and the transport are testable without a network.

pub mod fake;
pub mod policy;
pub mod queue;
pub mod telegram;

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::logging;
use crate::shutdown::{self, Shutdown};

use policy::Engine;
use queue::{Message, Queue};

pub use policy::Notification;

/// How often the delivery thread wakes to close a stabilisation window.
const IDLE_POLL: Duration = Duration::from_secs(1);

/// How many times one message is attempted before it is declared lost.
pub const ATTEMPTS: usize = 5;

/// What the monitor knows about notifications: a sole method for a state change, one
/// for the startup announcement, and no result.
pub trait Notifier: Send + Sync {
    fn notify(&self, notification: Notification);

    /// Hand the startup announcement to the channel, in its own words.
    fn announce(&self, text: String);
}

/// The notifier of a process without a `[telegram]` section.
#[derive(Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn notify(&self, _notification: Notification) {}

    fn announce(&self, _text: String) {}
}

/// Why a delivery attempt failed, and whether another attempt can help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendError {
    pub retryable: bool,
    pub reason: String,
}

/// One transport of one message.
pub trait Sender: Send {
    fn send(&mut self, text: &str) -> Result<(), SendError>;
}

/// The waits between two attempts: 1, 2, 4, and 8 seconds.
const BACKOFF: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// Attempt one message: five tries, waiting 1, 2, 4, and 8 seconds between them.
///
/// A non-retryable error and a shutdown request stop at once; every failure is one
/// `WARN` line, and giving up is one more.
pub fn deliver(sender: &mut dyn Sender, text: &str, shutdown: &Shutdown) -> Result<(), SendError> {
    deliver_with(sender, text, shutdown, &BACKOFF)
}

/// [`deliver`] with an explicit backoff, so a test needs no real wait.
pub(crate) fn deliver_with(
    sender: &mut dyn Sender,
    text: &str,
    shutdown: &Shutdown,
    waits: &[Duration],
) -> Result<(), SendError> {
    let mut last = None;

    for attempt in 1..=ATTEMPTS {
        match sender.send(text) {
            Ok(()) => return Ok(()),
            Err(error) => {
                logging::warn(
                    None,
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
    logging::warn(None, &format!("notification lost: {}", error.reason));
    Err(error)
}

/// The notifier the monitor holds: it enqueues and never blocks.
#[derive(Debug)]
pub struct ChannelNotifier {
    queue: Arc<Queue>,
}

impl Notifier for ChannelNotifier {
    fn notify(&self, notification: Notification) {
        report_drop(self.queue.push(notification));
    }

    fn announce(&self, text: String) {
        report_drop(self.queue.push_announcement(text));
    }
}

fn report_drop(dropped: bool) {
    if dropped {
        logging::warn(
            None,
            "notification queue full; the oldest message was dropped",
        );
    }
}

/// The delivery thread and the notifier that feeds it.
pub struct Handle {
    pub notifier: Arc<dyn Notifier>,
    queue: Arc<Queue>,
    stop: Shutdown,
    thread: Option<JoinHandle<()>>,
}

impl Handle {
    /// Stop the thread and wait for it; pending messages are dropped.
    pub fn shutdown(mut self) {
        self.queue.close();
        self.stop.request();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the delivery thread for the configured channel.
pub fn start(settings: &crate::config::Telegram, shutdown: &Shutdown) -> Handle {
    start_with(
        settings,
        Box::new(telegram::Telegram::new(settings)),
        shutdown,
    )
}

/// Start the delivery thread over a caller-supplied sender.
///
/// This is the seam the tests use.
pub fn start_with(
    settings: &crate::config::Telegram,
    sender: Box<dyn Sender>,
    shutdown: &Shutdown,
) -> Handle {
    let queue = Arc::new(Queue::new());
    let notifier: Arc<dyn Notifier> = Arc::new(ChannelNotifier {
        queue: Arc::clone(&queue),
    });

    let stop = Shutdown::new();
    let thread_stop = stop.clone();
    let thread_queue = Arc::clone(&queue);
    let process = shutdown.clone();
    let stabilize = settings.stabilize;

    let thread =
        thread::spawn(move || run(stabilize, sender, thread_queue, &thread_stop, &process));

    Handle {
        notifier,
        queue,
        stop,
        thread: Some(thread),
    }
}

/// The delivery loop: apply the policy to what arrives, and close the windows on time.
fn run(
    stabilize: Duration,
    mut sender: Box<dyn Sender>,
    queue: Arc<Queue>,
    stop: &Shutdown,
    shutdown: &Shutdown,
) {
    let mut engine = Engine::new(stabilize);

    while !stop.requested() && !shutdown::stopped(shutdown) {
        let wait = engine
            .next_deadline()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .map_or(IDLE_POLL, |remaining| remaining.min(IDLE_POLL));

        match queue.pop_timeout(wait) {
            Some(Message::Notification(notification)) => {
                if let Some(due) = engine.observe(notification) {
                    send(&mut *sender, &due, shutdown);
                }
            }
            Some(Message::Announcement(text)) => announce(&mut *sender, &text, shutdown),
            None => {
                if queue.is_closed() {
                    break;
                }
                for due in engine.due_all(Instant::now()) {
                    send(&mut *sender, &due, shutdown);
                }
            }
        }
    }
}

/// Deliver one notification and report the outcome.
fn send(sender: &mut dyn Sender, notification: &Notification, shutdown: &Shutdown) {
    if deliver(sender, &notification.text(), shutdown).is_ok() {
        logging::info(Some(&notification.target), "notification sent");
    }
}

/// Deliver the startup announcement and report the outcome.
fn announce(sender: &mut dyn Sender, text: &str, shutdown: &Shutdown) {
    if deliver(sender, text, shutdown).is_ok() {
        logging::info(None, "startup announcement sent");
    }
}
