//! FR-16: the running monitor re-reads a changed configuration file.
//!
//! The supervisor polls the file once per second, so these tests wait for the
//! change instead of assuming an instant reaction.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use uptime_monitor::config::{self, Config, Overrides};
use uptime_monitor::monitor::{self, Reload};
use uptime_monitor::notify::fake::FakeNotifier;
use uptime_monitor::notify::Notifier;
use uptime_monitor::probe::fake::{up, FakeProbe};
use uptime_monitor::probe::Probe;
use uptime_monitor::shutdown::Shutdown;
use uptime_monitor::store::{EventFilter, Store};

struct Running {
    stop: Shutdown,
    handle: Option<thread::JoinHandle<Result<(), monitor::RunError>>>,
}

impl Running {
    fn is_finished(&self) -> bool {
        self.handle
            .as_ref()
            .map(|handle| handle.is_finished())
            .unwrap_or(true)
    }

    fn stop(mut self) {
        self.stop.request();
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .expect("the monitor thread does not panic")
                .unwrap();
        }
    }
}

fn write_config(config_path: &Path, database: &Path, interval_ms: u64, targets: &[&str]) {
    let mut text = format!(
        "interval_ms = {interval_ms}\ntimeout_ms = 500\ndatabase = \"{}\"\nretention_days = 0\n",
        database.display()
    );
    for target in targets {
        text.push_str(&format!("\n[[targets]]\nip = \"{target}\"\n"));
    }
    std::fs::write(config_path, text).expect("the test writes the configuration file");
}

fn start(config_path: &Path, database: &Path) -> Running {
    start_with_probe(config_path, database).0
}

/// The same start, keeping the fake so a test can inspect the probes it received.
fn start_with_probe(config_path: &Path, database: &Path) -> (Running, Arc<FakeProbe>) {
    let loaded = config::load(Some(config_path)).unwrap().unwrap();
    let settings = Config::resolve(Some(loaded.config), Overrides::default()).unwrap();
    let reload = Reload {
        path: config_path.to_path_buf(),
        overrides: Overrides::default(),
    };
    let probe = Arc::new(FakeProbe::always(up()));
    let prober: Arc<dyn Probe> = probe.clone();
    let store = Arc::new(Mutex::new(Store::open_rw(database).unwrap()));
    let stop = Shutdown::new();
    let running = stop.clone();

    let handle = thread::spawn(move || {
        monitor::run(
            &settings,
            prober,
            store,
            &running,
            Some(reload),
            monitor::Start::default(),
        )
    });

    (
        Running {
            stop,
            handle: Some(handle),
        },
        probe,
    )
}

/// The same start over `run_with_notifier`, so a test can inspect what was notified.
fn start_with_notifier(config_path: &Path, database: &Path) -> (Running, Arc<FakeNotifier>) {
    let loaded = config::load(Some(config_path)).unwrap().unwrap();
    let settings = Config::resolve(Some(loaded.config), Overrides::default()).unwrap();
    let reload = Reload {
        path: config_path.to_path_buf(),
        overrides: Overrides::default(),
    };
    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::always(up()));
    let store = Arc::new(Mutex::new(Store::open_rw(database).unwrap()));
    let notifier = Arc::new(FakeNotifier::new());
    let sink: Arc<dyn Notifier> = notifier.clone();
    let stop = Shutdown::new();
    let running = stop.clone();

    let handle = thread::spawn(move || {
        monitor::run_with_notifier(
            &settings,
            prober,
            store,
            &running,
            Some(reload),
            sink,
            monitor::Start::default(),
        )
    });

    (
        Running {
            stop,
            handle: Some(handle),
        },
        notifier,
    )
}

fn write_config_with_telegram(
    config_path: &Path,
    database: &Path,
    token: &str,
    chat_id: &str,
    targets: &[&str],
) {
    let mut text = format!(
        "interval_ms = 30\ntimeout_ms = 500\ndatabase = \"{}\"\nretention_days = 0\n\n\
         [telegram]\nbot_token = \"{token}\"\nchat_id = \"{chat_id}\"\n",
        database.display()
    );
    for target in targets {
        text.push_str(&format!("\n[[targets]]\nip = \"{target}\"\n"));
    }
    std::fs::write(config_path, text).expect("the test writes the configuration file");
}

fn wait_for_notifications(notifier: &FakeNotifier, expected: usize, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while notifier.len() < expected {
        assert!(
            Instant::now() < deadline,
            "expected {expected} notification(s) within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_probe(probe: &FakeProbe, key: &str, timeout: Duration, within: Duration) {
    let deadline = Instant::now() + within;
    loop {
        if probe
            .calls()
            .iter()
            .any(|call| call.endpoint.to_string() == key && call.timeout == timeout)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no probe of {key} with a {timeout:?} timeout within {within:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_events(database: &Path, expected: usize, timeout: Duration) {
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
        thread::sleep(Duration::from_millis(25));
    }
}

fn temp_paths() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let config_path = directory.path().join("uptime-monitor.toml");
    let database = directory.path().join("events.db");
    (directory, config_path, database)
}

#[test]
fn a_new_target_starts_monitoring_without_a_restart() {
    let (_directory, config_path, database) = temp_paths();
    write_config(&config_path, &database, 30, &["127.0.0.1"]);

    let running = start(&config_path, &database);
    wait_for_events(&database, 1, Duration::from_secs(5));

    write_config(&config_path, &database, 30, &["127.0.0.1", "127.0.0.2"]);
    wait_for_events(&database, 2, Duration::from_secs(15));

    assert!(!running.is_finished(), "the monitor keeps running");
    running.stop();

    let store = Store::open_ro(&database).unwrap();
    let events = store.events(&EventFilter::default()).unwrap();
    assert_eq!(events.len(), 2, "one event per target, no duplicates");
    assert_eq!(store.states().unwrap().len(), 2);
}

#[test]
fn a_reloaded_gap_threshold_is_visible_and_records_no_gap() {
    let (_directory, config_path, database) = temp_paths();
    let write = |gap_after_ms: u64| {
        let text = format!(
            "interval_ms = 30\ntimeout_ms = 500\ndatabase = \"{}\"\nretention_days = 0\ngap_after_ms = {gap_after_ms}\n\n[[targets]]\nip = \"127.0.0.1\"\n",
            database.display()
        );
        std::fs::write(&config_path, text).expect("the test writes the configuration file");
    };
    write(5_000);

    let running = start(&config_path, &database);
    wait_for_events(&database, 1, Duration::from_secs(5));

    write(120_000);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(reader) = Store::open_ro(&database) {
            if reader
                .liveness()
                .ok()
                .flatten()
                .is_some_and(|liveness| liveness.gap_after_ms == 120_000)
            {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the reloaded threshold becomes visible"
        );
        thread::sleep(Duration::from_millis(20));
    }

    let reader = Store::open_ro(&database).unwrap();
    let events = reader.events(&EventFilter::default()).unwrap();
    assert_eq!(events.len(), 1, "a reload never records a gap");
    running.stop();
}

#[test]
fn an_invalid_file_keeps_the_previous_configuration_and_the_monitor() {
    let (_directory, config_path, database) = temp_paths();
    write_config(&config_path, &database, 30, &["127.0.0.1"]);

    let running = start(&config_path, &database);
    wait_for_events(&database, 1, Duration::from_secs(5));

    std::fs::write(&config_path, "interval_ms = 30\nbroken = [\n").unwrap();
    thread::sleep(Duration::from_millis(1_500));

    assert!(
        !running.is_finished(),
        "an invalid file never stops the monitor"
    );
    running.stop();

    let store = Store::open_ro(&database).unwrap();
    assert_eq!(store.events(&EventFilter::default()).unwrap().len(), 1);
    assert_eq!(store.states().unwrap().len(), 1);
}

#[test]
fn a_configuration_without_targets_is_rejected() {
    let (_directory, config_path, database) = temp_paths();
    write_config(&config_path, &database, 30, &["127.0.0.1"]);

    let running = start(&config_path, &database);
    wait_for_events(&database, 1, Duration::from_secs(5));

    std::fs::write(
        &config_path,
        format!("interval_ms = 30\ndatabase = \"{}\"\n", database.display()),
    )
    .unwrap();
    thread::sleep(Duration::from_millis(1_500));

    assert!(!running.is_finished());
    running.stop();

    let store = Store::open_ro(&database).unwrap();
    assert_eq!(store.states().unwrap().len(), 1);
}

#[test]
fn a_changed_database_path_is_reported_and_not_applied() {
    let (_directory, config_path, database) = temp_paths();
    let other_database = database.with_file_name("other.db");
    write_config(&config_path, &database, 30, &["127.0.0.1"]);

    let running = start(&config_path, &database);
    wait_for_events(&database, 1, Duration::from_secs(5));

    write_config(&config_path, &other_database, 30, &["127.0.0.1"]);
    thread::sleep(Duration::from_millis(1_500));

    assert!(!running.is_finished());
    running.stop();

    assert!(
        !other_database.exists(),
        "the running monitor keeps writing to the database it opened"
    );
    let store = Store::open_ro(&database).unwrap();
    assert_eq!(store.states().unwrap().len(), 1);
}

/// A changed `[telegram]` section is refused by the running monitor.
///
/// The section is read once, when the delivery thread starts, so a live change is
/// reported and the running settings are kept — exactly like a changed database path.
#[test]
fn a_changed_telegram_section_is_reported_and_not_applied() {
    let (_directory, config_path, database) = temp_paths();
    write_config_with_telegram(
        &config_path,
        &database,
        "111:first",
        "-1001",
        &["127.0.0.1"],
    );

    let (running, notifier) = start_with_notifier(&config_path, &database);
    wait_for_notifications(&notifier, 1, Duration::from_secs(5));

    write_config_with_telegram(
        &config_path,
        &database,
        "222:second",
        "-2002",
        &["127.0.0.1"],
    );
    thread::sleep(Duration::from_millis(1_500));
    assert!(!running.is_finished(), "the monitor keeps running");

    // A target added afterwards still notifies through the notifier the monitor started
    // with: the section is never re-applied by a reload.
    write_config_with_telegram(
        &config_path,
        &database,
        "222:second",
        "-2002",
        &["127.0.0.1", "127.0.0.2"],
    );
    wait_for_notifications(&notifier, 2, Duration::from_secs(15));
    running.stop();
}

/// A reload replaces the method and the timeout of a target without a restart.
#[test]
fn a_changed_target_takes_its_new_method_and_timeout() {
    let (_directory, config_path, database) = temp_paths();
    write_config(&config_path, &database, 30, &["127.0.0.1"]);

    let (running, probe) = start_with_probe(&config_path, &database);
    wait_for_probe(
        &probe,
        "icmp://127.0.0.1",
        Duration::from_millis(500),
        Duration::from_secs(5),
    );

    std::fs::write(
        &config_path,
        format!(
            "interval_ms = 30\ntimeout_ms = 500\ndatabase = \"{}\"\n\n\
             [[targets]]\nmethod = \"tcp\"\nhost = \"127.0.0.1\"\nport = 9\n\
             timeout_ms = 120\n",
            database.display()
        ),
    )
    .unwrap();

    wait_for_probe(
        &probe,
        "tcp://127.0.0.1:9",
        Duration::from_millis(120),
        Duration::from_secs(15),
    );

    assert!(!running.is_finished(), "the monitor keeps running");
    running.stop();
}

/// An applied reload reaches the channels as a message, so a chat hears that the
/// configuration changed and not only that a target moved.
#[test]
fn an_applied_reload_is_reported_as_a_message() {
    let (_directory, config_path, database) = temp_paths();
    write_config(&config_path, &database, 30, &["127.0.0.1"]);

    let (running, notifier) = start_with_notifier(&config_path, &database);
    wait_for_notifications(&notifier, 1, Duration::from_secs(5));

    write_config(&config_path, &database, 30, &["127.0.0.1", "127.0.0.2"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if notifier
            .messages()
            .iter()
            .any(|message| message.event_name() == "config_reload")
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the reload message"
        );
        thread::sleep(Duration::from_millis(50));
    }
    running.stop();
}

/// A promotion is the process's own write: the watcher rebases, so the next poll is quiet.
#[test]
fn a_promotion_is_not_replayed_as_a_reload() {
    let (directory, config_path, database) = temp_paths();
    write_config(&config_path, &database, 30, &["127.0.0.1"]);
    let candidate_path = directory.path().join("next.toml");
    write_config(&candidate_path, &database, 30, &["127.0.0.1", "127.0.0.2"]);
    let bytes = std::fs::read(&candidate_path).expect("the test reads the candidate");
    let candidate = uptime_monitor::promote::Candidate {
        source: candidate_path,
        main: config_path.clone(),
        bytes: bytes.clone(),
        stamp: 1_794_096_000_000,
    };

    let (running, notifier) = start_with_promotion(&config_path, &database, candidate);
    wait_for_file(&config_path, &bytes, Duration::from_secs(10));
    // Two watcher polls after the write: a spurious reload would appear here.
    thread::sleep(Duration::from_millis(2_500));

    assert!(
        !running.is_finished(),
        "a promotion never stops the monitor"
    );
    running.stop();

    let reloads: Vec<_> = notifier
        .messages()
        .into_iter()
        .filter(|message| message.event_name() == "config_reload")
        .collect();
    assert_eq!(
        reloads.len(),
        1,
        "one promotion message and no reload: {:?}",
        reloads
            .iter()
            .map(|message| message.text())
            .collect::<Vec<_>>()
    );
    assert!(reloads[0].text().contains("promoted"));
}

fn start_with_promotion(
    config_path: &Path,
    database: &Path,
    candidate: uptime_monitor::promote::Candidate,
) -> (Running, Arc<FakeNotifier>) {
    let loaded = config::load(Some(config_path)).unwrap().unwrap();
    let settings = Config::resolve(Some(loaded.config), Overrides::default()).unwrap();
    let reload = Reload {
        path: config_path.to_path_buf(),
        overrides: Overrides::default(),
    };
    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::always(up()));
    let store = Arc::new(Mutex::new(Store::open_rw(database).unwrap()));
    let notifier = Arc::new(FakeNotifier::new());
    let sink: Arc<dyn Notifier> = notifier.clone();
    let stop = Shutdown::new();
    let running = stop.clone();

    let handle = thread::spawn(move || {
        monitor::run_with_notifier(
            &settings,
            prober,
            store,
            &running,
            Some(reload),
            sink,
            monitor::Start {
                promotion: Some(candidate),
                ..Default::default()
            },
        )
    });

    (
        Running {
            stop,
            handle: Some(handle),
        },
        notifier,
    )
}

fn wait_for_file(path: &Path, expected: &[u8], timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if std::fs::read(path)
            .map(|bytes| bytes == expected)
            .unwrap_or(false)
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "{} did not hold the promoted bytes within {timeout:?}",
            path.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}
