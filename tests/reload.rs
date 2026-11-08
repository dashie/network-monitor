//! FR-16: the running monitor re-reads a changed configuration file.
//!
//! The supervisor polls the file once per second, so these tests wait for the
//! change instead of assuming an instant reaction.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use network_monitor::config::{self, Config, Overrides};
use network_monitor::monitor::{self, Reload};
use network_monitor::probe::fake::{up, FakeProbe};
use network_monitor::probe::Probe;
use network_monitor::shutdown::Shutdown;
use network_monitor::store::{EventFilter, Store};

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
    let loaded = config::load(Some(config_path)).unwrap().unwrap();
    let settings = Config::resolve(Some(loaded.config), Overrides::default()).unwrap();
    let reload = Reload {
        path: config_path.to_path_buf(),
        overrides: Overrides::default(),
    };
    let prober: Arc<dyn Probe> = Arc::new(FakeProbe::always(up()));
    let store = Arc::new(Mutex::new(Store::open_rw(database).unwrap()));
    let stop = Shutdown::new();
    let running = stop.clone();

    let handle =
        thread::spawn(move || monitor::run(&settings, prober, store, &running, Some(reload)));

    Running {
        stop,
        handle: Some(handle),
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
    let config_path = directory.path().join("network-monitor.toml");
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
