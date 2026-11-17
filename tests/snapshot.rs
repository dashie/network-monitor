//! The active snapshot: written once after a clean first round, never by a reload.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use uptime_monitor::config::{self, Config, Overrides};
use uptime_monitor::monitor::{self, Reload, Start};
use uptime_monitor::notify::fake::FakeNotifier;
use uptime_monitor::notify::{Event, Notifier};
use uptime_monitor::probe::fake::{up, FakeProbe};
use uptime_monitor::probe::{Code, Details, Probe, ProbeError};
use uptime_monitor::promote::Candidate;
use uptime_monitor::shutdown::Shutdown;
use uptime_monitor::snapshot::{self, Snapshot};
use uptime_monitor::store::Store;

struct Running {
    stop: Shutdown,
    handle: Option<thread::JoinHandle<Result<(), monitor::RunError>>>,
}

impl Running {
    fn stop(mut self) {
        self.stop.request();
        if let Some(handle) = self.handle.take() {
            handle
                .join()
                .expect("the monitor thread does not panic")
                .expect("the monitor stops cleanly");
        }
    }
}

fn temp_paths() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let directory = tempfile::tempdir().unwrap();
    let main = directory.path().join("uptime-monitor.toml");
    let database = directory.path().join("events.db");
    (directory, main, database)
}

fn write_config(path: &Path, database: &Path, port: u16, interval_ms: u64) {
    std::fs::write(
        path,
        format!(
            "interval_ms = {interval_ms}\ntimeout_ms = 500\ndatabase = \"{}\"\n\
             retention_days = 0\n\n[[targets]]\nmethod = \"tcp\"\n\
             host = \"127.0.0.1\"\nport = {port}\n",
            database.display()
        ),
    )
    .expect("the test writes the configuration");
}

fn load(path: &Path) -> Config {
    let loaded = config::load(Some(path)).unwrap().unwrap();
    Config::resolve(Some(loaded.config), Overrides::default()).expect("a valid configuration")
}

fn start(
    settings: Config,
    probe: Arc<FakeProbe>,
    database: &Path,
    start: Start,
    reload: Option<Reload>,
) -> (Running, Arc<FakeNotifier>) {
    let prober: Arc<dyn Probe> = probe;
    let store = Arc::new(Mutex::new(Store::open_rw(database).unwrap()));
    let notifier = Arc::new(FakeNotifier::new());
    let sink: Arc<dyn Notifier> = notifier.clone();
    let stop = Shutdown::new();
    let running = stop.clone();

    let handle = thread::spawn(move || {
        monitor::run_with_notifier(&settings, prober, store, &running, reload, sink, start)
    });

    (
        Running {
            stop,
            handle: Some(handle),
        },
        notifier,
    )
}

fn wait_for_snapshot(path: &Path, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "no snapshot at {} within {timeout:?}",
            path.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_for_reload(notifier: &FakeNotifier, applied: bool, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        if notifier.messages().iter().any(|message| {
            matches!(
                &message.event,
                Event::ConfigReload { applied: a, .. } if *a == applied
            )
        }) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "no reload (applied {applied}) within {timeout:?}"
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn temporaries(directory: &Path) -> Vec<String> {
    std::fs::read_dir(directory)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect()
}

#[test]
fn the_snapshot_is_written_once_after_a_clean_first_round() {
    let (directory, main, database) = temp_paths();
    write_config(&main, &database, 9, 30);
    let settings = load(&main);
    let bytes = std::fs::read(&main).unwrap();
    let snapshot_path = snapshot::path_for(&main);

    let probe = Arc::new(FakeProbe::always(up()));
    let (running, _notifier) = start(
        settings,
        probe,
        &database,
        Start {
            snapshot: Some(Snapshot {
                path: snapshot_path.clone(),
                bytes: bytes.clone(),
            }),
            ..Start::default()
        },
        None,
    );

    wait_for_snapshot(&snapshot_path, Duration::from_secs(10));
    running.stop();

    assert_eq!(
        std::fs::read(&snapshot_path).unwrap(),
        bytes,
        "the exact bytes the process started from"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&snapshot_path)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(
            mode, 0o600,
            "the snapshot carries the configuration secrets"
        );
    }
    assert!(
        temporaries(directory.path()).is_empty(),
        "{:?}",
        temporaries(directory.path())
    );
}

#[test]
fn a_local_failure_writes_nothing_and_keeps_the_old_snapshot() {
    let (directory, main, database) = temp_paths();
    write_config(&main, &database, 9, 30);
    let settings = load(&main);
    let snapshot_path = snapshot::path_for(&main);
    std::fs::write(&snapshot_path, b"the last configuration that worked\n").unwrap();
    let candidate = Candidate {
        source: directory.path().join("next.toml"),
        main: main.clone(),
        bytes: b"interval_ms = 25\n".to_vec(),
        stamp: 1_794_096_000_000,
    };

    let failure = ProbeError::of(
        Code::TcpSocket,
        &Details {
            reason: Some("operation not permitted"),
            ..Details::default()
        },
    );
    let probe = Arc::new(FakeProbe::scripted(vec![Err(failure)]));
    let (running, notifier) = start(
        settings.clone(),
        probe,
        &database,
        Start {
            promotion: Some(candidate),
            fallback: Some(settings),
            snapshot: Some(Snapshot {
                path: snapshot_path.clone(),
                bytes: b"the configuration that just failed\n".to_vec(),
            }),
            ..Start::default()
        },
        None,
    );

    wait_for_reload(&notifier, false, Duration::from_secs(10));
    running.stop();

    assert_eq!(
        std::fs::read(&snapshot_path).unwrap(),
        b"the last configuration that worked\n",
        "a failed first round leaves the previous snapshot alone"
    );
    assert!(!main
        .with_file_name("uptime-monitor.toml.20261108T000000Z.bak")
        .exists());
}

#[test]
fn an_accepted_or_rejected_reload_never_touches_the_snapshot() {
    let (_directory, main, database) = temp_paths();
    write_config(&main, &database, 9, 30);
    let settings = load(&main);
    let bytes = std::fs::read(&main).unwrap();
    let snapshot_path = snapshot::path_for(&main);
    let reload = Reload {
        path: main.clone(),
        overrides: Overrides::default(),
    };

    let probe = Arc::new(FakeProbe::always(up()));
    let (running, notifier) = start(
        settings,
        probe,
        &database,
        Start {
            snapshot: Some(Snapshot {
                path: snapshot_path.clone(),
                bytes: bytes.clone(),
            }),
            ..Start::default()
        },
        Some(reload),
    );

    wait_for_snapshot(&snapshot_path, Duration::from_secs(10));

    // Accepted: a valid edit is applied to the running workers, not the snapshot.
    write_config(&main, &database, 10, 30);
    wait_for_reload(&notifier, true, Duration::from_secs(10));
    assert_eq!(
        std::fs::read(&snapshot_path).unwrap(),
        bytes,
        "an accepted reload is not the start configuration"
    );

    // Rejected: a broken edit keeps the running configuration and the snapshot.
    std::fs::write(&main, "interval_ms = 30\nbroken = [\n").unwrap();
    wait_for_reload(&notifier, false, Duration::from_secs(10));
    assert_eq!(
        std::fs::read(&snapshot_path).unwrap(),
        bytes,
        "a rejected reload never rewrites the snapshot"
    );

    running.stop();
}
