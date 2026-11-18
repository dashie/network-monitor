//! Notification: the message, the decision, the routing, and the delivery threads.
//!
//! The monitor depends only on the [`Notifier`] contract published here, so the
//! stabilisation rules, the routing and the transports are testable without a network.
//!
//! One channel is declared once and enabled by name; one queue, thread and policy run
//! per declared channel, so a slow or failing transport delays nobody else.

pub mod channel;
pub mod fake;
pub mod logfile;
pub mod message;
pub mod policy;
pub mod queue;
pub mod telegram;
pub mod webhook;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use thiserror::Error;

use crate::config::{ChannelSettings, NotificationConfig};
use crate::logging::{self, Level};
use crate::shutdown::{self, Shutdown};
use crate::time::now_ms;

use channel::{deliver, deliver_once, Channel, Routing};
use policy::Engine;
use queue::Queue;

pub use channel::{Payload, SendError};
pub use message::Message;
pub use message::{Cause, Event, GapSummary, Kind, TargetRef};

/// How often the delivery thread wakes to close a stabilisation window.
const IDLE_POLL: Duration = Duration::from_secs(1);

/// What the monitor knows about notifications: one method, no result, never blocking.
pub trait Notifier: Send + Sync {
    fn send(&self, message: Message);
}

/// The notifier of a process without notification channels.
#[derive(Debug, Default)]
pub struct NoopNotifier;

impl Notifier for NoopNotifier {
    fn send(&self, _message: Message) {}
}

/// Why a channel could not start: the process does not begin without it.
#[derive(Debug, Error)]
#[error("notification channel '{channel}' cannot start: {reason}")]
pub struct StartError {
    pub channel: String,
    pub reason: String,
}

/// The notifier the monitor holds: it routes and enqueues, and never blocks.
#[derive(Debug)]
pub struct FanoutNotifier {
    queues: Vec<Arc<Queue>>,
    routing: Routing,
}

impl Notifier for FanoutNotifier {
    fn send(&self, message: Message) {
        for &index in self.routing.channels_for(&message) {
            if let Some(queue) = self.queues.get(index) {
                if queue.push(message.clone()) {
                    logging::notification(
                        Level::Warn,
                        message.target_key(),
                        "notification queue full; the oldest message was dropped",
                    );
                }
            }
        }
    }
}

struct Runner {
    channel: Arc<Mutex<Box<dyn Channel>>>,
    queue: Arc<Queue>,
    stop: Shutdown,
    thread: Option<JoinHandle<()>>,
}

/// The delivery threads and the notifier that feeds them.
pub struct Handle {
    pub notifier: Arc<dyn Notifier>,
    runners: Vec<Runner>,
    routing: Routing,
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Handle")
            .field("runners", &self.runners.len())
            .finish()
    }
}

impl Handle {
    /// Deliver the farewell once to the channels the root list enables, then stop.
    ///
    /// Pending incidents are dropped: the process is leaving, and a queue must not hold
    /// it. The farewell itself is attempted once, under the channel lock.
    pub fn shutdown(mut self, farewell: Option<Message>) {
        if let Some(farewell) = farewell {
            for &index in self.routing.global() {
                if let Some(runner) = self.runners.get(index) {
                    let mut channel = runner
                        .channel
                        .lock()
                        .expect("the channel lock is not poisoned");
                    let _ = deliver_once(&mut **channel, &farewell);
                }
            }
        }

        for runner in &mut self.runners {
            runner.queue.close();
            runner.stop.request();
        }
        for runner in &mut self.runners {
            if let Some(thread) = runner.thread.take() {
                let _ = thread.join();
            }
            let mut channel = runner
                .channel
                .lock()
                .expect("the channel lock is not poisoned");
            channel.close();
        }
    }
}

/// Start one delivery thread per declared channel.
pub fn start(config: &NotificationConfig, shutdown: &Shutdown) -> Result<Handle, StartError> {
    start_with(config, build, shutdown)
}

/// Start the delivery threads over caller-supplied channels.
///
/// This is the seam the tests use.
pub fn start_with<F>(
    config: &NotificationConfig,
    build: F,
    shutdown: &Shutdown,
) -> Result<Handle, StartError>
where
    F: Fn(&ChannelSettings) -> Box<dyn Channel>,
{
    let mut channels: Vec<Box<dyn Channel>> = config.channels.iter().map(build).collect();

    for index in 0..channels.len() {
        if let Err(reason) = channels[index].open() {
            let name = channels[index].name().to_string();
            for opened in &mut channels[..index] {
                opened.close();
            }
            return Err(StartError {
                channel: name,
                reason,
            });
        }
    }

    let indices: HashMap<&str, usize> = config
        .channels
        .iter()
        .enumerate()
        .map(|(index, settings)| (settings.name(), index))
        .collect();

    let mut by_target: HashMap<String, Vec<usize>> = HashMap::new();
    for (target, names) in &config.by_target {
        by_target.insert(target.clone(), indices_for(&indices, names));
    }
    let routing = Routing::new(indices_for(&indices, &config.global), by_target);

    let mut runners = Vec::with_capacity(channels.len());
    let mut queues = Vec::with_capacity(channels.len());
    for (index, channel) in channels.into_iter().enumerate() {
        let queue = Arc::new(Queue::new());
        let name = config.channels[index].name().to_string();
        let kind = config.channels[index].kind();
        let stabilize = config.channels[index].stabilize();
        // The channel announces itself on itself, so a target-only channel still
        // states that it started.
        queue.push(Message::channel_started(name, kind, now_ms()));

        let channel = Arc::new(Mutex::new(channel));
        let stop = Shutdown::new();
        let thread_stop = stop.clone();
        let thread_queue = Arc::clone(&queue);
        let thread_channel = Arc::clone(&channel);
        let process = shutdown.clone();
        let thread = thread::spawn(move || {
            run(
                thread_channel,
                thread_queue,
                &thread_stop,
                &process,
                stabilize,
            )
        });

        queues.push(Arc::clone(&queue));
        runners.push(Runner {
            channel,
            queue,
            stop,
            thread: Some(thread),
        });
    }

    let notifier: Arc<dyn Notifier> = Arc::new(FanoutNotifier {
        queues,
        routing: routing.clone(),
    });
    Ok(Handle {
        notifier,
        runners,
        routing,
    })
}

/// Prove every declared channel can deliver, without delivering anything.
///
/// The channels are built and verified in declaration order and stop at the first
/// failure, named exactly as a start failure names it. Verification opens a connection
/// or a file and closes it again; it never sends a message.
pub fn verify(config: &NotificationConfig) -> Result<(), StartError> {
    verify_with(config, build)
}

/// [`verify`] over caller-supplied channels: the seam the tests use.
pub fn verify_with<F>(config: &NotificationConfig, build: F) -> Result<(), StartError>
where
    F: Fn(&ChannelSettings) -> Box<dyn Channel>,
{
    let mut channels: Vec<Box<dyn Channel>> = config.channels.iter().map(build).collect();

    for index in 0..channels.len() {
        if let Err(reason) = channels[index].verify() {
            let name = channels[index].name().to_string();
            for opened in &mut channels[..index] {
                opened.close();
            }
            return Err(StartError {
                channel: name,
                reason,
            });
        }
    }

    for channel in &mut channels {
        channel.close();
    }
    Ok(())
}

fn build(settings: &ChannelSettings) -> Box<dyn Channel> {
    match settings {
        ChannelSettings::Telegram(settings) => Box::new(telegram::Telegram::new(settings)),
        ChannelSettings::Webhook(settings) => Box::new(webhook::Webhook::new(settings)),
        ChannelSettings::LogFile(settings) => Box::new(logfile::LogFile::new(settings)),
    }
}

fn indices_for(indices: &HashMap<&str, usize>, names: &[String]) -> Vec<usize> {
    names
        .iter()
        .filter_map(|name| indices.get(name.as_str()).copied())
        .collect()
}

/// The delivery loop: apply the policy to target events and deliver everything else.
fn run(
    channel: Arc<Mutex<Box<dyn Channel>>>,
    queue: Arc<Queue>,
    stop: &Shutdown,
    shutdown: &Shutdown,
    stabilize: Duration,
) {
    let mut engine = Engine::new(stabilize);

    while !stop.requested() && !shutdown::stopped(shutdown) {
        let wait = engine
            .next_deadline()
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
            .map_or(IDLE_POLL, |remaining| remaining.min(IDLE_POLL));

        match queue.pop_timeout(wait) {
            Some(message) => match message.event {
                Event::TargetDown { .. } | Event::TargetUp { .. } => {
                    if let Some(due) = engine.observe(message) {
                        send(&channel, &due, shutdown);
                    }
                }
                _ => send(&channel, &message, shutdown),
            },
            None => {
                if queue.is_closed() {
                    break;
                }
                for due in engine.due_all(Instant::now()) {
                    send(&channel, &due, shutdown);
                }
            }
        }
    }
}

fn send(channel: &Arc<Mutex<Box<dyn Channel>>>, message: &Message, shutdown: &Shutdown) {
    let mut channel = channel.lock().expect("the channel lock is not poisoned");
    if deliver(&mut **channel, message, shutdown).is_ok() {
        logging::notification(
            Level::Info,
            message.target_key(),
            &format!("{} sent", message.event_name()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{LogFile, LogFormat, NotificationConfig};
    use crate::logging::Level;
    use crate::notify::fake::FakeChannel;
    use crate::notify::message::{Cause, TargetRef};
    use std::path::PathBuf;

    fn settings(name: &str) -> ChannelSettings {
        ChannelSettings::LogFile(LogFile {
            name: name.to_string(),
            filename: PathBuf::from(format!("/tmp/{name}.log")),
            logger: "notification".to_string(),
            level: Level::Info,
            format: LogFormat::Text,
            stabilize: Duration::from_millis(50),
            max_bytes: 0,
            max_files: 5,
        })
    }

    fn config(global: Vec<&str>, by_target: Vec<(&str, Vec<&str>)>) -> NotificationConfig {
        NotificationConfig {
            channels: vec![settings("a"), settings("b")],
            global: global.into_iter().map(str::to_string).collect(),
            by_target: by_target
                .into_iter()
                .map(|(key, names)| {
                    (
                        key.to_string(),
                        names.into_iter().map(str::to_string).collect(),
                    )
                })
                .collect(),
        }
    }

    fn down(key: &str) -> Message {
        let mut message = Message::target_down(
            TargetRef::new(key, None),
            Cause::new("ICMP.NO_REPLY", "no reply"),
            0,
        );
        message.at = Instant::now();
        message
    }

    fn wait_for(condition: impl Fn() -> bool, what: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !condition() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn start_two(
        config: &NotificationConfig,
        shutdown: &Shutdown,
        a: &FakeChannel,
        b: &FakeChannel,
    ) -> Handle {
        start_with(
            config,
            |settings| match settings.name() {
                "a" => Box::new(a.clone()),
                _ => Box::new(b.clone()),
            },
            shutdown,
        )
        .expect("both channels start")
    }

    #[test]
    fn a_channel_that_cannot_verify_is_named_and_the_earlier_channels_are_closed() {
        let a = FakeChannel::always_ok("a", Kind::LogFile);
        let b = FakeChannel::failing_verify("b", Kind::LogFile);

        let error = verify_with(&config(vec!["a", "b"], vec![]), |settings| {
            match settings.name() {
                "a" => Box::new(a.clone()),
                _ => Box::new(b.clone()),
            }
        })
        .unwrap_err();

        assert_eq!(error.channel, "b");
        assert!(error.to_string().contains("rejected the token"));
        assert_eq!(a.verified(), 1, "the earlier channel was verified");
        assert_eq!(a.closed(), 1, "and then closed");
        assert_eq!(b.closed(), 0, "the failing channel opened nothing to close");
    }

    #[test]
    fn verifying_every_channel_sends_nothing_and_closes_them_all() {
        let a = FakeChannel::always_ok("a", Kind::LogFile);
        let b = FakeChannel::always_ok("b", Kind::Webhook);

        verify_with(
            &config(vec!["a"], vec![("icmp://1.1.1.1", vec!["b"])]),
            |settings| match settings.name() {
                "a" => Box::new(a.clone()),
                _ => Box::new(b.clone()),
            },
        )
        .expect("both channels verify");

        assert_eq!(a.verified(), 1);
        assert_eq!(b.verified(), 1);
        assert_eq!(a.closed(), 1);
        assert_eq!(b.closed(), 1);
        assert!(a.sent().is_empty(), "verification delivers nothing");
        assert!(b.sent().is_empty(), "verification delivers nothing");
    }

    #[test]
    fn a_target_event_reaches_only_the_channels_its_list_names() {
        let a = FakeChannel::always_ok("a", Kind::LogFile);
        let b = FakeChannel::always_ok("b", Kind::LogFile);
        let shutdown = Shutdown::new();
        let handle = start_two(
            &config(vec![], vec![("icmp://1.1.1.1", vec!["a"])]),
            &shutdown,
            &a,
            &b,
        );

        handle.notifier.send(down("icmp://1.1.1.1"));
        wait_for(
            || a.sent().iter().any(|body| body.starts_with("DOWN")),
            "the down",
        );

        assert!(a.sent().iter().any(|body| body.starts_with("DOWN")));
        assert!(
            !b.sent().iter().any(|body| body.starts_with("DOWN")),
            "b was not enabled for this target: {:?}",
            b.sent()
        );
        handle.shutdown(None);
    }

    #[test]
    fn a_global_event_reaches_the_root_list() {
        let a = FakeChannel::always_ok("a", Kind::LogFile);
        let b = FakeChannel::always_ok("b", Kind::LogFile);
        let shutdown = Shutdown::new();
        let handle = start_two(
            &config(vec!["a"], vec![("icmp://1.1.1.1", vec![])]),
            &shutdown,
            &a,
            &b,
        );

        handle
            .notifier
            .send(Message::config_reload(true, "/etc/test.toml", 0));
        wait_for(
            || {
                a.sent()
                    .iter()
                    .any(|body| body.starts_with("CONFIG applied"))
            },
            "the reload",
        );
        assert!(!b.sent().iter().any(|body| body.starts_with("CONFIG")));
        handle.shutdown(None);
    }

    #[test]
    fn every_channel_states_that_it_started() {
        let a = FakeChannel::always_ok("a", Kind::LogFile);
        let b = FakeChannel::always_ok("b", Kind::LogFile);
        let shutdown = Shutdown::new();
        let handle = start_two(
            &config(vec![], vec![("icmp://1.1.1.1", vec!["a"])]),
            &shutdown,
            &a,
            &b,
        );

        wait_for(
            || {
                a.sent().iter().any(|body| body.contains("CHANNEL a"))
                    && b.sent().iter().any(|body| body.contains("CHANNEL b"))
            },
            "the start lines",
        );
        assert_eq!(a.opened(), 1);
        assert_eq!(b.opened(), 1);
        handle.shutdown(None);
    }

    #[test]
    fn a_channel_that_cannot_open_stops_the_start() {
        let channel = FakeChannel::failing_open("a", Kind::LogFile);
        let shutdown = Shutdown::new();
        let error = start_with(
            &config(vec!["a"], vec![]),
            |_| Box::new(channel.clone()),
            &shutdown,
        )
        .unwrap_err();

        assert_eq!(error.channel, "a");
        assert!(error.to_string().contains("the endpoint is unreachable"));
    }

    #[test]
    fn the_farewell_reaches_only_the_root_channels_once() {
        let a = FakeChannel::always_ok("a", Kind::LogFile);
        let b = FakeChannel::always_ok("b", Kind::LogFile);
        let shutdown = Shutdown::new();
        let handle = start_two(
            &config(vec!["a"], vec![("icmp://1.1.1.1", vec!["a"])]),
            &shutdown,
            &a,
            &b,
        );

        handle.shutdown(Some(Message::shutdown(0)));
        assert_eq!(
            a.sent()
                .iter()
                .filter(|body| body.starts_with("STOPPED"))
                .count(),
            1
        );
        assert!(!b.sent().iter().any(|body| body.starts_with("STOPPED")));
        assert_eq!(a.closed(), 1);
        assert_eq!(b.closed(), 1);
    }
}
