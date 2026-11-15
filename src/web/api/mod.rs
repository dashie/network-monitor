//! The read-only JSON API under `/api/v1`.
//!
//! Same socket, same read-only connection, same connection cap as the page: the API adds
//! no surface and writes nothing. Every answer is an object — `data` and `meta` on
//! success, `error` and `meta` on failure — because an array at the root cannot carry the
//! pagination facts a client needs.
//!
//! This module holds the contract every route shares; one file per route holds the route.

mod daily;
mod faults;
mod openapi;
mod targets;

use std::io::{self, Write};
use std::net::TcpStream;
use std::path::Path;

use flate2::write::GzEncoder;

use crate::json;
use crate::logging;
use crate::store::{Store, StoreError};
use crate::time::{format_rfc3339, now_ms, parse_rfc3339};
use crate::web::{begin_chunked, end_chunked, Chunked, VARY};

const JSON: &str = "application/json; charset=utf-8";

/// A dashboard on another origin may read the history the page already publishes.
const CORS: &str = "Access-Control-Allow-Origin: *\r\n";

/// How many records a page holds when the request states no `limit`.
const DEFAULT_LIMIT: u32 = 100;
/// The largest page a client may ask for, so one request cannot read the whole history.
const MAX_LIMIT: u32 = 1000;
/// The window the daily totals cover when the request names none.
const DEFAULT_DAYS: i64 = 30;

/// The oldest instant a filter accepts: `1970-01-01T00:00:00Z`.
const DOMAIN_MIN_MS: i64 = 0;
/// The newest instant a filter accepts: `2100-01-01T00:00:00Z`.
///
/// A fixed calendar window, not a ceiling anchored to the clock: a client asking for "this
/// month" or "this year" sends a `to` in the future, and a moving bound would refuse it. The
/// window also keeps every day of the daily route far from the edges of `i64` — 2100 is 4.1e12
/// against 9.2e18 — and the whole domain holds 47 482 days, so no answer states an absurd total.
const DOMAIN_MAX_MS: i64 = 4_102_444_800_000;

/// Route the request, or answer that there is no such route.
pub fn handle(
    stream: &mut TcpStream,
    method: &str,
    path: &str,
    query: &str,
    database: &Path,
    include_body: bool,
    gzip: bool,
) {
    let reply = Reply {
        include_body,
        gzip,
        extra: CORS,
    };

    if method == "OPTIONS" {
        respond_options(stream);
        return;
    }

    if method != "GET" && method != "HEAD" {
        let reply = Reply {
            extra: "Access-Control-Allow-Origin: *\r\nAllow: GET, HEAD, OPTIONS\r\n",
            ..reply
        };
        fail(
            stream,
            405,
            "METHOD_NOT_ALLOWED",
            None,
            "only GET, HEAD, and OPTIONS are answered here",
            reply,
        );
        return;
    }

    match path {
        "/api/v1/openapi.json" => openapi::route(stream, reply),
        "/api/v1/targets" => targets::route(stream, query, database, reply),
        "/api/v1/faults" => faults::route(stream, query, database, reply),
        "/api/v1/faults/daily" => daily::route(stream, query, database, reply),
        _ => fail(stream, 404, "NOT_FOUND", None, "no such route", reply),
    }
}

/// How an answer leaves: whether the body is written, whether it may be compressed, and
/// the headers this answer adds.
#[derive(Clone, Copy)]
pub(super) struct Reply<'a> {
    pub(super) include_body: bool,
    pub(super) gzip: bool,
    pub(super) extra: &'a str,
}

/// The facts a client needs to page: what it asked for, and how much there is.
pub(super) struct Meta<'a> {
    pub(super) limit: u32,
    pub(super) offset: u32,
    pub(super) total: i64,
    pub(super) target: Option<&'a str>,
    pub(super) from: Option<i64>,
    pub(super) to: Option<i64>,
    pub(super) generated_at: i64,
}

impl Meta<'_> {
    pub(super) fn json(&self) -> String {
        format!(
            "  \"meta\": {{\"limit\": {}, \"offset\": {}, \"total\": {}, \
             \"filters\": {{\"target\": {}, \"from\": {}, \"to\": {}}}, \"generatedAt\": {}}}",
            self.limit,
            self.offset,
            self.total,
            json::option_string(&self.target.map(str::to_string)),
            instant(self.from),
            instant(self.to),
            json::string(&format_rfc3339(self.generated_at)),
        )
    }
}

/// A data answer written as it is read: the head goes out at once, then one chunk per
/// record, then `meta`. Nothing holds the whole document.
pub(super) struct Envelope<'a> {
    body: Body<'a>,
    records: usize,
}

enum Body<'a> {
    /// A `HEAD`: the headers are written and no byte follows.
    Head,
    Plain(Chunked<'a>),
    Gzip(Box<GzEncoder<Chunked<'a>>>),
}

impl Body<'_> {
    fn write(&mut self, text: &str) -> io::Result<()> {
        match self {
            Body::Head => Ok(()),
            Body::Plain(writer) => writer.write_all(text.as_bytes()),
            Body::Gzip(encoder) => encoder.write_all(text.as_bytes()),
        }
    }

    fn finish(self) -> io::Result<()> {
        match self {
            Body::Head => Ok(()),
            Body::Plain(mut writer) => writer.flush(),
            Body::Gzip(encoder) => encoder.finish().map(|_| ()),
        }
    }
}

impl<'a> Envelope<'a> {
    pub(super) fn begin(stream: &'a mut TcpStream, reply: Reply) -> Envelope<'a> {
        let encoding = if reply.gzip {
            "Content-Encoding: gzip\r\n"
        } else {
            ""
        };
        let extra = format!("{}{VARY}{encoding}", reply.extra);
        begin_chunked(stream, JSON, &extra);

        let body = if !reply.include_body {
            Body::Head
        } else if reply.gzip {
            Body::Gzip(Box::new(GzEncoder::new(
                Chunked { stream },
                flate2::Compression::default(),
            )))
        } else {
            Body::Plain(Chunked { stream })
        };
        Envelope { body, records: 0 }
    }

    /// Write one rendered record, opening or continuing the `data` array.
    pub(super) fn record(&mut self, record: &str) -> io::Result<()> {
        if self.records == 0 {
            self.body.write("{\n  \"data\": [\n")?;
        } else {
            self.body.write(",\n")?;
        }
        self.records += 1;
        self.body.write(record)
    }

    /// Close the array, state `meta`, and flush the encoder.
    pub(super) fn finish(mut self, meta: &Meta) -> io::Result<()> {
        if self.records == 0 {
            self.body.write("{\n  \"data\": []")?;
        } else {
            self.body.write("\n]")?;
        }
        self.body.write(",\n")?;
        self.body.write(&meta.json())?;
        self.body.write("\n}\n")?;
        self.body.finish()
    }
}

/// A failure that happened after the status line went out, so no status can state it.
pub(super) enum StreamError {
    Store(StoreError),
    Write(io::Error),
}

/// Report a failure that truncated an answer: the status is already sent, so the reader
/// sees an incomplete body and the reason stays in the log.
pub(super) fn truncated(error: StreamError) {
    match error {
        StreamError::Store(error) => {
            logging::error(None, &format!("web: api: query failed mid-answer: {error}"))
        }
        StreamError::Write(error) => {
            logging::warn(None, &format!("web: api: the answer stopped: {error}"))
        }
    }
}

/// Write the last chunk of a streamed answer, unless the request asked for headers only.
pub(super) fn close(stream: &mut TcpStream, reply: Reply) {
    if reply.include_body {
        end_chunked(stream);
    }
}

pub(super) fn fail(
    stream: &mut TcpStream,
    status: u16,
    code: &str,
    parameter: Option<&str>,
    message: &str,
    reply: Reply,
) {
    let body = format!(
        "{{\n  \"error\": {{\"status\": {status}, \"code\": {}, \"parameter\": {}, \
         \"message\": {}}},\n  \"meta\": {{\"generatedAt\": {}}}\n}}",
        json::string(code),
        json::option_string(&parameter.map(str::to_string)),
        json::string(message),
        json::string(&format_rfc3339(now_ms())),
    );
    reply.send(stream, status, &body);
}

pub(super) fn reject(stream: &mut TcpStream, rejection: Rejection, reply: Reply) {
    if let Some(detail) = &rejection.detail {
        logging::error(None, &format!("web: api: {detail}"));
    }
    fail(
        stream,
        rejection.status,
        rejection.code,
        rejection.parameter.as_deref(),
        &rejection.message,
        reply,
    );
}

fn respond_options(stream: &mut TcpStream) {
    let extra = format!(
        "{CORS}Access-Control-Allow-Methods: GET, HEAD, OPTIONS\r\n\
         Access-Control-Max-Age: 600\r\n"
    );
    let head = super::head(204, JSON, &extra, 0);
    let _ = std::io::Write::write_all(stream, head.as_bytes());
    let _ = std::io::Write::flush(stream);
}

/// Open the database read-only, answering 503 in the shape of the API when it is missing.
pub(super) fn open(stream: &mut TcpStream, database: &Path, reply: Reply) -> Option<Store> {
    match Store::open_ro(database) {
        Ok(store) => Some(store),
        Err(error) => {
            logging::warn(None, &format!("web: cannot read the database: {error}"));
            fail(
                stream,
                503,
                "DATABASE_UNAVAILABLE",
                None,
                "cannot read the event database",
                reply,
            );
            None
        }
    }
}

/// The canonical key of the requested target, `None` when the request names none.
pub(super) fn resolve(store: &Store, needle: Option<&str>) -> Result<Option<String>, Rejection> {
    match needle {
        Some(needle) if !needle.is_empty() => match store.resolve_target(needle) {
            Ok(Some(key)) => Ok(Some(key)),
            Ok(None) => Err(Rejection::unknown_target(needle)),
            Err(error) => Err(Rejection::failed(&error)),
        },
        _ => Ok(None),
    }
}

/// What a route accepts, with the defaults the API states in its documentation.
pub(super) struct Params {
    pub(super) target: Option<String>,
    pub(super) from: Option<i64>,
    pub(super) to: Option<i64>,
    pub(super) limit: u32,
    pub(super) offset: u32,
}

impl Params {
    pub(super) fn parse(query: &str, accepted: &[&str]) -> Result<Self, Rejection> {
        let mut params = Params {
            target: None,
            from: None,
            to: None,
            limit: DEFAULT_LIMIT,
            offset: 0,
        };

        let mut seen: Vec<&str> = Vec::new();
        for pair in query.split('&') {
            if pair.is_empty() {
                continue;
            }
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            if !accepted.contains(&name) {
                return Err(Rejection::unknown_parameter(name));
            }
            if seen.contains(&name) {
                return Err(Rejection::parameter(
                    name,
                    format!("{name} is given more than once"),
                ));
            }
            seen.push(name);
            match name {
                "target" => params.target = Some(super::decode(value)),
                "from" => params.from = Some(parse_instant(name, value)?),
                "to" => params.to = Some(parse_instant(name, value)?),
                "limit" => {
                    params.limit = parse_number(name, value, 1, u64::from(MAX_LIMIT))? as u32
                }
                "offset" => {
                    params.offset = parse_number(name, value, 0, u64::from(u32::MAX))? as u32
                }
                _ => unreachable!("every accepted name is handled"),
            }
        }

        if let (Some(from), Some(to)) = (params.from, params.to) {
            if from >= to {
                return Err(Rejection::range());
            }
        }
        Ok(params)
    }
}

/// An RFC 3339 UTC instant, or epoch milliseconds, inside the accepted calendar domain.
fn parse_instant(name: &str, value: &str) -> Result<i64, Rejection> {
    let millis = match value.parse::<i64>() {
        Ok(millis) => millis,
        Err(_) => parse_rfc3339(value).ok_or_else(|| {
            Rejection::parameter(
                name,
                format!(
                    "{name} must be an RFC 3339 UTC instant such as 2026-11-14T22:00:00.000Z, \
                     or epoch milliseconds"
                ),
            )
        })?,
    };
    if !(DOMAIN_MIN_MS..=DOMAIN_MAX_MS).contains(&millis) {
        return Err(Rejection::parameter(
            name,
            format!(
                "{name} must be an RFC 3339 UTC instant between {} and {}, \
                 or epoch milliseconds in that range",
                format_rfc3339(DOMAIN_MIN_MS),
                format_rfc3339(DOMAIN_MAX_MS),
            ),
        ));
    }
    Ok(millis)
}

fn parse_number(name: &str, value: &str, min: u64, max: u64) -> Result<u64, Rejection> {
    let parsed = value.parse::<u64>().map_err(|_| {
        Rejection::parameter(
            name,
            format!("{name} must be an integer between {min} and {max}"),
        )
    })?;
    if parsed < min || parsed > max {
        return Err(Rejection::parameter(
            name,
            format!("{name} must be an integer between {min} and {max}"),
        ));
    }
    Ok(parsed)
}

pub(super) fn instant(value: Option<i64>) -> String {
    value.map_or_else(
        || "null".to_string(),
        |ms| json::string(&format_rfc3339(ms)),
    )
}

impl Reply<'_> {
    pub(super) fn send(self, stream: &mut TcpStream, status: u16, body: &str) {
        super::respond_compressed(
            stream,
            status,
            JSON,
            body,
            self.include_body,
            self.gzip,
            self.extra,
        );
    }
}

/// Why a request was refused, and the answer that states it.
pub(super) struct Rejection {
    pub(super) status: u16,
    pub(super) code: &'static str,
    pub(super) parameter: Option<String>,
    pub(super) message: String,
    /// What only the log states: a store error may name a path or a SQLite detail.
    pub(super) detail: Option<String>,
}

impl Rejection {
    pub(super) fn parameter(name: &str, message: String) -> Self {
        Rejection {
            status: 400,
            code: "INVALID_PARAMETER",
            parameter: Some(name.to_string()),
            message,
            detail: None,
        }
    }

    pub(super) fn unknown_parameter(name: &str) -> Self {
        Self::parameter(
            name,
            format!(
                "unknown parameter '{name}'; the route accepts target, from, to, limit, and offset"
            ),
        )
    }

    pub(super) fn range() -> Self {
        Rejection {
            status: 400,
            code: "INVALID_RANGE",
            parameter: Some("from".to_string()),
            message: "from must be earlier than to".to_string(),
            detail: None,
        }
    }

    pub(super) fn unknown_target(needle: &str) -> Self {
        Rejection {
            status: 404,
            code: "UNKNOWN_TARGET",
            parameter: Some("target".to_string()),
            message: format!("unknown target '{needle}'"),
            detail: None,
        }
    }

    pub(super) fn failed(error: &StoreError) -> Self {
        Rejection {
            status: 500,
            code: "QUERY_FAILED",
            parameter: None,
            message: "query failed".to_string(),
            detail: Some(error.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The parse without its `Rejection`, which carries no `Debug` and needs none.
    fn parsed(name: &str, value: &str) -> Option<i64> {
        parse_instant(name, value).ok()
    }

    /// The two bounds are the dates the failure message and the documents state: a wrong literal
    /// would otherwise only show up in an answer, never in a test.
    #[test]
    fn the_accepted_domain_is_the_two_calendar_dates_it_claims() {
        assert_eq!(format_rfc3339(DOMAIN_MIN_MS), "1970-01-01T00:00:00.000Z");
        assert_eq!(format_rfc3339(DOMAIN_MAX_MS), "2100-01-01T00:00:00.000Z");
    }

    #[test]
    fn an_instant_inside_the_domain_is_accepted_in_both_forms() {
        assert_eq!(parsed("from", "0"), Some(DOMAIN_MIN_MS));
        assert_eq!(parsed("from", "4102444800000"), Some(DOMAIN_MAX_MS));
        assert_eq!(
            parsed("from", "2026-11-14T22:00:00.000Z"),
            Some(1_794_693_600_000)
        );
    }

    #[test]
    fn an_instant_outside_the_domain_is_refused_naming_the_parameter() {
        for (name, value) in [
            ("from", "-1"),
            ("to", "4102444800001"),
            ("from", "1900-01-01T00:00:00.000Z"),
            ("to", "2100-01-02T00:00:00.000Z"),
        ] {
            match parse_instant(name, value) {
                Ok(millis) => panic!("{name}={value} is outside the domain but parsed as {millis}"),
                Err(rejection) => {
                    assert_eq!(rejection.code, "INVALID_PARAMETER", "{name}={value}");
                    assert_eq!(rejection.status, 400, "{name}={value}");
                    assert_eq!(rejection.parameter.as_deref(), Some(name), "{name}={value}");
                    assert!(
                        rejection.message.contains("1970-01-01T00:00:00.000Z")
                            && rejection.message.contains("2100-01-01T00:00:00.000Z"),
                        "{name}={value} names both bounds: {}",
                        rejection.message
                    );
                }
            }
        }
    }
}
