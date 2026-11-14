//! The notifier end to end with a fake sender: no network and no real clock.

use std::time::{Duration, Instant};

use network_monitor::config::Telegram;
use network_monitor::notify::fake::FakeSender;
use network_monitor::notify::{start_with, Notification, SendError};
use network_monitor::shutdown::Shutdown;
use network_monitor::store::TargetState;

const KEY: &str = "tcp://1.1.1.1:443";

fn settings(stabilize: Duration) -> Telegram {
    Telegram {
        bot_token: "123456:abc".to_string(),
        chat_id: "-100200".to_string(),
        stabilize,
    }
}

fn notification(state: TargetState, cause: Option<&str>) -> Notification {
    Notification {
        target: KEY.to_string(),
        key: KEY.to_string(),
        state,
        cause: cause.map(str::to_string),
        occurred_at_ms: 1_764_000_000_000,
        at: Instant::now(),
    }
}

fn wait_for(condition: impl Fn() -> bool, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_disconnection_is_delivered_at_once() {
    let shutdown = Shutdown::new();
    let sink = FakeSender::always_ok();
    let handle = start_with(
        &settings(Duration::from_secs(60)),
        Box::new(sink.clone()),
        &shutdown,
    );

    handle.notifier.notify(notification(
        TargetState::Down,
        Some("TCP.REFUSED — the connection was refused"),
    ));

    wait_for(
        || sink.attempts() >= 1,
        Duration::from_secs(2),
        "the down message",
    );
    let sent = sink.sent();
    assert_eq!(sent.len(), 1, "one message per transition");
    assert!(
        sent[0].starts_with(&format!("DOWN {KEY}\n")),
        "unexpected: {}",
        sent[0]
    );
    assert!(
        sent[0].contains("Cause: TCP.REFUSED"),
        "unexpected: {}",
        sent[0]
    );

    handle.shutdown();
}

#[test]
fn a_recovery_is_delivered_when_the_window_closes() {
    let shutdown = Shutdown::new();
    let sink = FakeSender::always_ok();
    let handle = start_with(
        &settings(Duration::from_millis(300)),
        Box::new(sink.clone()),
        &shutdown,
    );

    handle
        .notifier
        .notify(notification(TargetState::Down, None));
    wait_for(
        || sink.attempts() >= 1,
        Duration::from_secs(2),
        "the down message",
    );

    handle.notifier.notify(notification(TargetState::Up, None));
    assert_eq!(sink.attempts(), 1, "the recovery waits for the window");

    wait_for(
        || sink.attempts() >= 2,
        Duration::from_secs(5),
        "the up message",
    );
    assert!(
        sink.sent()[1].starts_with(&format!("UP {KEY}\n")),
        "unexpected: {}",
        sink.sent()[1]
    );

    handle.shutdown();
}

#[test]
fn a_flap_inside_the_window_sends_nothing() {
    let shutdown = Shutdown::new();
    let sink = FakeSender::always_ok();
    let handle = start_with(
        &settings(Duration::from_millis(300)),
        Box::new(sink.clone()),
        &shutdown,
    );

    handle
        .notifier
        .notify(notification(TargetState::Down, None));
    wait_for(
        || sink.attempts() >= 1,
        Duration::from_secs(2),
        "the down message",
    );

    handle.notifier.notify(notification(TargetState::Up, None));
    std::thread::sleep(Duration::from_millis(50));
    handle
        .notifier
        .notify(notification(TargetState::Down, None));

    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(sink.attempts(), 1, "the flap is silent");

    handle.shutdown();
}

#[test]
fn a_failed_delivery_is_retried_before_it_gives_up() {
    let shutdown = Shutdown::new();
    let sink = FakeSender::new(vec![
        Err(SendError {
            retryable: true,
            reason: "temporary".to_string(),
        }),
        Err(SendError {
            retryable: true,
            reason: "temporary".to_string(),
        }),
    ]);
    let handle = start_with(
        &settings(Duration::from_secs(60)),
        Box::new(sink.clone()),
        &shutdown,
    );

    handle
        .notifier
        .notify(notification(TargetState::Down, None));

    wait_for(
        || sink.attempts() >= 3,
        Duration::from_secs(10),
        "the retried message",
    );
    let sent = sink.sent();
    assert_eq!(sent.len(), 3, "two failures then one success");
    assert!(
        sent.iter().all(|text| text == &sent[0]),
        "the same message every time"
    );

    handle.shutdown();
}

#[test]
fn an_announcement_reaches_the_channel_as_it_is() {
    let shutdown = Shutdown::new();
    let sink = FakeSender::always_ok();
    let handle = start_with(
        &settings(Duration::from_secs(60)),
        Box::new(sink.clone()),
        &shutdown,
    );

    handle
        .notifier
        .announce("READY network-monitor\nTargets: 0\n".to_string());

    wait_for(
        || sink.attempts() >= 1,
        Duration::from_secs(2),
        "the startup announcement",
    );
    assert_eq!(
        sink.sent(),
        vec!["READY network-monitor\nTargets: 0\n".to_string()],
        "the announcement is sent without the state policy touching it"
    );

    handle.shutdown();
}
