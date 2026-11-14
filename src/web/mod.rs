//! The web interface: a read-only timeline, one detail page per target, and a
//! health check. `monitor --listen` serves it from the monitor process; `serve`
//! serves the same pages without probing.
//!
//! Minimal HTTP/1.1 by hand: no framework, no template engine, no port unless asked.

pub mod detail;
pub mod export;
pub mod page;
pub mod stats;
pub mod templates;

use std::io::{self, Read, Write};
use std::net::{AddrParseError, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use flate2::write::GzEncoder;
use thiserror::Error;

use crate::cli::Compression;
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
const RECENT_FAULTS: u32 = 10;

/// The icon of the application, embedded so the executable stays self-contained: the
/// masthead mark, the favicon, and the PNG behind both for the browsers without SVG
/// support.
const ICON_SVG: &str = include_str!("../../assets/icon/icon.svg");
const ICON_PNG: &[u8] = include_bytes!("../../assets/icon/favicon-32.png");

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
pub fn run(
    database: &Path,
    listen: &str,
    shutdown: &Shutdown,
    compression: Compression,
) -> Result<(), ServeError> {
    let listener = bind(listen)?;
    serve(listener, database, shutdown, compression)
}

/// Serve on an already bound listener, so a test can ask for port 0.
pub fn serve(
    listener: TcpListener,
    database: &Path,
    shutdown: &Shutdown,
    compression: Compression,
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
                    handle(stream, &database, compression);
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

fn handle(mut stream: TcpStream, database: &Path, compression: Compression) {
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
    let gzip = compression == Compression::Auto && accepts_gzip(&request);

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

            let downs = match store.recent_faults(RECENT_FAULTS) {
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
            respond_page(&mut stream, &body, include_body, gzip);
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

            let recent = match store.recent_faults_for(&target.target, RECENT_FAULTS) {
                Ok(recent) => recent,
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };

            let body = detail::target_page(&target, &days, &segments, &recent, now, period_days);
            respond_page(&mut stream, &body, include_body, gzip);
        }
        // The path the page used before the rows were called faults. Kept for one
        // release, so a bookmark and a script written against 0.2 keep working.
        "/export/disconnections.csv.gz" => {
            respond_redirect(&mut stream, "/export/faults.csv.gz");
        }
        "/export/faults.csv.gz" => {
            let params = parse_query(query);
            let store = match open_store(&mut stream, database, include_body) {
                Some(store) => store,
                None => return,
            };

            // A target in the query is the download of its detail page: the file carries
            // the history of that target only and says whose it is.
            let target = match params.target.as_deref() {
                Some(needle) if !needle.is_empty() => match store.find_target(needle) {
                    Ok(Some(target)) => Some(target),
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
                },
                _ => None,
            };

            let exported_at = export::exported_at();
            let disposition = format!(
                "Content-Disposition: attachment; filename=\"{}\"\r\n",
                export_name(
                    exported_at,
                    target
                        .as_ref()
                        .map(|target| target.label.as_deref().unwrap_or(&target.target))
                )
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
                let only = target.as_ref().map(|target| target.target.as_str());
                export::write_csv_gz(&mut chunked, &store, only)
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
        // The stylesheet and the script of the pages, served as files: the browser names
        // them in its devtools with their own lines, instead of showing them inside the page.
        "/style.css" => respond(
            &mut stream,
            200,
            "text/css; charset=utf-8",
            &templates::stylesheet(),
            include_body,
        ),
        "/app.js" => respond(
            &mut stream,
            200,
            "text/javascript; charset=utf-8",
            &templates::script(),
            include_body,
        ),
        "/icon.svg" => respond(&mut stream, 200, "image/svg+xml", ICON_SVG, include_body),
        "/icon.png" => respond_bytes(&mut stream, 200, "image/png", ICON_PNG, include_body),
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
    respond_bytes(stream, status, content_type, body.as_bytes(), include_body);
}

/// The same response for a body that is not text, such as the favicon.
fn respond_bytes(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    include_body: bool,
) {
    let head = head(status, content_type, "", body.len());
    let _ = stream.write_all(head.as_bytes());
    if include_body && !body.is_empty() {
        let _ = stream.write_all(body);
    }
    let _ = stream.flush();
}

/// A permanent redirect, so an address that moved keeps working.
///
/// The response carries no `Cache-Control`: a `308` is meant to be remembered, and the
/// page's `no-store` would tell a cache to forget it.
fn respond_redirect(stream: &mut TcpStream, location: &str) {
    let head = format!(
        "HTTP/1.1 308 Permanent Redirect\r\n\
         Location: {location}\r\n\
         Content-Length: 0\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\r\n"
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.flush();
}

/// An HTML page, compressed when the request advertises gzip and the operator allows it.
///
/// `HEAD` states the same headers as `GET` would, compressed length included.
fn respond_page(stream: &mut TcpStream, body: &str, include_body: bool, gzip: bool) {
    let content_type = "text/html; charset=utf-8";
    if !gzip {
        let head = head(200, content_type, VARY, body.len());
        let _ = stream.write_all(head.as_bytes());
        if include_body {
            let _ = stream.write_all(body.as_bytes());
        }
        let _ = stream.flush();
        return;
    }

    let mut encoder = GzEncoder::new(Vec::new(), flate2::Compression::default());
    let compressed = encoder
        .write_all(body.as_bytes())
        .and_then(|()| encoder.finish())
        .ok();
    let Some(compressed) = compressed else {
        // A compression failure must not cost the page: fall back to the plain body.
        let head = head(200, content_type, VARY, body.len());
        let _ = stream.write_all(head.as_bytes());
        if include_body {
            let _ = stream.write_all(body.as_bytes());
        }
        let _ = stream.flush();
        return;
    };

    let extra = format!("Content-Encoding: gzip\r\n{VARY}");
    let head = head(200, content_type, &extra, compressed.len());
    let _ = stream.write_all(head.as_bytes());
    if include_body {
        let _ = stream.write_all(&compressed);
    }
    let _ = stream.flush();
}

/// A cache must keep the compressed and the plain page apart.
const VARY: &str = "Vary: Accept-Encoding\r\n";

/// The head every reply starts with, so the security headers are stated once.
fn head(status: u16, content_type: &str, extra: &str, length: usize) -> String {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        404 => "Not Found",
        405 => "Method Not Allowed",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "OK",
    };

    format!(
        "HTTP/1.1 {status} {reason}\r\n\
         Content-Type: {content_type}\r\n\
         {extra}\
         Content-Length: {length}\r\n\
         Cache-Control: no-store\r\n\
         X-Content-Type-Options: nosniff\r\n\
         Connection: close\r\n\r\n"
    )
}

/// Whether the request advertises gzip, with a quality above zero.
///
/// Every browser sends `gzip`; a client that does not must get the plain body.
fn accepts_gzip(request: &str) -> bool {
    for line in request.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if !name.trim().eq_ignore_ascii_case("accept-encoding") {
            continue;
        }
        for item in value.split(',') {
            let mut pieces = item.split(';');
            if !pieces
                .next()
                .unwrap_or_default()
                .trim()
                .eq_ignore_ascii_case("gzip")
            {
                continue;
            }
            let refused = pieces.any(|parameter| {
                parameter
                    .trim()
                    .strip_prefix("q=")
                    .and_then(|quality| quality.trim().parse::<f32>().ok())
                    == Some(0.0)
            });
            if !refused {
                return true;
            }
        }
    }
    false
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

/// `faults-20261109.csv.gz`, or `faults-cloudflare-dns-20261109.csv.gz` for the history of
/// one target, so two downloads never collide and a file says whose history it holds.
fn export_name(now_ms: i64, target: Option<&str>) -> String {
    let stamp = crate::time::format_rfc3339(now_ms);
    let date = format!("{}{}{}", &stamp[0..4], &stamp[5..7], &stamp[8..10]);
    match target {
        Some(name) => format!("faults-{}-{date}.csv.gz", slug(name)),
        None => format!("faults-{date}.csv.gz"),
    }
}

/// The longest a target name may run in a file name.
const SLUG_LIMIT: usize = 48;

/// A file name that names a target: ASCII letters, digits, `.`, and `_` are kept, every
/// other character folds into one `-`, and a name that keeps nothing falls back to
/// `target`. A key such as `https://example.com/a?b=1` becomes `https-example.com-a-b-1`.
fn slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut pending_dash = false;
    for character in name.chars() {
        if slug.len() >= SLUG_LIMIT {
            break;
        }
        if character.is_ascii_alphanumeric() || character == '.' || character == '_' {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(character);
        } else {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        "target".to_string()
    } else {
        slug
    }
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

/// Percent-encode a value carried in a query, so a target key survives the link.
///
/// Every character outside the unreserved set is escaped: a key such as
/// `https://example.com/a?b=1` must not lose its `?` and `&` to the query parser.
pub(crate) fn encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
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

    /// The file of one target says whose history it holds; the whole history says only
    /// when it was taken.
    #[test]
    fn the_export_of_one_target_is_named_after_it() {
        // 2026-11-09T00:00:00Z
        let at = 1_794_182_400_000;
        assert_eq!(export_name(at, None), "faults-20261109.csv.gz");
        assert_eq!(
            export_name(at, Some("cloudflare-dns")),
            "faults-cloudflare-dns-20261109.csv.gz"
        );

        assert_eq!(slug("cloudflare-dns"), "cloudflare-dns");
        assert_eq!(slug("https://example.com/a?b=1"), "https-example.com-a-b-1");
        assert_eq!(
            slug("///"),
            "target",
            "a name that keeps nothing still names a file"
        );
        assert!(
            slug(&"a".repeat(200)).len() <= SLUG_LIMIT,
            "a long key is capped"
        );
    }

    #[test]
    fn a_request_that_advertises_gzip_gets_a_compressed_page() {
        let request = "GET / HTTP/1.1\r\nHost: localhost\r\nAccept-Encoding: gzip\r\n\r\n";
        assert!(accepts_gzip(request));
    }

    #[test]
    fn a_request_without_the_header_or_with_another_coding_gets_plain_bytes() {
        assert!(!accepts_gzip("GET / HTTP/1.1\r\nHost: localhost\r\n\r\n"));
        assert!(!accepts_gzip(
            "GET / HTTP/1.1\r\nAccept-Encoding: deflate\r\n\r\n"
        ));
    }

    #[test]
    fn the_header_is_read_case_insensitively_and_among_other_codings() {
        assert!(accepts_gzip(
            "GET / HTTP/1.1\r\naccept-encoding: br, GZIP;q=0.8, deflate\r\n\r\n"
        ));
    }

    #[test]
    fn a_coding_refused_with_a_zero_quality_is_not_used() {
        assert!(!accepts_gzip(
            "GET / HTTP/1.1\r\nAccept-Encoding: gzip;q=0, deflate\r\n\r\n"
        ));
        assert!(accepts_gzip(
            "GET / HTTP/1.1\r\nAccept-Encoding: gzip;q=1.0\r\n\r\n"
        ));
    }

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
    fn a_target_key_survives_the_query_it_is_written_into() {
        let key = "https://example.com/a?b=1&c=2#fragment";
        let encoded = encode(key);

        assert_eq!(
            encoded,
            "https%3A%2F%2Fexample.com%2Fa%3Fb%3D1%26c%3D2%23fragment"
        );
        assert_eq!(decode(&encoded), key);
        assert_eq!(
            parse_query(&format!("target={encoded}")).target.as_deref(),
            Some(key)
        );
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
