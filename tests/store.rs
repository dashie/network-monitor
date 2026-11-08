//! INV-4: a reader never blocks the writer and never sees a broken state.
//!
//! The unit tests in `src/store` cover the schema; this test exercises the
//! concurrent access the query subcommand relies on.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use network_monitor::store::{Event, EventFilter, Store, TargetState};

fn event(index: i64) -> Event {
    Event {
        target: "1.1.1.1".to_string(),
        interface: Some("eth1".to_string()),
        label: None,
        previous_state: if index % 2 == 0 {
            TargetState::Up
        } else {
            TargetState::Down
        },
        state: if index % 2 == 0 {
            TargetState::Down
        } else {
            TargetState::Up
        },
        occurred_at_ms: index,
        rtt_us: None,
    }
}

#[test]
fn reads_succeed_while_the_writer_runs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("events.db");

    let mut writer = Store::open_rw(&path).unwrap();
    writer.record_transition(&event(0)).unwrap();

    let stop = Arc::new(AtomicBool::new(false));

    let reader_path = path.clone();
    let reader_stop = Arc::clone(&stop);
    let reader = thread::spawn(move || {
        let store = Store::open_ro(&reader_path).unwrap();
        let mut reads = 0_u32;
        while !reader_stop.load(Ordering::SeqCst) {
            let events = store.events(&EventFilter::default()).unwrap();
            let states = store.states().unwrap();
            assert!(!events.is_empty());
            assert_eq!(states.len(), 1);
            reads += 1;
        }
        reads
    });

    for index in 1..200 {
        writer.record_transition(&event(index)).unwrap();
    }

    stop.store(true, Ordering::SeqCst);
    let reads = reader.join().unwrap();
    assert!(reads > 0, "the reader ran at least once");
}

#[test]
fn the_target_row_matches_the_last_event() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();

    for index in 0..10 {
        store.record_transition(&event(index)).unwrap();

        let events = store.events(&EventFilter::default()).unwrap();
        let states = store.states().unwrap();
        assert_eq!(states[0].state, events[0].state, "after event {index}");
        assert_eq!(states[0].last_change_at_ms, events[0].occurred_at_ms);
    }
}

#[test]
fn labels_resolve_to_their_target() {
    let directory = tempfile::tempdir().unwrap();
    let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();

    let mut first = event(0);
    first.label = Some("cloudflare-dns".to_string());
    store.record_transition(&first).unwrap();

    assert_eq!(
        store.resolve_target("cloudflare-dns").unwrap().as_deref(),
        Some("1.1.1.1")
    );
    assert_eq!(
        store.resolve_target("1.1.1.1").unwrap().as_deref(),
        Some("1.1.1.1")
    );
    assert!(store.resolve_target("unknown").unwrap().is_none());
}
