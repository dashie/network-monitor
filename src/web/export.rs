//! Streamed export of the whole history.
//!
//! The response is chunked and the CSV is compressed while it is sent, one page of
//! events at a time: the server never holds the file in memory, and every read
//! transaction stays short.
//!
//! The format is a plain gzipped CSV, not an archive: gzip is a single stream with
//! its header in front and the checksum and length in a trailer, so nothing has to be
//! known before the first byte goes out.
//!
//! One row per event, with the same columns the import reads: an exported file can be
//! imported back as it is.

use std::io::{self, Write};

use flate2::write::GzEncoder;
use flate2::Compression;
use thiserror::Error;

use crate::store::{ExportedEvent, Store, StoreError};
use crate::time::{format_rfc3339, now_ms};

/// Events read from the database per page of the export.
const PAGE_SIZE: u32 = 1_000;

#[derive(Debug, Error)]
pub enum ExportError {
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("cannot write the export: {0}")]
    Io(#[from] io::Error),
}

/// The moment the export started, so one response is consistent with itself.
pub fn exported_at() -> i64 {
    now_ms()
}

/// Stream every event, oldest first, as a gzipped CSV.
pub fn write_csv_gz<W: Write>(out: W, store: &Store) -> Result<(), ExportError> {
    let mut gzip = GzEncoder::new(out, Compression::fast());
    gzip.write_all(header().as_bytes())?;

    let mut after_id = i64::MIN;
    loop {
        let page = store.events_after(after_id, PAGE_SIZE)?;
        if page.is_empty() {
            break;
        }
        for event in &page {
            gzip.write_all(row(event).as_bytes())?;
        }
        after_id = page[page.len() - 1].id;
    }

    gzip.finish()?;
    Ok(())
}

/// The header of the CSV file. The import reads exactly these columns.
fn header() -> String {
    "target,label,interface,state,occurred_at,rtt_us\n".to_string()
}

/// One CSV row: the identity of the target, what happened, when, and the round-trip
/// time when the event is a return to `up`.
fn row(event: &ExportedEvent) -> String {
    format!(
        "{},{},{},{},{},{}\n",
        field(&event.target),
        field(event.label.as_deref().unwrap_or_default()),
        field(event.interface.as_deref().unwrap_or_default()),
        event.state.as_str(),
        format_rfc3339(event.occurred_at_ms),
        event
            .rtt_us
            .map(|rtt_us| rtt_us.to_string())
            .unwrap_or_default()
    )
}

/// Quote a CSV field when it contains a comma, a quote, or a line break.
fn field(value: &str) -> String {
    if !value.contains([',', '"', '\n', '\r']) {
        return value.to_string();
    }
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('"');
    for character in value.chars() {
        if character == '"' {
            quoted.push('"');
        }
        quoted.push(character);
    }
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Event, Store, TargetState};
    use flate2::read::GzDecoder;
    use std::io::Read;

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

    fn archive_of(store: &Store) -> String {
        let mut buffer = Vec::new();
        write_csv_gz(&mut buffer, store).unwrap();
        assert_eq!(&buffer[..2], &[0x1f, 0x8b], "the gzip magic number");

        let mut csv = String::new();
        GzDecoder::new(&buffer[..])
            .read_to_string(&mut csv)
            .expect("the stream decompresses");
        csv
    }

    #[test]
    fn a_row_states_the_target_what_happened_and_when() {
        let event = ExportedEvent {
            id: 1,
            target: "1.1.1.1".to_string(),
            label: Some("cloudflare".to_string()),
            interface: Some("eth0".to_string()),
            state: TargetState::Down,
            occurred_at_ms: 90_000,
            rtt_us: None,
        };
        assert_eq!(
            row(&event),
            "1.1.1.1,cloudflare,eth0,down,1970-01-01T00:01:30.000Z,\n"
        );
    }

    #[test]
    fn a_return_to_up_carries_the_round_trip_time() {
        let event = ExportedEvent {
            id: 2,
            target: "1.1.1.1".to_string(),
            label: None,
            interface: None,
            state: TargetState::Up,
            occurred_at_ms: 90_000,
            rtt_us: Some(12_500),
        };
        assert_eq!(row(&event), "1.1.1.1,,,up,1970-01-01T00:01:30.000Z,12500\n");
    }

    #[test]
    fn a_hostile_label_is_quoted() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn every_event_is_exported_in_order() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        for event in [
            transition("1.1.1.1", TargetState::Unknown, TargetState::Up, 1_000),
            transition("1.1.1.1", TargetState::Up, TargetState::Down, 2_000),
            transition("1.1.1.1", TargetState::Down, TargetState::Up, 3_000),
            transition("8.8.8.8", TargetState::Unknown, TargetState::Up, 2_500),
        ] {
            store.record_transition(&event).unwrap();
        }

        let csv = archive_of(&store);
        let lines: Vec<&str> = csv.lines().collect();

        assert_eq!(lines[0], "target,label,interface,state,occurred_at,rtt_us");
        assert_eq!(
            lines[1],
            "1.1.1.1,probe,eth0,up,1970-01-01T00:00:01.000Z,12000"
        );
        assert_eq!(
            lines[2],
            "1.1.1.1,probe,eth0,down,1970-01-01T00:00:02.000Z,"
        );
        assert_eq!(
            lines[3],
            "1.1.1.1,probe,eth0,up,1970-01-01T00:00:03.000Z,12000"
        );
        assert_eq!(
            lines[4],
            "8.8.8.8,probe,eth0,up,1970-01-01T00:00:02.500Z,12000"
        );
        assert_eq!(lines.len(), 5, "every event is a row");
    }
}
