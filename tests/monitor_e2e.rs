//! INV-1 and INV-5 at the decision level, plus the full loop with a fake prober.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use uptime_monitor::config::{Config, FileConfig, FileTarget, Overrides};
use uptime_monitor::monitor;
use uptime_monitor::notify::fake::FakeNotifier;
use uptime_monitor::notify::Notifier;
use uptime_monitor::notify::{Event, Message};
use uptime_monitor::probe::fake::{down, up, FakeProbe};
use uptime_monitor::probe::{Code, Details, Endpoint, Probe, ProbeError};
use uptime_monitor::shutdown::Shutdown;
use uptime_monitor::store::{EventFilter, Store, TargetState};

/// The endpoint and the timeout a direct fake call uses.
fn loopback() -> Endpoint {
    Endpoint::Icmp {
        address: Ipv4Addr::LOCALHOST,
    }
}

fn timeout() -> Duration {
    Duration::from_millis(2_000)
}

#[test]
fn a_local_error_is_not_a_state_change() {
    let probe = FakeProbe::scripted(vec![
        Ok(up()),
        Err(ProbeError::of(
            Code::TcpInterface,
            &Details {
                interface: Some("eth1"),
                reason: Some("the interface disappeared"),
                ..Details::default()
            },
        )),
        Ok(up()),
    ]);

    assert_eq!(probe.probe(&loopback(), None, timeout()).unwrap(), up());
    assert!(probe.probe(&loopback(), None, timeout()).is_err());
    assert_eq!(probe.probe(&loopback(), None, timeout()).unwrap(), up());
}

#[test]
fn the_monitor_records_only_transitions() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("events.db");
    let config = test_config(&database);
    let expected = database.clone();

    let stop = Shutdown::new();
    let running = stop.clone();

    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::scripted(vec![
        Ok(up()),
        Ok(up()),
        Ok(down()),
        Ok(down()),
        Ok(up()),
    ]));
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));

    let handle = thread::spawn(move || {
        monitor::run(
            &config,
            prober,
            store,
            &running,
            None,
            monitor::Start::default(),
        )
    });

    wait_for_events(&expected, 3, Duration::from_secs(5));
    stop.request();
    handle
        .join()
        .expect("the monitor thread does not panic")
        .unwrap();

    let reader = Store::open_ro(&expected).unwrap();
    let events = reader.events(&EventFilter::default()).unwrap();
    assert_eq!(events.len(), 3, "one event per transition");

    let states: Vec<&str> = events.iter().map(|event| event.state.as_str()).collect();
    assert_eq!(states, vec!["up", "down", "up"]);

    let target = &reader.states().unwrap()[0];
    assert_eq!(target.state, TargetState::Up);
    assert_eq!(target.label.as_deref(), Some("loop-test"));
}

#[test]
fn a_restart_continues_from_the_stored_state() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("events.db");
    let expected = database.clone();

    let stop = Shutdown::new();
    let running = stop.clone();
    let first = test_config(&database);
    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::always(up()));
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));
    let handle = thread::spawn(move || {
        monitor::run(
            &first,
            prober,
            store,
            &running,
            None,
            monitor::Start::default(),
        )
    });
    wait_for_events(&expected, 1, Duration::from_secs(5));
    stop.request();
    handle.join().unwrap().unwrap();

    let stop = Shutdown::new();
    let running = stop.clone();
    let second = test_config(&database);
    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::always(down()));
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));
    let handle = thread::spawn(move || {
        monitor::run(
            &second,
            prober,
            store,
            &running,
            None,
            monitor::Start::default(),
        )
    });
    wait_for_events(&expected, 2, Duration::from_secs(5));
    stop.request();
    handle.join().unwrap().unwrap();

    let reader = Store::open_ro(&expected).unwrap();
    let events = reader.events(&EventFilter::default()).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].previous_state, TargetState::Up);
    assert_eq!(events[0].state, TargetState::Down);
}

/// Poll the database from another connection, exactly as `query` does.
fn wait_for_events(database: &std::path::Path, expected: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(store) = Store::open_ro(database) {
            if let Ok(events) = store.events(&EventFilter::default()) {
                if events.len() >= expected {
                    return;
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "expected {expected} event(s) within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

/// The notifier is fed after the event is committed, so waiting on it is enough.
fn wait_for_notifications(notifier: &FakeNotifier, expected: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while notifier.len() < expected {
        assert!(
            Instant::now() < deadline,
            "expected {expected} notification(s) within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

fn test_config(database: &std::path::Path) -> Config {
    Config::resolve(
        Some(FileConfig {
            interval_ms: Some(30),
            timeout_ms: Some(500),
            database: Some(database.to_string_lossy().into_owned()),
            retention_days: None,
            targets: Some(vec![FileTarget {
                ip: Some("127.0.0.1".to_string()),
                label: Some("loop-test".to_string()),
                ..FileTarget::default()
            }]),
            ..FileConfig::default()
        }),
        Overrides::default(),
    )
    .unwrap()
}

/// Each recorded transition is one notification, and a local probe error is none.
#[test]
fn each_recorded_transition_is_notified_once() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("events.db");
    let config = test_config(&database);

    let stop = Shutdown::new();
    let running = stop.clone();

    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::scripted(vec![
        Ok(up()),
        Err(ProbeError::of(
            Code::TcpInterface,
            &Details {
                interface: Some("eth1"),
                reason: Some("the interface disappeared"),
                ..Details::default()
            },
        )),
        Ok(down()),
        Ok(down()),
        Ok(up()),
    ]));
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));
    let notifier = Arc::new(FakeNotifier::new());
    let sink: Arc<dyn Notifier> = notifier.clone();

    let handle = thread::spawn(move || {
        monitor::run_with_notifier(
            &config,
            prober,
            store,
            &running,
            None,
            sink,
            monitor::Start::default(),
        )
    });

    wait_for_notifications(&notifier, 3, Duration::from_secs(5));
    stop.request();
    handle
        .join()
        .expect("the monitor thread does not panic")
        .unwrap();

    let received = notifier.messages();
    assert_eq!(received.len(), 3, "one notification per transition");
    let events: Vec<&str> = received.iter().map(|item| item.event_name()).collect();
    assert_eq!(events, vec!["target_up", "target_down", "target_up"]);
    assert_eq!(target_of(&received[0]).key, "icmp://127.0.0.1");
    assert_eq!(target_of(&received[0]).display(), "loop-test");
    assert_eq!(
        cause_of(&received[0]),
        None,
        "a return to up carries no cause"
    );
    assert!(
        cause_of(&received[1])
            .unwrap_or_default()
            .starts_with("TCP."),
        "unexpected: {:?}",
        cause_of(&received[1])
    );
}

/// Each worker uses the timeout and the interval of its own target, not the globals.
#[test]
fn every_target_uses_its_own_timeout_and_interval() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("events.db");
    let config = Config::resolve(
        Some(FileConfig {
            interval_ms: Some(10),
            timeout_ms: Some(500),
            database: Some(database.to_string_lossy().into_owned()),
            targets: Some(vec![
                FileTarget {
                    ip: Some("127.0.0.1".to_string()),
                    timeout_ms: Some(150),
                    interval_ms: Some(10),
                    ..FileTarget::default()
                },
                FileTarget {
                    ip: Some("127.0.0.2".to_string()),
                    timeout_ms: Some(400),
                    interval_ms: Some(2_000),
                    ..FileTarget::default()
                },
            ]),
            ..FileConfig::default()
        }),
        Overrides::default(),
    )
    .unwrap();

    let probe = Arc::new(FakeProbe::always(up()));
    let prober: Arc<dyn Probe> = probe.clone();
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));
    let stop = Shutdown::new();
    let running = stop.clone();
    let handle = thread::spawn(move || {
        monitor::run(
            &config,
            prober,
            store,
            &running,
            None,
            monitor::Start::default(),
        )
    });

    thread::sleep(Duration::from_millis(450));
    stop.request();
    handle
        .join()
        .expect("the monitor thread does not panic")
        .unwrap();

    let calls = probe.calls();
    let count = |key: &str| {
        calls
            .iter()
            .filter(|call| call.endpoint.to_string() == key)
            .count()
    };
    let fast = count("icmp://127.0.0.1");
    let slow = count("icmp://127.0.0.2");

    assert!(
        calls
            .iter()
            .all(|call| call.timeout == Duration::from_millis(150)
                || call.timeout == Duration::from_millis(400)),
        "unexpected timeout in {calls:?}"
    );
    assert!(fast >= 3, "the 10 ms target was probed {fast} time(s)");
    assert!(slow <= 2, "the 2 s target was probed {slow} time(s)");
}

/// The channel learns that the monitor started, before the first transition.
#[test]
fn the_startup_announcement_reaches_the_channel_once() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("events.db");
    let config = test_config(&database);

    let stop = Shutdown::new();
    let running = stop.clone();
    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::always(up()));
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));
    let notifier = Arc::new(FakeNotifier::new());
    let sink: Arc<dyn Notifier> = notifier.clone();
    let startup = Message::startup(
        Some("/etc/uptime-monitor.toml".to_string()),
        vec!["icmp://127.0.0.1".to_string()],
        None,
        0,
    );

    let handle = thread::spawn(move || {
        monitor::run_with_notifier(
            &config,
            prober,
            store,
            &running,
            None,
            sink,
            monitor::Start {
                startup: Some(startup),
                ..Default::default()
            },
        )
    });

    let deadline = Instant::now() + Duration::from_secs(5);
    while notifier.messages().is_empty() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the startup message"
        );
        thread::sleep(Duration::from_millis(20));
    }
    stop.request();
    handle
        .join()
        .expect("the monitor thread does not panic")
        .unwrap();

    let messages = notifier.messages();
    let startup: Vec<&Message> = messages
        .iter()
        .filter(|message| message.event_name() == "startup")
        .collect();
    assert_eq!(startup.len(), 1, "one startup message per start");
    assert!(startup[0].text().starts_with("READY uptime-monitor\n"));
    assert!(startup[0]
        .text()
        .contains("Config: /etc/uptime-monitor.toml"));
}

fn target_of(message: &Message) -> &uptime_monitor::notify::TargetRef {
    match &message.event {
        Event::TargetUp { target } | Event::TargetDown { target, .. } => target,
        other => panic!("not a target event: {other:?}"),
    }
}

fn cause_of(message: &Message) -> Option<&str> {
    match &message.event {
        Event::TargetDown { cause, .. } => Some(&cause.code),
        _ => None,
    }
}
