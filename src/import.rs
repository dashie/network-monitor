//! The `import` subcommand: restore the history from an exported CSV.
//!
//! The input is the file the web page offers for download — one row per event, plain
//! or gzipped, recognised from its first two bytes — read from a path, an `http://`
//! URL, or standard input. The columns are the ones the export writes, so the two
//! commands share one format.
//!
//! The rows may arrive in any order: the chain is rebuilt from the timestamps at the
//! end, and an event that repeats the state before it is dropped (BR-1).

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flate2::read::GzDecoder;
use thiserror::Error;

use crate::store::{Event, Store, StoreError, TargetState};
use crate::time::parse_rfc3339;

/// Events handed to the store in one transaction.
const BATCH: usize = 500;
/// The first line of a file this tool writes.
const HEADER: &str = "target,label,interface,state,occurred_at,rtt_us";
/// The header of the disconnection-only format this tool used to write.
const LEGACY_HEADER: &str = "target,started_at,ended_at,duration_seconds,ongoing";

#[derive(Debug, Error)]
pub enum ImportError {
    #[error("{0}")]
    Store(#[from] StoreError),
    #[error("cannot read the archive: {0}")]
    Io(#[from] io::Error),
    #[error("cannot download the archive: {0}")]
    Http(String),
    #[error("cannot read '{origin}': {reason}")]
    Source { origin: String, reason: String },
    #[error("the archive holds no row, so there is no window to replace")]
    EmptyArchive,
    #[error(
        "this file uses the old disconnection format, which is no longer imported; \
         export the history again from the page"
    )]
    LegacyFormat,
}

/// What the command did, for the line it prints on stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub source: String,
    pub events: u32,
    pub skipped: u32,
    pub targets: usize,
    pub gzipped: bool,
    /// Events deleted before writing, when the window was replaced.
    pub deleted: u32,
    /// The window the archive covers, when the window was replaced.
    pub window: Option<(i64, i64)>,
    /// Events dropped because they repeated the state before them.
    pub dropped: u32,
}

impl Report {
    fn new(source: &str, gzipped: bool) -> Self {
        Self {
            source: source.to_string(),
            events: 0,
            skipped: 0,
            targets: 0,
            gzipped,
            deleted: 0,
            window: None,
            dropped: 0,
        }
    }
}

/// How the archive is applied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Delete every event inside the window the archive covers, then write the archive.
    /// Events before and after the window are kept.
    pub replace_window: bool,
}

/// Where the archive comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Stdin,
    Path(PathBuf),
    Url(String),
}

/// A dash is standard input, a scheme is a URL, anything else is a path.
pub fn parse_source(text: &str) -> Source {
    let trimmed = text.trim();
    if trimmed == "-" {
        return Source::Stdin;
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Source::Url(trimmed.to_string());
    }
    Source::Path(PathBuf::from(trimmed))
}

/// Read the archive into the database, one batch of events at a time.
pub fn run(database: &Path, source_text: &str, options: Options) -> Result<Report, ImportError> {
    let source = parse_source(source_text);
    let mut spool = Spool::default();

    // Replacing needs the window before the first write: read the archive once, delete
    // what it covers, then apply it in a second pass over the same bytes.
    if options.replace_window {
        let reopenable = spool.reopenable(database, &source)?;
        let window = coverage(&reopenable)?;

        let mut store = Store::open_rw(database)?;
        let mut report = Report::new(source_text, false);
        report.deleted = store.delete_events_between(window.0, window.1)? as u32;
        report.window = Some(window);

        let (reader, gzipped) = open(&reopenable)?;
        report.gzipped = gzipped;
        apply(reader, &mut store, &mut report)?;
        return Ok(report);
    }

    let mut store = Store::open_rw(database)?;
    let (reader, gzipped) = open(&source)?;
    let mut report = Report::new(source_text, gzipped);
    apply(reader, &mut store, &mut report)?;
    Ok(report)
}

/// A copy of the archive that can be read twice, for the replace mode.
///
/// A path is already re-readable. Standard input and an URL are copied once, into a
/// temporary file next to the database, because the second pass has to see exactly the
/// same bytes — an exported URL is regenerated on every request.
#[derive(Debug, Default)]
struct Spool {
    path: Option<PathBuf>,
}

impl Spool {
    fn reopenable(&mut self, database: &Path, source: &Source) -> Result<Source, ImportError> {
        match source {
            Source::Path(path) => Ok(Source::Path(path.clone())),
            Source::Stdin | Source::Url(_) => {
                let directory = database.parent().unwrap_or_else(|| Path::new("."));
                let path = directory.join(format!(".import-{}.tmp", std::process::id()));
                let (mut reader, _) = open(source)?;
                let mut file = File::create(&path)?;
                io::copy(&mut reader, &mut file)?;
                file.flush()?;
                self.path = Some(path.clone());
                Ok(Source::Path(path))
            }
        }
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        if let Some(path) = self.path.take() {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// The window an archive covers: the earliest start and the latest end it mentions.
fn coverage(source: &Source) -> Result<(i64, i64), ImportError> {
    let (reader, _) = open(source)?;
    let mut first: Option<i64> = None;
    let mut last: Option<i64> = None;

    for line in reader.lines() {
        let line = line?;
        let line = line.trim_end_matches('\r');
        if line.starts_with(LEGACY_HEADER) {
            return Err(ImportError::LegacyFormat);
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with(HEADER) {
            continue;
        }
        let row = match Row::parse(line) {
            Some(row) => row,
            None => continue,
        };
        first = Some(first.map_or(row.occurred_at_ms, |earliest| {
            earliest.min(row.occurred_at_ms)
        }));
        last = Some(last.map_or(row.occurred_at_ms, |latest| latest.max(row.occurred_at_ms)));
    }

    match (first, last) {
        (Some(first), Some(last)) => Ok((first, last)),
        _ => Err(ImportError::EmptyArchive),
    }
}

/// Write every row of the archive, in batches, and update the counters.
fn apply(
    reader: Box<dyn BufRead>,
    store: &mut Store,
    report: &mut Report,
) -> Result<(), ImportError> {
    let mut targets: HashSet<String> = HashSet::new();
    let mut batch: Vec<Event> = Vec::with_capacity(BATCH);

    for line in reader.lines() {
        let line = line?;
        let line = line.trim_end_matches('\r');
        if line.starts_with(LEGACY_HEADER) {
            return Err(ImportError::LegacyFormat);
        }
        if line.is_empty() || line.starts_with('#') || line.starts_with(HEADER) {
            continue;
        }

        let row = match Row::parse(line) {
            Some(row) => row,
            None => {
                report.skipped += 1;
                continue;
            }
        };

        targets.insert(row.target.clone());
        batch.push(row.event());

        if batch.len() >= BATCH {
            report.events += flush(store, &mut batch, &mut report.skipped)?;
        }
    }

    if !batch.is_empty() {
        report.events += flush(store, &mut batch, &mut report.skipped)?;
    }

    report.dropped = store.normalize_history()?;
    store.rebuild_from_events()?;
    report.targets = targets.len();
    Ok(())
}

/// Write one batch and count what the store refused because it was already there.
fn flush(store: &mut Store, batch: &mut Vec<Event>, skipped: &mut u32) -> Result<u32, ImportError> {
    let attempted = batch.len() as u32;
    let inserted = store.import_events(batch)?;
    *skipped += attempted - inserted;
    batch.clear();
    Ok(inserted)
}

/// Open the source and wrap it in a gzip decoder when it looks gzipped.
fn open(source: &Source) -> Result<(Box<dyn BufRead>, bool), ImportError> {
    let raw: Box<dyn Read> = match source {
        Source::Stdin => Box::new(io::stdin()),
        Source::Path(path) => Box::new(File::open(path).map_err(|error| ImportError::Source {
            origin: path.display().to_string(),
            reason: error.to_string(),
        })?),
        Source::Url(url) => Box::new(fetch(url)?),
    };

    let mut buffered = BufReader::new(raw);
    // Peeking two bytes costs nothing and removes the need for a format flag.
    let gzipped = matches!(buffered.fill_buf()?, [0x1f, 0x8b, ..]);
    if gzipped {
        return Ok((Box::new(BufReader::new(GzDecoder::new(buffered))), true));
    }
    Ok((Box::new(buffered), false))
}

/// One line of the export: the target, what happened, when, and the round-trip time.
struct Row {
    target: String,
    label: Option<String>,
    interface: Option<String>,
    state: TargetState,
    occurred_at_ms: i64,
    rtt_us: Option<i64>,
}

impl Row {
    /// `target,label,interface,state,occurred_at,rtt_us`; the last field may be empty.
    fn parse(line: &str) -> Option<Self> {
        let fields = split_csv(line);
        if fields.len() < 6 {
            return None;
        }

        let target = fields[0].trim().to_string();
        if target.is_empty() {
            return None;
        }

        let state = TargetState::parse(fields[3].trim())?;
        if state == TargetState::Unknown {
            return None;
        }

        let rtt_us = match fields[5].trim() {
            "" => None,
            value => Some(value.parse().ok()?),
        };

        Some(Self {
            target,
            label: optional(fields[1].trim()),
            interface: optional(fields[2].trim()),
            state,
            occurred_at_ms: parse_rfc3339(fields[4].trim())?,
            rtt_us,
        })
    }

    fn event(&self) -> Event {
        Event {
            target: self.target.clone(),
            interface: self.interface.clone(),
            label: self.label.clone(),
            // The real predecessor is worked out from the timestamps after the writes.
            previous_state: TargetState::Unknown,
            state: self.state,
            occurred_at_ms: self.occurred_at_ms,
            rtt_us: self.rtt_us,
        }
    }
}

fn optional(value: &str) -> Option<String> {
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

/// Split one CSV line, honouring double quotes around a field and `""` inside them.
fn split_csv(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut characters = line.chars().peekable();

    while let Some(character) = characters.next() {
        match character {
            '"' if quoted => {
                if characters.peek() == Some(&'"') {
                    current.push('"');
                    characters.next();
                } else {
                    quoted = false;
                }
            }
            '"' if current.is_empty() => quoted = true,
            ',' if !quoted => fields.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    fields.push(current);
    fields
}

/// Download the archive over plain HTTP/1.1.
fn fetch(url: &str) -> Result<HttpBody, ImportError> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| ImportError::Source {
            origin: url.to_string(),
            reason: "only http:// URLs are supported; download the file and pass its path, \
                     or read it from stdin"
                .to_string(),
        })?;

    let (authority, path) = match rest.find('/') {
        Some(at) => (&rest[..at], &rest[at..]),
        None => (rest, "/"),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((host, port)) => (
            host,
            port.parse::<u16>().map_err(|_| ImportError::Source {
                origin: url.to_string(),
                reason: format!("'{port}' is not a port"),
            })?,
        ),
        None => (authority, 80),
    };
    if host.is_empty() {
        return Err(ImportError::Source {
            origin: url.to_string(),
            reason: "the URL has no host".to_string(),
        });
    }

    let stream = TcpStream::connect((host, port))
        .map_err(|error| ImportError::Http(format!("cannot reach {host}:{port}: {error}")))?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    let mut request = stream.try_clone()?;
    write!(
        request,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nAccept-Encoding: identity\r\n\
         User-Agent: network-monitor\r\nConnection: close\r\n\r\n"
    )?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);

    let mut chunked = false;
    let mut length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 {
            break;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':') {
            let value = value.trim();
            match name.trim().to_ascii_lowercase().as_str() {
                "transfer-encoding" if value.to_ascii_lowercase().contains("chunked") => {
                    chunked = true
                }
                "content-length" => length = value.parse::<u64>().ok(),
                _ => {}
            }
        }
    }

    if status != 200 {
        return Err(ImportError::Http(format!(
            "{url} answered {status}, expected 200"
        )));
    }

    let mode = if chunked {
        Mode::Chunked { remaining: 0 }
    } else {
        match length {
            Some(remaining) => Mode::Length(remaining),
            None => Mode::UntilEof,
        }
    };
    Ok(HttpBody { reader, mode })
}

enum Mode {
    /// `remaining` bytes of the current chunk, `0` means "a chunk header comes next".
    Chunked {
        remaining: u64,
    },
    Length(u64),
    UntilEof,
}

/// The body of an HTTP response, de-chunked while it is read.
struct HttpBody {
    reader: BufReader<TcpStream>,
    mode: Mode,
}

impl Read for HttpBody {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        match self.mode {
            Mode::UntilEof => self.reader.read(buffer),
            Mode::Length(remaining) => {
                if remaining == 0 {
                    return Ok(0);
                }
                let limit = buffer.len().min(remaining as usize);
                let read = self.reader.read(&mut buffer[..limit])?;
                if read == 0 {
                    return Ok(0);
                }
                self.mode = Mode::Length(remaining - read as u64);
                Ok(read)
            }
            Mode::Chunked { remaining: 0 } => {
                let mut header = String::new();
                if self.reader.read_line(&mut header)? == 0 {
                    return Ok(0);
                }
                let size = header
                    .trim()
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let size = u64::from_str_radix(&size, 16)
                    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "bad chunk size"))?;
                if size == 0 {
                    // Last chunk: swallow the trailer up to the empty line.
                    loop {
                        let mut trailer = String::new();
                        if self.reader.read_line(&mut trailer)? == 0 {
                            break;
                        }
                        if trailer.trim().is_empty() {
                            break;
                        }
                    }
                    return Ok(0);
                }
                self.mode = Mode::Chunked { remaining: size };
                self.read(buffer)
            }
            Mode::Chunked { remaining } => {
                let limit = buffer.len().min(remaining as usize);
                let read = self.reader.read(&mut buffer[..limit])?;
                if read == 0 {
                    return Ok(0);
                }
                let left = remaining - read as u64;
                if left == 0 {
                    let mut ending = [0u8; 2];
                    self.reader.read_exact(&mut ending)?;
                    self.mode = Mode::Chunked { remaining: 0 };
                } else {
                    self.mode = Mode::Chunked { remaining: left };
                }
                Ok(read)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_is_a_path_a_url_or_stdin() {
        assert_eq!(parse_source("-"), Source::Stdin);
        assert_eq!(
            parse_source("http://nas:8642/export/disconnections.csv.gz"),
            Source::Url("http://nas:8642/export/disconnections.csv.gz".to_string())
        );
        assert_eq!(
            parse_source("/share/events.csv"),
            Source::Path(PathBuf::from("/share/events.csv"))
        );
        assert_eq!(
            parse_source(" ./events.csv.gz "),
            Source::Path(PathBuf::from("./events.csv.gz"))
        );
    }

    #[test]
    fn a_row_carries_the_target_what_happened_and_when() {
        let row = Row::parse("1.1.1.1,cloudflare,eth0,down,2026-11-08T00:00:00.000Z,")
            .expect("a valid row");
        assert_eq!(row.target, "1.1.1.1");
        assert_eq!(row.label.as_deref(), Some("cloudflare"));
        assert_eq!(row.interface.as_deref(), Some("eth0"));
        assert_eq!(row.state, TargetState::Down);
        assert_eq!(row.occurred_at_ms, 1_794_096_000_000);
        assert_eq!(row.rtt_us, None);
    }

    #[test]
    fn the_round_trip_time_is_read_when_present() {
        let row = Row::parse("1.1.1.1,,,up,2026-11-08T00:00:00.000Z,12500").expect("a valid row");
        assert_eq!(row.state, TargetState::Up);
        assert_eq!(row.label, None);
        assert_eq!(row.interface, None);
        assert_eq!(row.rtt_us, Some(12_500));
    }

    #[test]
    fn broken_rows_are_rejected() {
        for line in [
            "",
            "1.1.1.1",
            "1.1.1.1,,,down,not-a-date,",
            ",,,down,2026-11-08T00:00:00.000Z,",
            "1.1.1.1,,,unknown,2026-11-08T00:00:00.000Z,",
            "1.1.1.1,,,sideways,2026-11-08T00:00:00.000Z,",
            "1.1.1.1,,,up,2026-11-08T00:00:00.000Z,lento",
        ] {
            assert!(Row::parse(line).is_none(), "accepted {line:?}");
        }
    }

    #[test]
    fn a_quoted_label_survives_the_split() {
        let fields = split_csv("\"nas, lab\",\"say \"\"hi\"\"\",eth0,up,1970-01-01T00:00:00.000Z,");
        assert_eq!(fields[0], "nas, lab");
        assert_eq!(fields[1], "say \"hi\"");
        assert_eq!(fields.len(), 6);
    }
}
