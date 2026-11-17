//! The notifier end to end with a fake channel: no network and no real clock.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use network_monitor::config::{
    ChannelSettings, LogFile as LogFileSettings, LogFormat, NotificationConfig,
};
use network_monitor::logging::Level;
use network_monitor::notify::fake::FakeChannel;
use network_monitor::notify::message::{Cause, Kind, TargetRef};
use network_monitor::notify::{start_with, Message, SendError};
use network_monitor::shutdown::Shutdown;

const KEY: &str = "tcp://1.1.1.1:443";

fn settings(stabilize: Duration) -> ChannelSettings {
    ChannelSettings::LogFile(LogFileSettings {
        name: "channel".to_string(),
        filename: PathBuf::from("/tmp/notify-test.log"),
        logger: "notification".to_string(),
        level: Level::Info,
        format: LogFormat::Text,
        stabilize,
        max_bytes: 0,
        max_files: 5,
    })
}

fn config(stabilize: Duration, enabled: bool) -> NotificationConfig {
    let mut by_target = HashMap::new();
    by_target.insert(
        KEY.to_string(),
        if enabled {
            vec!["channel".to_string()]
        } else {
            Vec::new()
        },
    );
    NotificationConfig {
        channels: vec![settings(stabilize)],
        global: vec!["channel".to_string()],
        by_target,
    }
}

fn down(cause: Option<&str>) -> Message {
    let mut message = Message::target_down(
        TargetRef::new(KEY, None),
        Cause::new("TCP.REFUSED", cause.unwrap_or("the connection was refused")),
        1_764_000_000_000,
    );
    message.at = Instant::now();
    message
}

fn up() -> Message {
    let mut message = Message::target_up(TargetRef::new(KEY, None), 1_764_000_000_000);
    message.at = Instant::now();
    message
}

fn wait_for(condition: impl Fn() -> bool, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn start(config: &NotificationConfig, sink: &FakeChannel) -> network_monitor::notify::Handle {
    start_with(config, |_| Box::new(sink.clone()), &Shutdown::new()).expect("the channel starts")
}

#[test]
fn a_disconnection_is_delivered_at_once() {
    let sink = FakeChannel::always_ok("channel", Kind::LogFile);
    let handle = start(&config(Duration::from_secs(60), true), &sink);

    handle
        .notifier
        .send(down(Some("TCP.REFUSED — the connection was refused")));
    wait_for(
        || sink.sent().iter().any(|body| body.starts_with("DOWN")),
        Duration::from_secs(2),
        "the down message",
    );

    let sent = sink.sent();
    let down = sent
        .iter()
        .find(|body| body.starts_with("DOWN"))
        .expect("the down message");
    assert!(down.contains("Cause: TCP.REFUSED"), "unexpected: {down}");
    assert_eq!(
        sent.iter().filter(|body| body.starts_with("DOWN")).count(),
        1,
        "one message per transition"
    );

    handle.shutdown(None);
}

#[test]
fn a_recovery_is_delivered_when_the_window_closes() {
    let sink = FakeChannel::always_ok("channel", Kind::LogFile);
    let handle = start(&config(Duration::from_millis(300), true), &sink);

    handle.notifier.send(down(None));
    wait_for(
        || sink.sent().iter().any(|body| body.starts_with("DOWN")),
        Duration::from_secs(2),
        "the down message",
    );

    handle.notifier.send(up());
    assert!(
        !sink.sent().iter().any(|body| body.starts_with("UP")),
        "the recovery waits for the window"
    );

    wait_for(
        || sink.sent().iter().any(|body| body.starts_with("UP")),
        Duration::from_secs(5),
        "the up message",
    );
    handle.shutdown(None);
}

#[test]
fn a_flap_inside_the_window_sends_nothing() {
    let sink = FakeChannel::always_ok("channel", Kind::LogFile);
    let handle = start(&config(Duration::from_millis(300), true), &sink);

    handle.notifier.send(down(None));
    wait_for(
        || sink.sent().iter().any(|body| body.starts_with("DOWN")),
        Duration::from_secs(2),
        "the down message",
    );

    handle.notifier.send(up());
    std::thread::sleep(Duration::from_millis(50));
    handle.notifier.send(down(None));

    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !sink.sent().iter().any(|body| body.starts_with("UP")),
        "the flap is silent"
    );
    assert_eq!(
        sink.sent()
            .iter()
            .filter(|body| body.starts_with("DOWN"))
            .count(),
        1
    );
    handle.shutdown(None);
}

#[test]
fn a_failed_delivery_is_retried_before_it_gives_up() {
    let failure = || SendError {
        retryable: true,
        reason: "temporary".to_string(),
    };
    // The first send is the channel's own start line: it succeeds. The down then
    // fails twice, waiting 1 and 2 seconds, and succeeds on the third attempt.
    let sink = FakeChannel::with_outcomes(
        "channel",
        Kind::LogFile,
        vec![Ok(()), Err(failure()), Err(failure())],
    );
    let handle = start(&config(Duration::from_secs(60), true), &sink);

    handle.notifier.send(down(None));
    wait_for(
        || sink.attempts() >= 4,
        Duration::from_secs(10),
        "the retried message",
    );

    assert_eq!(
        sink.sent()
            .iter()
            .filter(|body| body.starts_with("DOWN"))
            .count(),
        1,
        "the payload is rendered once"
    );
    handle.shutdown(None);
}

#[test]
fn the_startup_message_reaches_the_root_list() {
    let sink = FakeChannel::always_ok("channel", Kind::LogFile);
    let handle = start(&config(Duration::from_secs(60), true), &sink);

    handle.notifier.send(Message::startup(
        Some("/etc/network-monitor.toml".to_string()),
        vec![KEY.to_string()],
        Some("http://0.0.0.0:8642/".to_string()),
        0,
    ));
    wait_for(
        || sink.sent().iter().any(|body| body.starts_with("READY")),
        Duration::from_secs(2),
        "the startup message",
    );

    let ready = sink
        .sent()
        .into_iter()
        .find(|body| body.starts_with("READY"))
        .expect("the startup payload");
    assert!(ready.contains("Config: /etc/network-monitor.toml"));
    assert!(ready.contains("\n- tcp://1.1.1.1:443"));
    handle.shutdown(None);
}

#[test]
fn a_silenced_target_produces_nothing() {
    let sink = FakeChannel::always_ok("channel", Kind::LogFile);
    let handle = start(&config(Duration::from_secs(60), false), &sink);

    handle.notifier.send(down(None));
    std::thread::sleep(Duration::from_millis(300));
    assert!(
        !sink.sent().iter().any(|body| body.starts_with("DOWN")),
        "the target enables no channel"
    );
    handle.shutdown(None);
}

#[test]
fn the_farewell_reaches_the_root_list_once() {
    let sink = FakeChannel::always_ok("channel", Kind::LogFile);
    let handle = start(&config(Duration::from_secs(60), true), &sink);

    handle.shutdown(Some(Message::shutdown(0)));
    assert_eq!(
        sink.sent()
            .iter()
            .filter(|body| body.starts_with("STOPPED"))
            .count(),
        1
    );
    assert_eq!(sink.closed(), 1);
}
