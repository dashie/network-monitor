//! HTTP contract of the `serve` subcommand.
//!
//! The server binds port 0 and is driven with raw requests, so the test needs no
//! client library and no browser.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use network_monitor::shutdown::Shutdown;
use network_monitor::store::{Event, EventFilter, Store, TargetState};
use network_monitor::time::now_ms;
use network_monitor::web;

struct Server {
    stop: Shutdown,
    handle: Option<thread::JoinHandle<()>>,
    address: String,
}

impl Server {
    fn get(&self, target: &str) -> (u16, String) {
        request(
            &self.address,
            &format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n"),
        )
    }

    /// The status, the headers, and the de-chunked body of a binary response.
    fn get_raw(&self, target: &str) -> (u16, String, Vec<u8>) {
        let mut stream = TcpStream::connect(&self.address).expect("the server answers");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("the timeout is set");
        stream
            .write_all(format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n\r\n").as_bytes())
            .expect("the request is written");

        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .expect("the response is read");
        let split = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|at| at + 4)
            .expect("the headers end");

        let head = String::from_utf8_lossy(&response[..split]).to_string();
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or(0);
        (status, head, dechunk(&response[split..]))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.request();
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn request(address: &str, raw: &str) -> (u16, String) {
    try_request(address, raw).expect("the server answers")
}

fn try_request(address: &str, raw: &str) -> Option<(u16, String)> {
    let mut stream = TcpStream::connect(address).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    stream.write_all(raw.as_bytes()).ok()?;

    let mut response = String::new();
    stream.read_to_string(&mut response).ok()?;

    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    Some((status, response))
}

/// Reassemble a chunked body: each chunk is a hexadecimal size, then its bytes.
fn dechunk(data: &[u8]) -> Vec<u8> {
    let mut rest = data;
    let mut body = Vec::new();
    while let Some(end) = rest.windows(2).position(|window| window == b"\r\n") {
        let size = usize::from_str_radix(
            std::str::from_utf8(&rest[..end])
                .expect("a chunk size is ASCII")
                .trim(),
            16,
        )
        .expect("a chunk size is hexadecimal");
        rest = &rest[end + 2..];
        if size == 0 {
            break;
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
    body
}

fn event(target: &str, previous: TargetState, state: TargetState, at: i64) -> Event {
    Event {
        target: target.to_string(),
        interface: None,
        label: Some("probe".to_string()),
        previous_state: previous,
        state,
        occurred_at_ms: at,
        rtt_us: None,
    }
}

fn seeded_database() -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("events.db");
    let mut store = Store::open_rw(&path).expect("the database opens");
    let now = now_ms();
    store
        .record_transition(&event(
            "1.1.1.1",
            TargetState::Unknown,
            TargetState::Up,
            now - 3_600_000,
        ))
        .expect("the first transition is recorded");
    store
        .record_transition(&event(
            "1.1.1.1",
            TargetState::Up,
            TargetState::Down,
            now - 1_800_000,
        ))
        .expect("the second transition is recorded");
    // A second target that is still up, so the page has a running `up` stretch to state.
    store
        .record_transition(&event(
            "9.9.9.9",
            TargetState::Unknown,
            TargetState::Up,
            now - 600_000,
        ))
        .expect("the up transition is recorded");
    (directory, path)
}

fn start(database: &Path) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port");
    let address = listener
        .local_addr()
        .expect("the bound address")
        .to_string();
    let stop = Shutdown::new();
    let running = stop.clone();
    let database = database.to_path_buf();

    let handle = thread::spawn(move || {
        web::serve(listener, &database, &running).expect("the server runs");
    });

    let server = Server {
        stop,
        handle: Some(handle),
        address,
    };
    server.wait_ready();
    server
}

impl Server {
    /// `/healthz` answers only after `serve` opened the database, so this removes the
    /// race with tests that modify the database as soon as the server is up.
    fn wait_ready(&self) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            if let Some((200, _)) = try_request(
                &self.address,
                "GET /healthz HTTP/1.1\r\nHost: localhost\r\n\r\n",
            ) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("the web server did not become ready");
    }
}

#[test]
fn serves_the_page_with_a_band_per_state() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, response) = server.get("/");
    assert_eq!(status, 200);
    assert!(response.contains("<svg"));
    assert!(response.contains("1.1.1.1"));
    assert!(response.contains("#2e7d32"), "green for up");
    assert!(response.contains("#c62828"), "red for down");
    assert!(
        response.contains("<meta http-equiv=\"refresh\" content=\"30\">"),
        "the page reloads itself every 30 s"
    );
    assert!(!response.contains("wheel"), "the timeline has no zoom");
    assert!(
        response.contains("Last 10 disconnections"),
        "the page lists the latest disconnections"
    );
    assert!(
        response.contains("href=\"/export/disconnections.csv.gz\""),
        "the page offers the full export"
    );
    assert!(
        response.contains("and counting"),
        "the seeded target was down at the end of the window"
    );
}

#[test]
fn a_window_can_be_requested_in_the_url() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let now = now_ms();
    let (status, response) = server.get(&format!(
        "/?from={}&to={}",
        now - 3_000_000,
        now - 2_400_000
    ));

    assert_eq!(status, 200);
    assert!(response.contains("<p class=\"window\">"));
    assert!(response.contains("1.1.1.1"));
}

#[test]
fn the_window_links_change_the_span_and_only_one_is_current() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    for hours in [1, 2, 4, 6, 12, 24] {
        assert!(
            page.contains(&format!("href=\"/?window={hours}h\"")),
            "{hours} h is offered"
        );
    }
    assert!(
        page.find("href=\"/?window=1h\"").unwrap() < page.find("href=\"/?window=24h\"").unwrap(),
        "the links read from the narrowest window to the widest"
    );
    assert!(
        page.contains("href=\"/?window=1h\" aria-current=\"page\""),
        "the default window is one hour, marked current"
    );

    let (status, wide) = server.get("/?window=24h");
    assert_eq!(status, 200);
    assert!(wide.contains("href=\"/?window=24h\" aria-current=\"page\""));
    assert!(!wide.contains("href=\"/?window=1h\" aria-current"));

    let (status, unknown) = server.get("/?window=99h");
    assert_eq!(status, 200, "an unknown window is ignored, not refused");
    assert!(unknown.contains("href=\"/?window=1h\" aria-current=\"page\""));
}

#[test]
fn the_history_can_be_downloaded_as_a_gzipped_csv() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, body) = server.get_raw("/export/disconnections.csv.gz");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: application/gzip"));
    assert!(head.contains("Transfer-Encoding: chunked"), "streamed");
    assert!(head.contains("attachment; filename=\"disconnections-"));
    assert!(head.contains(".csv.gz\""));

    let mut csv = String::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_string(&mut csv)
        .expect("the response decompresses");

    assert!(csv.starts_with("target,label,interface,state,occurred_at,rtt_us\n"));
    assert!(csv.contains("1.1.1.1"), "got {csv}");
    assert!(
        csv.contains(",up,"),
        "the event that came back is exported too"
    );
    assert!(csv.contains(",down,"), "and the one that went down");
}

#[test]
fn every_row_shows_its_statistics_and_links_to_its_detail_page() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    assert!(page.contains("DROPS"), "the drop column has a header");
    assert!(page.contains("DOWN"), "the down-time column has a header");
    assert!(
        page.contains("UP %"),
        "the availability column has a header"
    );
    assert!(
        page.contains("AVG UP"),
        "the average up run column has a header"
    );
    assert!(
        page.contains("MAX"),
        "the longest up run column has a header"
    );
    assert!(
        page.contains("UP FOR"),
        "the column stating how long the target has been up"
    );
    assert!(
        page.contains(">10m<"),
        "the target that is up states the ten minutes of its running stretch"
    );
    assert!(page.contains("href=\"/target?target=1.1.1.1\""));

    let (status, detail) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);
    assert!(detail.contains("<!doctype html>"));
    assert!(
        detail.contains("<svg"),
        "one bar per day of the last 30 days"
    );
    assert!(detail.contains("1 disconnections"));
    assert!(
        detail.contains("Disconnections per day") && detail.contains("Day by day"),
        "the chart and the table are titled"
    );
    assert!(
        detail.matches("<tr><td>").count() >= 30,
        "one row per day of the window, like the chart"
    );
    assert!(detail.contains("<th class=\"num\">down time</th>"));
    assert!(
        detail.contains(">avg up</th>"),
        "every day states the average of its up runs, named as on the timeline"
    );
    assert!(
        detail.contains("<span class=\"tag\">last 24 h</span>"),
        "the day still running states the last 24 hours, tagged as such"
    );
    assert!(
        detail.contains("up run in the last 30 days"),
        "the third KPI states the sample it is built on"
    );
    assert!(
        detail.contains("avg up run in 30 days · max "),
        "the third KPI names what it averages and states the longest run too"
    );
    assert!(
        detail.contains("<td class=\"num\">30m</td>"),
        "the target has been down for half an hour"
    );
    assert!(
        !detail.contains("class=\"since\""),
        "the target is down, so there is no running up stretch to state"
    );
    assert!(detail.contains("href=\"/\""), "a way back to the timeline");
    assert!(
        detail.find("href=\"/\"").unwrap() < detail.find("<svg").unwrap(),
        "the way back is at the top, before the chart"
    );
}

#[test]
fn the_detail_page_offers_the_periods_and_keeps_thirty_days_as_the_default() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);
    assert!(page.contains("last 30 days"));
    for days in [30, 60, 90, 180, 360] {
        assert!(
            page.contains(&format!("href=\"/target?target=1.1.1.1&days={days}d\"")),
            "the {days}-day period is offered"
        );
    }
    assert!(
        page.contains("days=30d\" aria-current=\"page\""),
        "thirty days is the period marked as current"
    );
    assert!(
        page.matches("<tr><td>").count() >= 30,
        "thirty days is the period shown"
    );

    let (status, longer) = server.get("/target?target=1.1.1.1&days=90d");
    assert_eq!(status, 200);
    assert!(longer.contains("last 90 days"));
    assert!(longer.contains("days=90d\" aria-current=\"page\""));
    assert!(
        longer.matches("<tr><td>").count() >= 90,
        "the chosen period is the one drawn and listed"
    );

    let (status, unknown) = server.get("/target?target=1.1.1.1&days=45d");
    assert_eq!(status, 200);
    assert!(
        unknown.contains("last 30 days"),
        "an unknown period falls back to the default instead of failing"
    );
}

#[test]
fn a_target_can_be_opened_by_its_label() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, detail) = server.get("/target?target=probe");
    assert_eq!(status, 200);
    assert!(detail.contains("1.1.1.1"));
}

#[test]
fn an_unknown_or_missing_target_is_refused() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, response) = server.get("/target?target=8.8.8.8");
    assert_eq!(status, 404);
    assert!(response.contains("unknown target"));

    let (status, _) = server.get("/target");
    assert_eq!(status, 404);
}

#[test]
fn unknown_routes_and_methods_are_refused() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, _) = server.get("/nope");
    assert_eq!(status, 404);

    let (status, _) = request(
        &server.address,
        "POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: 0\r\n\r\n",
    );
    assert_eq!(status, 405);
}

#[test]
fn a_missing_database_is_reported_per_request() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    std::fs::remove_file(&database).expect("the database is removed");
    let (status, response) = server.get("/");

    assert_eq!(status, 503);
    assert!(response.contains("does not exist"));
}

#[test]
fn serving_never_writes_to_the_database() {
    let (_directory, database) = seeded_database();
    let before = {
        let store = Store::open_ro(&database).expect("the database opens");
        store
            .events(&EventFilter::default())
            .expect("the history is read")
            .len()
    };

    let server = start(&database);
    for _ in 0..3 {
        let (status, _) = server.get("/");
        assert_eq!(status, 200);
    }

    let after = {
        let store = Store::open_ro(&database).expect("the database opens");
        store
            .events(&EventFilter::default())
            .expect("the history is read")
            .len()
    };
    assert_eq!(before, after);
}
