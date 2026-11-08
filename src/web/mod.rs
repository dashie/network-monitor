//! The web interface: a read-only timeline, one detail page per target, and a
//! health check. `monitor --listen` serves it from the monitor process; `serve`
//! serves the same pages without probing.
//!
//! Minimal HTTP/1.1 by hand: no framework, no template engine, no port unless asked.

pub mod detail;
pub mod page;

use std::io::{Read, Write};
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
            let window = resolve_window(params.from, params.to, now_ms());

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

            let body = if params.partial {
                page::chart(&timelines, window)
            } else {
                page::page(&timelines, window)
            };
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
            let days = match store.daily_drops(&target.target, now - 30 * DAY_MS, now) {
                Ok(days) => days,
                Err(error) => {
                    query_failed(&mut stream, &error, include_body);
                    return;
                }
            };

            let body = detail::target_page(&target, &days);
            respond(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                &body,
                include_body,
            );
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

#[derive(Debug, Default)]
struct Params {
    from: Option<i64>,
    to: Option<i64>,
    partial: bool,
    target: Option<String>,
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
            "partial" => params.partial = value == "1" || value == "true",
            "target" => params.target = Some(decode(value)),
            _ => {}
        }
    }
    params
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

fn resolve_window(from: Option<i64>, to: Option<i64>, now: i64) -> Window {
    let to = to.unwrap_or(now).min(now);
    let mut from = from.unwrap_or(to - DAY_MS);

    if from >= to {
        from = to - DAY_MS;
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
    fn the_default_window_is_the_last_day() {
        let now = 1_794_096_000_000;
        let window = resolve_window(None, None, now);
        assert_eq!(window.to_ms, now);
        assert_eq!(window.from_ms, now - DAY_MS);
    }

    #[test]
    fn the_window_never_extends_into_the_future() {
        let now = 1_000_000;
        let window = resolve_window(Some(500_000), Some(2_000_000), now);
        assert_eq!(window.to_ms, now);
        assert_eq!(window.from_ms, 500_000);
    }

    #[test]
    fn a_reversed_window_falls_back_to_a_day() {
        let now = 10 * DAY_MS;
        let window = resolve_window(Some(now), Some(now - DAY_MS), now);
        assert_eq!(window.to_ms - window.from_ms, DAY_MS);
    }

    #[test]
    fn a_tiny_window_is_widened() {
        let now = 10 * DAY_MS;
        let window = resolve_window(Some(now - 1_000), Some(now), now);
        assert_eq!(window.to_ms - window.from_ms, MIN_SPAN_MS);
    }

    #[test]
    fn the_query_is_parsed_leniently() {
        let params = parse_query("from=10&to=20&partial=1&junk&x=y");
        assert_eq!(params.from, Some(10));
        assert_eq!(params.to, Some(20));
        assert!(params.partial);

        let broken = parse_query("from=abc");
        assert_eq!(broken.from, None);
        assert!(!broken.partial);
    }
}
