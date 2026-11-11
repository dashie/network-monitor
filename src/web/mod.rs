//! The web interface: a read-only timeline, one detail page per target, and a
//! health check. `monitor --listen` serves it from the monitor process; `serve`
//! serves the same pages without probing.
//!
//! Minimal HTTP/1.1 by hand: no framework, no template engine, no port unless asked.

pub mod detail;
pub mod export;
pub mod page;
pub mod stats;

use std::io::{self, Read, Write};
use std::net::{AddrParseError, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use thiserror::Error;

use crate::logging;
use crate::shutdown::{self, Shutdown};
use crate::store::{Store, StoreError};
use crate::time::now_ms;

use page::Window;

const DAY_MS: i64 = 86_400_000;
const MIN_SPAN_MS: i64 = 5 * 60_000;
const MAX_SPAN_MS: i64 = 30 * DAY_MS;
const MAX_CONNECTIONS: usize = 32;
/// How many disconnections the timeline page lists below the chart.
const RECENT_DOWNS: u32 = 10;

#[derive(Debug, Error)]
pub enum ServeError {
    #[error("invalid listen address '{address}': {reason}")]
    Address { address: String, reason: String },
    #[error("cannot listen on {address}: {source}")]
    Bind {
        address: String,
        source: std::io::Error,
    },
    #[error("cannot read the event database: {0}")]
    Store(#[from] StoreError),
}

/// Bind the address, so a caller can fail before doing other work.
pub fn bind(listen: &str) -> Result<TcpListener, ServeError> {
    let address: SocketAddr =
        listen
            .parse()
            .map_err(|error: AddrParseError| ServeError::Address {
                address: listen.to_string(),
                reason: error.to_string(),
            })?;
    TcpListener::bind(address).map_err(|source| ServeError::Bind {
        address: address.to_string(),
        source,
    })
}

/// Bind the address and serve until shutdown.
pub fn run(database: &Path, listen: &str, shutdown: &Shutdown) -> Result<(), ServeError> {
    let listener = bind(listen)?;
    serve(listener, database, shutdown)
}

/// Serve on an already bound listener, so a test can ask for port 0.
pub fn serve(
    listener: TcpListener,
    database: &Path,
    shutdown: &Shutdown,
) -> Result<(), ServeError> {
    // Fail before the first request when the database is not usable.
    Store::open_ro(database)?;

    let address = listener
        .local_addr()
        .map(|address| address.to_string())
        .unwrap_or_else(|_| "?".to_string());
    listener
        .set_nonblocking(true)
        .map_err(|source| ServeError::Bind {
            address: address.clone(),
            source,
        })?;

    logging::info(
        None,
        &format!(
            "web interface on http://{address}/ reading {}",
            database.display()
        ),
    );

    let database = Arc::new(database.to_path_buf());
    let active = Arc::new(AtomicUsize::new(0));
    let mut workers = Vec::new();

    while !shutdown::stopped(shutdown) {
        match listener.accept() {
            Ok((stream, _)) => {
                if active.load(Ordering::Relaxed) >= MAX_CONNECTIONS {
                    drop(stream);
                    continue;
                }
                active.fetch_add(1, Ordering::Relaxed);
                let database = Arc::clone(&database);
                let active = Arc::clone(&active);
                workers.push(thread::spawn(move || {
                    handle(stream, &database);
                    active.fetch_sub(1, Ordering::Relaxed);
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                logging::warn(None, &format!("web: cannot accept a connection: {error}"));
                thread::sleep(Duration::from_millis(200));
            }
        }
    }

    for worker in workers {
        let _ = worker.join();
    }
    logging::info(None, "web interface stopped");
    Ok(())
}

fn handle(mut stream: TcpStream, database: &Path) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));

    let mut buffer = [0u8; 4096];
    let read = match stream.read(&mut buffer) {
        Ok(0) => return,
        Ok(read) => read,
        Err(_) => return,
    };

    let request = String::from_utf8_lossy(&buffer[..read]);
    let line = request.lines().next().unwrap_or_default();
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or("/");

    if method != "GET" && method != "HEAD" {
        respond(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            "method not allowed\n",
            true,
        );
        return;
    }
    let include_body = method == "GET";

    let (path, query) = match target.split_once('?') {
        Some((path, query)) => (path, query),
        None => (target, ""),
    };

    match path {
        "/" => {
            let params = parse_query(query);
            let window = resolve_window(params.from, params.to, params.window, now_ms());

            let store = match open_store(&mut stream, database, include_body) {
                Some(store) => store,
                None => return,
            };

            let timelines = match store.timeline(window.from_ms, window.to_ms) {
                Ok(timelines) => timelines,
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };

            let downs = match store.recent_downs(RECENT_DOWNS) {
                Ok(downs) => downs,
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };

            // How long each target has been up without a drop: the age of the current
            // state, from the transition that set it, which the segments cannot say when
            // the window opened in the middle of a run.
            let states = match store.states() {
                Ok(states) => states,
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };
            let mut up_since = std::collections::HashMap::new();
            for state in &states {
                if let Some(up_for_ms) =
                    page::up_for(state.state, state.last_change_at_ms, now_ms())
                {
                    up_since.insert(state.target.clone(), up_for_ms);
                }
            }

            let body = page::page(&timelines, &downs, &up_since, window);
            respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                &body,
                include_body,
            );
        }
        "/target" => {
            let params = parse_query(query);
            let store = match open_store(&mut stream, database, include_body) {
                Some(store) => store,
                None => return,
            };

            let found = match params.target.as_deref() {
                Some(needle) if !needle.is_empty() => store.find_target(needle),
                _ => Ok(None),
            };
            let target = match found {
                Ok(Some(target)) => target,
                Ok(None) => {
                    respond(
                        &mut stream,
                        404,
                        "text/plain; charset=utf-8",
                        "unknown target\n",
                        include_body,
                    );
                    return;
                }
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };

            let now = now_ms();
            let period_days = params.days.unwrap_or(detail::DEFAULT_PERIOD_DAYS);
            let from_ms = now - i64::from(period_days) * DAY_MS;
            let days = match store.daily_drops(&target.target, from_ms, now) {
                Ok(days) => days,
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };

            // The same segments the timeline page draws, for this target only: the
            // per-day average of the `up` runs comes from the run boundaries, which
            // the aggregated day buckets do not carry.
            let segments = match store.timeline(from_ms, now) {
                Ok(timelines) => timelines
                    .into_iter()
                    .find(|timeline| timeline.target == target.target)
                    .map(|timeline| timeline.segments)
                    .unwrap_or_default(),
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };

            let body = detail::target_page(&target, &days, &segments, now, period_days);
            respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                &body,
                include_body,
            );
        }
        "/export/disconnections.csv.gz" => {
            let store = match open_store(&mut stream, database, include_body) {
                Some(store) => store,
                None => return,
            };

            let exported_at = export::exported_at();
            let disposition = format!(
                "Content-Disposition: attachment; filename=\"{}\"\r\n",
                export_name(exported_at)
            );
            begin_chunked(&mut stream, "application/gzip", &disposition);
            if !include_body {
                return;
            }

            // The file is compressed straight into the response: it is never built in
            // memory and its length is never known in advance.
            let outcome = {
                let mut chunked = Chunked {
                    stream: &mut stream,
                };
                export::write_csv_gz(&mut chunked, &store)
            };
            if let Err(error) = outcome {
                logging::error(None, &format!("web: export failed: {error}"));
            }
            end_chunked(&mut stream);
        }
        "/healthz" => respond(
            &mut stream,
            200,
            "text/plain; charset=utf-8",
            "ok\n",
            include_body,
        ),
        "/favicon.ico" => respond(&mut stream, 204, "text/plain; charset=utf-8", "", true),
        _ => respond(
            &mut stream,
            404,
            "text/plain; charset=utf-8",
            "not found\n",
            include_body,
        ),
    }
}

/// Open the database read-only, answering 503 when it is not usable.
fn open_store(stream: &mut TcpStream, database: &Path, include_body: bool) -> Option<Store> {
    match Store::open_ro(database) {
        Ok(store) => Some(store),
        Err(error) => {
            logging::warn(None, &format!("web: cannot read the database: {error}"));
            respond(
                stream,
                503,
                "text/plain; charset=utf-8",
                &format!("{error}\n"),
                include_body,
            );
            None
        }
    }
}

fn query_failed(stream: &mut TcpStream, error: &StoreError, include_body: bool) {
    logging::error(None, &format!("web: query failed: {error}"));
    respond(
        stream,
        500,
        "text/plain; charset=utf-8",
        "query failed\n",
        include_body,
    );
}

fn respond(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &str,
    include_body: bool,
) {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    };

    let head = format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );

    let _ = stream.write_all(head.as_bytes());
    if include_body && !body.is_empty() {
        let _ = stream.write_all(body.as_bytes());
    }
    let _ = stream.flush();
}

/// A response body of unknown length, written as HTTP/1.1 chunks.
///
/// The archive is generated while it is sent, so no size is known in advance and
/// nothing is buffered.
struct Chunked<'a> {
    stream: &'a mut TcpStream,
}

impl Write for Chunked<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        self.stream
            .write_all(format!("{:x}\r\n", buffer.len()).as_bytes())?;
        self.stream.write_all(buffer)?;
        self.stream.write_all(b"\r\n")?;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stream.flush()
    }
}

fn begin_chunked(stream: &mut TcpStream, content_type: &str, extra_headers: &str) {
    let head = format!(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: {content_type}\r\n\
         Transfer-Encoding: chunked\r\n\
         {extra_headers}\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\r\n"
    );
    let _ = stream.write_all(head.as_bytes());
}

fn end_chunked(stream: &mut TcpStream) {
    let _ = stream.write_all(b"0\r\n\r\n");
    let _ = stream.flush();
}

/// `disconnections-20261109.csv.gz`, so two downloads never collide.
fn export_name(now_ms: i64) -> String {
    let stamp = crate::time::format_rfc3339(now_ms);
    format!(
        "disconnections-{}{}{}.csv.gz",
        &stamp[0..4],
        &stamp[5..7],
        &stamp[8..10]
    )
}

#[derive(Debug, Default)]
struct Params {
    from: Option<i64>,
    to: Option<i64>,
    window: Option<u8>,
    target: Option<String>,
    days: Option<u32>,
}

fn parse_query(query: &str) -> Params {
    let mut params = Params::default();
    for pair in query.split('&') {
        let (key, value) = match pair.split_once('=') {
            Some(pair) => pair,
            None => continue,
        };
        match key {
            "from" => params.from = value.parse().ok(),
            "to" => params.to = value.parse().ok(),
            "window" => params.window = parse_hours(value),
            "target" => params.target = Some(decode(value)),
            "days" => params.days = detail::parse_period(value),
            _ => {}
        }
    }
    params
}

/// `6h` selects one of the windows the page offers; anything else is ignored, so a
/// hand-edited query falls back to the default instead of failing.
fn parse_hours(value: &str) -> Option<u8> {
    let hours: u8 = value.strip_suffix('h').unwrap_or(value).parse().ok()?;
    page::WINDOWS.contains(&hours).then_some(hours)
}

/// Decode the escapes a browser puts in a query value.
fn decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => match value.get(index + 1..index + 3) {
                Some(hex) => match u8::from_str_radix(hex, 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(bytes[index]);
                        index += 1;
                    }
                },
                None => {
                    out.push(bytes[index]);
                    index += 1;
                }
            },
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn resolve_window(from: Option<i64>, to: Option<i64>, hours: Option<u8>, now: i64) -> Window {
    let span = hours.map_or_else(
        || page::hours_span(page::DEFAULT_WINDOW_HOURS),
        page::hours_span,
    );
    let to = to.unwrap_or(now).min(now);
    let mut from = from.unwrap_or(to - span);

    if from >= to {
        from = to - span;
    }
    if to - from < MIN_SPAN_MS {
        from = to - MIN_SPAN_MS;
    }
    if to - from > MAX_SPAN_MS {
        from = to - MAX_SPAN_MS;
    }
    from = from.max(0);

    Window {
        from_ms: from,
        to_ms: to.max(from + 1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_window_is_the_last_hour() {
        let now = 1_794_096_000_000;
        let window = resolve_window(None, None, None, now);
        assert_eq!(window.to_ms, now);
        assert_eq!(
            window.from_ms,
            now - page::hours_span(page::DEFAULT_WINDOW_HOURS)
        );
        assert_eq!(window.to_ms - window.from_ms, 3_600_000);
    }

    #[test]
    fn a_selected_window_ends_now_and_lasts_what_it_says() {
        let now = 1_794_096_000_000;
        for hours in page::WINDOWS {
            let window = resolve_window(None, None, Some(hours), now);
            assert_eq!(window.to_ms, now);
            assert_eq!(window.to_ms - window.from_ms, page::hours_span(hours));
        }
    }

    #[test]
    fn the_window_value_names_one_of_the_offered_spans() {
        assert_eq!(parse_query("window=6h").window, Some(6));
        assert_eq!(parse_query("window=1h").window, Some(1));
        assert_eq!(parse_query("window=24h").window, Some(24));
        assert_eq!(parse_query("window=5h").window, None, "not offered");
        assert_eq!(parse_query("window=abc").window, None);
        assert_eq!(parse_query("window=").window, None);
    }

    #[test]
    fn an_unknown_window_falls_back_to_the_default() {
        let now = 1_794_096_000_000;
        let params = parse_query("window=99h");
        let window = resolve_window(params.from, params.to, params.window, now);
        assert_eq!(
            window.to_ms - window.from_ms,
            page::hours_span(page::DEFAULT_WINDOW_HOURS)
        );
    }

    #[test]
    fn the_window_never_extends_into_the_future() {
        let now = 1_000_000;
        let window = resolve_window(Some(500_000), Some(2_000_000), None, now);
        assert_eq!(window.to_ms, now);
        assert_eq!(window.from_ms, 500_000);
    }

    #[test]
    fn a_reversed_window_falls_back_to_the_default() {
        let now = 10 * DAY_MS;
        let window = resolve_window(Some(now), Some(now - DAY_MS), None, now);
        assert_eq!(window.to_ms - window.from_ms, page::hours_span(1));
    }

    #[test]
    fn a_tiny_window_is_widened() {
        let now = 10 * DAY_MS;
        let window = resolve_window(Some(now - 1_000), Some(now), None, now);
        assert_eq!(window.to_ms - window.from_ms, MIN_SPAN_MS);
    }

    #[test]
    fn the_query_is_parsed_leniently() {
        let params = parse_query("from=10&to=20&junk&x=y");
        assert_eq!(params.from, Some(10));
        assert_eq!(params.to, Some(20));

        let broken = parse_query("from=abc");
        assert_eq!(broken.from, None);
    }

    #[test]
    fn the_period_value_names_one_of_the_offered_spans() {
        assert_eq!(parse_query("days=30d").days, Some(30));
        assert_eq!(parse_query("days=360d").days, Some(360));
        assert_eq!(parse_query("days=90").days, Some(90));
        assert_eq!(parse_query("days=45d").days, None, "not offered");
        assert_eq!(parse_query("days=abc").days, None);
        assert_eq!(parse_query("target=1.1.1.1&days=180d").days, Some(180));
    }

    #[test]
    fn a_target_is_decoded_from_the_query() {
        assert_eq!(
            parse_query("target=1.1.1.1").target.as_deref(),
            Some("1.1.1.1")
        );
        assert_eq!(
            parse_query("target=my%2Dnas").target.as_deref(),
            Some("my-nas")
        );
        assert_eq!(parse_query("target=a+b").target.as_deref(), Some("a b"));
    }
}
