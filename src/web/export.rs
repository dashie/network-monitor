//! Streamed export of the disconnection history.
//!
//! The response is chunked and the CSV is compressed while it is sent, one page of
//! events at a time: the server never holds the file in memory, and every read
//! transaction stays short.
//!
//! The format is a plain gzipped CSV, not an archive: gzip is a single stream with
//! its header in front and the checksum and length in a trailer, so nothing has to be
//! known before the first byte goes out.

use std::io::{self, Write};

use flate2::write::GzEncoder;
use flate2::Compression;
use thiserror::Error;

use crate::store::{DownEpisode, Store, StoreError};
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

/// Stream every disconnection, oldest first, as a gzipped CSV.
pub fn write_csv_gz<W: Write>(
    out: W,
    store: &Store,
    exported_at_ms: i64,
) -> Result<(), ExportError> {
    let mut gzip = GzEncoder::new(out, Compression::fast());
    gzip.write_all(header().as_bytes())?;

    let mut after_id = i64::MIN;
    loop {
        let page = store.downs_after(after_id, PAGE_SIZE)?;
        if page.is_empty() {
            break;
        }
        for episode in &page {
            gzip.write_all(row(episode, exported_at_ms).as_bytes())?;
        }
        after_id = page[page.len() - 1].event_id;
    }

    gzip.finish()?;
    Ok(())
}

/// The header of the CSV file.
fn header() -> String {
    "target,started_at,ended_at,duration_seconds,ongoing\n".to_string()
}

/// One CSV row: ISO 8601 timestamps, whole seconds, and whether the target is still
/// down — in which case the duration runs to the moment of the export.
fn row(episode: &DownEpisode, exported_at_ms: i64) -> String {
    let (ended, duration) = match episode.ended_at_ms {
        Some(end) => (
            format_rfc3339(end),
            ((end - episode.started_at_ms) / 1_000).max(0),
        ),
        None => (
            String::new(),
            ((exported_at_ms - episode.started_at_ms) / 1_000).max(0),
        ),
    };

    format!(
        "{},{},{},{},{}\n",
        field(&episode.target),
        format_rfc3339(episode.started_at_ms),
        ended,
        duration,
        episode.ended_at_ms.is_none()
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
    use crate::store::{Event, TargetState};
    use flate2::read::GzDecoder;
    use std::io::Read;

    fn episode(started_at_ms: i64, ended_at_ms: Option<i64>) -> DownEpisode {
        DownEpisode {
            event_id: 1,
            target: "1.1.1.1".to_string(),
            started_at_ms,
            ended_at_ms,
        }
    }

    fn transition(previous: TargetState, state: TargetState, at: i64) -> Event {
        Event {
            target: "1.1.1.1".to_string(),
            interface: None,
            label: None,
            previous_state: previous,
            state,
            occurred_at_ms: at,
            rtt_us: None,
        }
    }

    #[test]
    fn a_closed_episode_states_both_times_and_the_duration() {
        let row = row(&episode(0, Some(90_000)), 500_000);
        assert_eq!(
            row,
            "1.1.1.1,1970-01-01T00:00:00.000Z,1970-01-01T00:01:30.000Z,90,false\n"
        );
    }

    #[test]
    fn an_open_episode_runs_to_the_moment_of_the_export() {
        let row = row(&episode(0, None), 120_000);
        assert!(row.contains(",,120,true"), "got {row}");
    }

    #[test]
    fn a_hostile_target_is_quoted() {
        assert_eq!(field("plain"), "plain");
        assert_eq!(field("a,b"), "\"a,b\"");
        assert_eq!(field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn the_payload_is_readable_by_a_gzip_reader() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = Store::open_rw(&directory.path().join("events.db")).unwrap();
        store
            .record_transition(&transition(TargetState::Unknown, TargetState::Down, 1_000))
            .unwrap();
        store
            .record_transition(&transition(TargetState::Down, TargetState::Up, 61_000))
            .unwrap();

        let mut buffer = Vec::new();
        write_csv_gz(&mut buffer, &store, 100_000).unwrap();
        assert_eq!(&buffer[..2], &[0x1f, 0x8b], "the gzip magic number");

        let mut csv = String::new();
        GzDecoder::new(&buffer[..])
            .read_to_string(&mut csv)
            .expect("the stream decompresses");

        assert!(csv.starts_with("target,started_at,ended_at,duration_seconds,ongoing\n"));
        assert!(csv.contains("1.1.1.1,1970-01-01T00:00:01.000Z,1970-01-01T00:01:01.000Z,60,false"));
    }
}
