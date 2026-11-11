//! Restoring a history: the `import` subcommand over a file, stdin, and a URL.
//!
//! The fixture format is the one the export writes, one row per event.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::thread;

use assert_cmd::Command;
use network_monitor::import;
use network_monitor::store::{Event, EventFilter, Store, TargetState};
use network_monitor::web::export::write_csv_gz;
use predicates::prelude::*;

const HEADER: &str = "target,label,interface,state,occurred_at,rtt_us\n";

fn transition(target: &str, previous: TargetState, state: TargetState, at: i64) -> Event {
    Event {
        target: target.to_string(),
        interface: Some("eth0".to_string()),
        label: Some("probe".to_string()),
        previous_state: previous,
        state,
        occurred_at_ms: at,
        rtt_us: if state == TargetState::Up {
            Some(12_000)
        } else {
            None
        },
    }
}

/// A database with two disconnections of one target, one of another, and a target that
/// only ever came up.
fn seeded(path: &Path) {
    let mut store = Store::open_rw(path).expect("the database opens");
    for event in [
        transition("1.1.1.1", TargetState::Unknown, TargetState::Up, 1_000),
        transition("1.1.1.1", TargetState::Up, TargetState::Down, 2_000),
        transition("1.1.1.1", TargetState::Down, TargetState::Up, 62_000),
        transition("8.8.8.8", TargetState::Unknown, TargetState::Down, 50_000),
        transition("9.9.9.9", TargetState::Unknown, TargetState::Up, 70_000),
    ] {
        store
            .record_transition(&event)
            .expect("the event is written");
    }
}

fn archive_of(path: &Path) -> Vec<u8> {
    let store = Store::open_ro(path).expect("the database opens");
    let mut bytes = Vec::new();
    write_csv_gz(&mut bytes, &store).expect("the export succeeds");
    bytes
}

/// Target, state, and instant of every event, for an order-insensitive comparison.
fn events_of(path: &Path) -> Vec<(String, String, i64)> {
    let store = Store::open_ro(path).expect("the database opens");
    let mut rows = store
        .events(&EventFilter::default())
        .expect("the history is read")
        .into_iter()
        .map(|row| {
            (
                row.target,
                row.state.as_str().to_string(),
                row.occurred_at_ms,
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

/// The chain of a database, in time order: instant, previous state, state.
fn chain_of(path: &Path) -> Vec<(i64, String, String)> {
    let store = Store::open_ro(path).expect("the database opens");
    let mut rows = store
        .events(&EventFilter::default())
        .expect("the history is read")
        .into_iter()
        .map(|row| {
            (
                row.occurred_at_ms,
                row.previous_state.as_str().to_string(),
                row.state.as_str().to_string(),
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

#[test]
fn a_gzipped_export_restores_the_same_history() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let original = directory.path().join("original.db");
    let restored = directory.path().join("restored.db");
    seeded(&original);
    let file = directory.path().join("history.csv.gz");
    std::fs::write(&file, archive_of(&original)).expect("the archive is written");

    let report = import::run(
        &restored,
        file.to_str().unwrap(),
        import::Options::default(),
    )
    .expect("the import succeeds");

    assert!(report.gzipped, "the format was recognised");
    assert_eq!(report.events, 5);
    assert_eq!(report.skipped, 0);
    assert_eq!(report.dropped, 0);
    assert_eq!(report.targets, 3);
    assert_eq!(
        events_of(&restored),
        events_of(&original),
        "every event comes back, the always-up target included"
    );
    assert_eq!(chain_of(&restored), chain_of(&original));

    let store = Store::open_ro(&restored).expect("the database opens");
    let target = store
        .find_target("1.1.1.1")
        .expect("the target is read")
        .expect("the target exists");
    assert_eq!(target.label.as_deref(), Some("probe"));
    assert_eq!(target.interface.as_deref(), Some("eth0"));
}

#[test]
fn a_plain_csv_needs_no_flag() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let restored = directory.path().join("restored.db");
    let file = directory.path().join("history.csv");
    std::fs::write(
        &file,
        format!(
            "{HEADER}\
             1.1.1.1,cloudflare,eth0,down,1970-01-01T00:00:02.000Z,\n\
             8.8.8.8,,,up,1970-01-01T00:00:50.000Z,12000\n"
        ),
    )
    .expect("the CSV is written");

    let report = import::run(
        &restored,
        file.to_str().unwrap(),
        import::Options::default(),
    )
    .expect("the import succeeds");

    assert!(!report.gzipped, "not compressed");
    assert_eq!(report.events, 2);
    assert_eq!(
        events_of(&restored),
        vec![
            ("1.1.1.1".to_string(), "down".to_string(), 2_000),
            ("8.8.8.8".to_string(), "up".to_string(), 50_000),
        ]
    );
}

#[test]
fn importing_the_same_archive_twice_changes_nothing() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let original = directory.path().join("original.db");
    let restored = directory.path().join("restored.db");
    seeded(&original);
    let file = directory.path().join("history.csv.gz");
    std::fs::write(&file, archive_of(&original)).expect("the archive is written");
    let path = file.to_str().unwrap();

    import::run(&restored, path, import::Options::default()).expect("the first import succeeds");
    let before = events_of(&restored);
    let second = import::run(&restored, path, import::Options::default())
        .expect("the second import succeeds");

    assert_eq!(second.events, 0, "nothing new");
    assert!(second.skipped > 0, "the repeated rows are reported");
    assert_eq!(events_of(&restored), before);
}

#[test]
fn a_broken_file_is_reported_without_touching_the_database() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let restored = directory.path().join("restored.db");
    let file = directory.path().join("broken.csv");
    std::fs::write(
        &file,
        format!("{HEADER}this,is,not,a,row,\n1.1.1.1,,,sideways,1970-01-01T00:00:01.000Z,\n"),
    )
    .expect("the file is written");

    let report = import::run(
        &restored,
        file.to_str().unwrap(),
        import::Options::default(),
    )
    .expect("the run finishes");

    assert_eq!(report.events, 0);
    assert_eq!(report.skipped, 2);
    assert!(events_of(&restored).is_empty());
}

#[test]
fn the_old_disconnection_format_is_refused() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let restored = directory.path().join("restored.db");
    let file = directory.path().join("old.csv");
    std::fs::write(
        &file,
        "target,started_at,ended_at,duration_seconds,ongoing\n\
         1.1.1.1,1970-01-01T00:00:02.000Z,1970-01-01T00:01:02.000Z,60,false\n",
    )
    .expect("the file is written");

    let error = import::run(
        &restored,
        file.to_str().unwrap(),
        import::Options::default(),
    )
    .expect_err("the old format is no longer imported");

    assert!(
        error.to_string().contains("old disconnection format"),
        "got {error}"
    );
}

#[test]
fn the_cli_reads_the_archive_from_standard_input() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let restored = directory.path().join("restored.db");

    Command::cargo_bin("network-monitor")
        .expect("the binary builds")
        .args(["import", "-", "--database", restored.to_str().unwrap()])
        .write_stdin(format!(
            "{HEADER}1.1.1.1,probe,eth0,up,1970-01-01T00:00:02.000Z,12000\n"
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "imported 1 events for 1 target(s)",
        ))
        .stdout(predicate::str::contains("plain csv"));

    assert_eq!(events_of(&restored).len(), 1);
}

#[test]
fn the_archive_can_be_downloaded_from_an_url() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let original = directory.path().join("original.db");
    let restored = directory.path().join("restored.db");
    seeded(&original);
    let payload = archive_of(&original);

    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port");
    let address = listener.local_addr().expect("the bound address");
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("one request");
        // Read the whole request: closing with unread input resets the connection.
        let mut request = [0u8; 2048];
        let mut received = 0;
        while received < request.len() {
            let read = stream.read(&mut request[received..]).expect("the request");
            if read == 0 {
                break;
            }
            received += read;
            if request[..received]
                .windows(4)
                .any(|window| window == b"\r\n\r\n")
            {
                break;
            }
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/gzip\r\n\
                  Transfer-Encoding: chunked\r\n\r\n",
            )
            .expect("the headers are written");
        for chunk in payload.chunks(97) {
            write!(stream, "{:x}\r\n", chunk.len()).expect("a chunk header");
            stream.write_all(chunk).expect("the chunk is written");
            stream.write_all(b"\r\n").expect("the chunk ends");
        }
        stream.write_all(b"0\r\n\r\n").expect("the body ends");
        stream.flush().expect("the response is flushed");
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut rest = [0u8; 64];
        while let Ok(read) = stream.read(&mut rest) {
            if read == 0 {
                break;
            }
        }
    });

    let url = format!("http://{address}/export/disconnections.csv.gz");
    let report =
        import::run(&restored, &url, import::Options::default()).expect("the import succeeds");
    server.join().expect("the server finishes");

    assert!(report.gzipped);
    assert_eq!(events_of(&restored), events_of(&original));
}

#[test]
fn https_is_refused_with_an_explanation() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let restored = directory.path().join("restored.db");

    let error = import::run(
        &restored,
        "https://nas:8642/export/disconnections.csv.gz",
        import::Options::default(),
    )
    .expect_err("no TLS support");

    assert!(
        error
            .to_string()
            .contains("only http:// URLs are supported"),
        "got {error}"
    );
}

/// A database with history before, inside, and after the archived window.
fn seeded_with_surrounding_history(path: &Path) {
    let mut store = Store::open_rw(path).expect("the database opens");
    for event in [
        transition("1.1.1.1", TargetState::Unknown, TargetState::Up, 1_000),
        transition("1.1.1.1", TargetState::Up, TargetState::Down, 5_000),
        transition("1.1.1.1", TargetState::Down, TargetState::Up, 6_000),
        transition("1.1.1.1", TargetState::Up, TargetState::Down, 100_000),
        transition("1.1.1.1", TargetState::Down, TargetState::Up, 160_000),
        transition("1.1.1.1", TargetState::Up, TargetState::Down, 900_000),
        transition("1.1.1.1", TargetState::Down, TargetState::Up, 960_000),
        transition("8.8.8.8", TargetState::Unknown, TargetState::Down, 700_000),
    ] {
        store
            .record_transition(&event)
            .expect("the event is written");
    }
}

#[test]
fn replacing_a_window_keeps_the_history_outside_it() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let target = directory.path().join("events.db");
    seeded_with_surrounding_history(&target);
    // The archive rewrites the middle episode: it ends at 00:03:20 instead of 00:02:40.
    let file = directory.path().join("window.csv");
    std::fs::write(
        &file,
        format!(
            "{HEADER}\
             1.1.1.1,probe,eth0,down,1970-01-01T00:01:40.000Z,\n\
             1.1.1.1,probe,eth0,up,1970-01-01T00:03:20.000Z,12000\n"
        ),
    )
    .expect("the CSV is written");

    let report = import::run(
        &target,
        file.to_str().unwrap(),
        import::Options {
            replace_window: true,
        },
    )
    .expect("the import succeeds");

    assert_eq!(report.window, Some((100_000, 200_000)));
    assert_eq!(report.deleted, 2, "the stored down/up of that window");
    assert_eq!(report.events, 2);
    assert_eq!(
        events_of(&target),
        vec![
            ("1.1.1.1".to_string(), "down".to_string(), 5_000),
            ("1.1.1.1".to_string(), "down".to_string(), 100_000),
            ("1.1.1.1".to_string(), "down".to_string(), 900_000),
            ("1.1.1.1".to_string(), "up".to_string(), 1_000),
            ("1.1.1.1".to_string(), "up".to_string(), 6_000),
            ("1.1.1.1".to_string(), "up".to_string(), 200_000),
            ("1.1.1.1".to_string(), "up".to_string(), 960_000),
            ("8.8.8.8".to_string(), "down".to_string(), 700_000),
        ],
        "the events before and after the window are untouched"
    );
    assert_eq!(
        chain_of(&target)[1],
        (5_000, "up".to_string(), "down".to_string()),
        "the seam knows the state it replaced"
    );
}

#[test]
fn replacing_a_window_leaves_the_targets_outside_the_archive_alone() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let target = directory.path().join("events.db");
    let mut store = Store::open_rw(&target).expect("the database opens");
    for event in [
        transition("1.1.1.1", TargetState::Unknown, TargetState::Down, 100_000),
        transition("1.1.1.1", TargetState::Down, TargetState::Up, 160_000),
    ] {
        store
            .record_transition(&event)
            .expect("the event is written");
    }
    drop(store);
    let file = directory.path().join("window.csv");
    std::fs::write(
        &file,
        format!(
            "{HEADER}\
             9.9.9.9,new,eth1,down,1970-01-01T00:01:40.000Z,\n\
             9.9.9.9,new,eth1,up,1970-01-01T00:02:40.000Z,9000\n"
        ),
    )
    .expect("the CSV is written");

    import::run(
        &target,
        file.to_str().unwrap(),
        import::Options {
            replace_window: true,
        },
    )
    .expect("the import succeeds");

    assert_eq!(
        events_of(&target),
        vec![
            ("1.1.1.1".to_string(), "down".to_string(), 100_000),
            ("1.1.1.1".to_string(), "up".to_string(), 160_000),
            ("9.9.9.9".to_string(), "down".to_string(), 100_000),
            ("9.9.9.9".to_string(), "up".to_string(), 160_000),
        ],
        "the archive rewrites the history of 9.9.9.9 and says nothing about 1.1.1.1"
    );
}

#[test]
fn replacing_a_window_gives_every_target_its_own_window() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let target = directory.path().join("events.db");
    let mut store = Store::open_rw(&target).expect("the database opens");
    for event in [
        // The target the archive rewrites over a long span.
        transition("1.1.1.1", TargetState::Unknown, TargetState::Up, 90_000),
        transition("1.1.1.1", TargetState::Up, TargetState::Down, 150_000),
        // The target whose part of the archive stops early: its events after 00:02:00
        // are outside the archive and must survive.
        transition("8.8.8.8", TargetState::Unknown, TargetState::Down, 100_000),
        transition("8.8.8.8", TargetState::Down, TargetState::Up, 130_000),
        transition("8.8.8.8", TargetState::Up, TargetState::Down, 180_000),
    ] {
        store
            .record_transition(&event)
            .expect("the event is written");
    }
    drop(store);

    let file = directory.path().join("window.csv");
    std::fs::write(
        &file,
        format!(
            "{HEADER}\
             1.1.1.1,probe,eth0,up,1970-01-01T00:01:40.000Z,9000\n\
             1.1.1.1,probe,eth0,down,1970-01-01T00:03:20.000Z,\n\
             8.8.8.8,probe,eth0,down,1970-01-01T00:01:40.000Z,\n"
        ),
    )
    .expect("the CSV is written");

    let report = import::run(
        &target,
        file.to_str().unwrap(),
        import::Options {
            replace_window: true,
        },
    )
    .expect("the import succeeds");

    assert_eq!(
        report.window,
        Some((100_000, 200_000)),
        "the line still names the span that holds every window"
    );
    assert_eq!(
        report.deleted, 2,
        "the down of 1.1.1.1 and the first down of 8.8.8.8, each inside its own window"
    );
    assert_eq!(
        events_of(&target),
        vec![
            ("1.1.1.1".to_string(), "down".to_string(), 200_000),
            ("1.1.1.1".to_string(), "up".to_string(), 90_000),
            ("8.8.8.8".to_string(), "down".to_string(), 100_000),
            ("8.8.8.8".to_string(), "down".to_string(), 180_000),
            ("8.8.8.8".to_string(), "up".to_string(), 130_000),
        ],
        "8.8.8.8 keeps what its own part of the archive never covered"
    );
}

#[test]
fn a_replacement_over_stdin_works_by_spooling() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let target = directory.path().join("events.db");
    let mut store = Store::open_rw(&target).expect("the database opens");
    store
        .record_transition(&transition(
            "1.1.1.1",
            TargetState::Unknown,
            TargetState::Up,
            1_000,
        ))
        .expect("the event is written");
    drop(store);

    Command::cargo_bin("network-monitor")
        .expect("the binary builds")
        .args([
            "import",
            "-",
            "--replace-window",
            "--database",
            target.to_str().unwrap(),
        ])
        .write_stdin(format!(
            "{HEADER}1.1.1.1,probe,eth0,down,2026-11-08T00:00:02.000Z,\n\
             1.1.1.1,probe,eth0,up,2026-11-08T00:01:02.000Z,12000\n"
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("replaced"));

    let store = Store::open_ro(&target).expect("the database opens");
    assert!(
        store
            .find_target("1.1.1.1")
            .expect("the target is read")
            .is_some(),
        "the imported target is there"
    );
    let leftovers = std::fs::read_dir(directory.path())
        .expect("the directory is readable")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(".import-"))
        .count();
    assert_eq!(leftovers, 0, "the spooled copy is removed");
}

#[test]
fn an_empty_archive_cannot_replace_a_window() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let target = directory.path().join("events.db");
    seeded_with_surrounding_history(&target);
    let file = directory.path().join("empty.csv");
    std::fs::write(&file, HEADER).expect("the CSV is written");

    let error = import::run(
        &target,
        file.to_str().unwrap(),
        import::Options {
            replace_window: true,
        },
    )
    .expect_err("nothing to replace");

    assert!(error.to_string().contains("no window"), "got {error}");
    assert_eq!(events_of(&target).len(), 8, "nothing was deleted");
}

#[test]
fn rows_in_any_order_produce_the_same_history() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let ordered = directory.path().join("ordered.csv");
    let shuffled = directory.path().join("shuffled.csv");
    std::fs::write(
        &ordered,
        format!(
            "{HEADER}\
             1.1.1.1,,,down,1970-01-01T00:00:10.000Z,\n\
             1.1.1.1,,,up,1970-01-01T00:00:20.000Z,12000\n\
             1.1.1.1,,,down,1970-01-01T00:00:30.000Z,\n\
             8.8.8.8,,,down,1970-01-01T00:00:15.000Z,\n"
        ),
    )
    .expect("the CSV is written");
    // The same events, newest first and targets interleaved.
    std::fs::write(
        &shuffled,
        format!(
            "{HEADER}\
             1.1.1.1,,,down,1970-01-01T00:00:30.000Z,\n\
             8.8.8.8,,,down,1970-01-01T00:00:15.000Z,\n\
             1.1.1.1,,,up,1970-01-01T00:00:20.000Z,12000\n\
             1.1.1.1,,,down,1970-01-01T00:00:10.000Z,\n"
        ),
    )
    .expect("the CSV is written");

    let first = directory.path().join("first.db");
    let second = directory.path().join("second.db");
    import::run(
        &first,
        ordered.to_str().unwrap(),
        import::Options::default(),
    )
    .expect("the ordered import succeeds");
    let report = import::run(
        &second,
        shuffled.to_str().unwrap(),
        import::Options::default(),
    )
    .expect("the shuffled import succeeds");

    assert_eq!(report.skipped, 0, "no row is rejected because of its place");
    assert_eq!(report.dropped, 0);
    assert_eq!(chain_of(&second), chain_of(&first));
}

#[test]
fn every_event_knows_the_state_it_replaced() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let database = directory.path().join("events.db");
    let file = directory.path().join("history.csv");
    std::fs::write(
        &file,
        format!(
            "{HEADER}\
             1.1.1.1,,,up,1970-01-01T00:00:20.000Z,12000\n\
             1.1.1.1,,,down,1970-01-01T00:00:10.000Z,\n"
        ),
    )
    .expect("the CSV is written");

    import::run(
        &database,
        file.to_str().unwrap(),
        import::Options::default(),
    )
    .expect("the import succeeds");

    assert_eq!(
        chain_of(&database),
        vec![
            (10_000, "unknown".to_string(), "down".to_string()),
            (20_000, "down".to_string(), "up".to_string()),
        ]
    );
}

#[test]
fn an_event_repeating_the_state_before_it_is_dropped() {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let database = directory.path().join("events.db");
    let file = directory.path().join("repeat.csv");
    std::fs::write(
        &file,
        format!(
            "{HEADER}\
             1.1.1.1,,,down,1970-01-01T00:00:10.000Z,\n\
             1.1.1.1,,,down,1970-01-01T00:00:20.000Z,\n\
             1.1.1.1,,,up,1970-01-01T00:00:30.000Z,12000\n"
        ),
    )
    .expect("the CSV is written");

    let report = import::run(
        &database,
        file.to_str().unwrap(),
        import::Options::default(),
    )
    .expect("the import succeeds");

    assert_eq!(report.dropped, 1, "the second down is redundant");
    assert_eq!(
        chain_of(&database),
        vec![
            (10_000, "unknown".to_string(), "down".to_string()),
            (30_000, "down".to_string(), "up".to_string()),
        ],
        "no two consecutive events share a state"
    );
}
