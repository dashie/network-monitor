//! INV-1 and INV-5 at the decision level, plus the full loop with a fake prober.

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use network_monitor::config::{Config, FileConfig, FileTarget, Overrides};
use network_monitor::monitor;
use network_monitor::probe::fake::{up, FakeProbe};
use network_monitor::probe::{Probe, ProbeError, Verdict};
use network_monitor::shutdown::Shutdown;
use network_monitor::store::{EventFilter, Store, TargetState};

#[test]
fn a_local_error_is_not_a_state_change() {
    let probe = FakeProbe::scripted(vec![
        Ok(up()),
        Err(ProbeError::local("the interface disappeared")),
        Ok(up()),
    ]);

    assert_eq!(probe.probe(Ipv4Addr::LOCALHOST, None).unwrap(), up());
    assert!(probe.probe(Ipv4Addr::LOCALHOST, None).is_err());
    assert_eq!(probe.probe(Ipv4Addr::LOCALHOST, None).unwrap(), up());
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
        Ok(Verdict::Down),
        Ok(Verdict::Down),
        Ok(up()),
    ]));
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));

    let handle = thread::spawn(move || monitor::run(&config, prober, store, &running));

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
    let handle = thread::spawn(move || monitor::run(&first, prober, store, &running));
    wait_for_events(&expected, 1, Duration::from_secs(5));
    stop.request();
    handle.join().unwrap().unwrap();

    let stop = Shutdown::new();
    let running = stop.clone();
    let second = test_config(&database);
    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::always(Verdict::Down));
    let store = Arc::new(Mutex::new(Store::open_rw(&database).unwrap()));
    let handle = thread::spawn(move || monitor::run(&second, prober, store, &running));
    wait_for_events(&expected, 2, Duration::from_secs(5));
    stop.request();
    handle.join().unwrap().unwrap();

    let reader = Store::open_ro(&expected).unwrap();
    let events = reader.events(&EventFilter::default()).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].previous_state, TargetState::Up);
    assert_eq!(events[0].state, TargetState::Down);
}

fn test_config(database: &std::path::Path) -> Config {
    Config::resolve(
        Some(FileConfig {
            interval_ms: Some(30),
            timeout_ms: Some(500),
            database: Some(database.to_string_lossy().into_owned()),
            retention_days: None,
            targets: Some(vec![FileTarget {
                ip: "127.0.0.1".to_string(),
                interface: None,
                label: Some("loop-test".to_string()),
            }]),
        }),
        Overrides::default(),
    )
    .unwrap()
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
