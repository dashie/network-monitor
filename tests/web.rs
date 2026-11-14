//! HTTP contract of the `serve` subcommand.
//!
//! The server binds port 0 and is driven with raw requests, so the test needs no
//! client library and no browser.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use network_monitor::cli::Compression;
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

    /// The status, the headers, and the de-chunked body of a streamed response.
    fn get_raw(&self, target: &str) -> (u16, String, Vec<u8>) {
        let (status, head, body) = self.get_bytes(target);
        (status, head, dechunk(&body))
    }

    /// The status, the headers, and the body as it arrived, for a response whose
    /// length is known in advance.
    fn get_bytes(&self, target: &str) -> (u16, String, Vec<u8>) {
        self.get_bytes_with(target, "")
    }

    /// The same, with extra request headers such as `Accept-Encoding`.
    fn get_bytes_with(&self, target: &str, headers: &str) -> (u16, String, Vec<u8>) {
        let mut stream = TcpStream::connect(&self.address).expect("the server answers");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("the timeout is set");
        stream
            .write_all(
                format!("GET {target} HTTP/1.1\r\nHost: localhost\r\n{headers}\r\n").as_bytes(),
            )
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
        (status, head, response[split..].to_vec())
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
        cause: None,
        cause_message: None,
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
    start_with(database, Compression::Auto)
}

fn start_with(database: &Path, compression: Compression) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port");
    let address = listener
        .local_addr()
        .expect("the bound address")
        .to_string();
    let stop = Shutdown::new();
    let running = stop.clone();
    let database = database.to_path_buf();

    let handle = thread::spawn(move || {
        web::serve(listener, &database, &running, compression).expect("the server runs");
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
        response.contains("Last 10 faults"),
        "the page lists the latest faults"
    );
    assert!(
        response
            .contains("<a class=\"download\" href=\"/export/faults.csv.gz\">Download History</a>"),
        "the page offers the full export under a plain label"
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

    let (status, head, body) = server.get_raw("/export/faults.csv.gz");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: application/gzip"));
    assert!(head.contains("Transfer-Encoding: chunked"), "streamed");
    assert!(head.contains("attachment; filename=\"faults-"));
    assert!(head.contains(".csv.gz\""));

    let mut csv = String::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_string(&mut csv)
        .expect("the response decompresses");

    assert!(
        csv.starts_with("target,label,interface,state,occurred_at,rtt_us,cause,cause_message\n")
    );
    assert!(csv.contains("1.1.1.1"), "got {csv}");
    assert!(
        csv.contains(",up,"),
        "the event that came back is exported too"
    );
    assert!(csv.contains(",down,"), "and the one that went down");
}

/// The page in its compressed form, decompressed by the test.
fn gunzip(body: &[u8]) -> String {
    let mut text = String::new();
    flate2::read::GzDecoder::new(body)
        .read_to_string(&mut text)
        .expect("a gzip body decompresses");
    text
}

#[test]
fn a_page_is_gzipped_when_the_client_accepts_it_and_only_then() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, plain) = server.get_bytes("/");
    assert_eq!(status, 200);
    assert!(
        !head.contains("Content-Encoding"),
        "a client that says nothing gets the page as it is"
    );
    assert!(head.contains("Vary: Accept-Encoding"), "caches keep both");

    let (status, head, compressed) = server.get_bytes_with("/", "Accept-Encoding: gzip\r\n");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Encoding: gzip"));
    assert!(head.contains("Vary: Accept-Encoding"));
    let decoded = gunzip(&compressed);
    assert!(
        decoded.starts_with("<!doctype html>"),
        "the body decompresses to the page"
    );
    assert!(decoded.contains("Network Monitor"));
    assert!(decoded.contains("Last 10 faults"));
    assert!(
        compressed.len() * 2 < plain.len(),
        "the page is much smaller: {} against {}",
        compressed.len(),
        plain.len()
    );
}

#[test]
fn the_detail_page_is_gzipped_too() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, compressed) =
        server.get_bytes_with("/target?target=1.1.1.1", "Accept-Encoding: gzip\r\n");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Encoding: gzip"));
    assert!(gunzip(&compressed).contains("Disconnections per day"));
}

#[test]
fn a_refused_coding_or_an_operator_switch_leaves_the_page_plain() {
    let (_directory, database) = seeded_database();

    let server = start(&database);
    let (_, head, _) = server.get_bytes_with("/", "Accept-Encoding: gzip;q=0\r\n");
    assert!(
        !head.contains("Content-Encoding"),
        "a zero quality means the coding is not wanted"
    );

    let off = start_with(&database, Compression::Off);
    let (status, head, body) = off.get_bytes_with("/", "Accept-Encoding: gzip\r\n");
    assert_eq!(status, 200);
    assert!(
        !head.contains("Content-Encoding"),
        "with the switch off the proxy compresses"
    );
    assert!(String::from_utf8_lossy(&body).contains("Network Monitor"));
}

#[test]
fn the_archive_is_not_compressed_twice() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, _body) =
        server.get_bytes_with("/export/faults.csv.gz", "Accept-Encoding: gzip\r\n");
    assert_eq!(status, 200);
    assert!(
        !head.contains("Content-Encoding"),
        "the archive is already a gzip file, not a compressed response body"
    );
}

#[test]
fn the_old_export_path_still_answers_and_points_at_the_new_one() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, body) = server.get_bytes_with("/export/disconnections.csv.gz", "");
    assert_eq!(status, 308, "the address moved, it was not dropped");
    assert!(head.contains("Location: /export/faults.csv.gz"));
    assert!(
        !head.contains("Cache-Control"),
        "a permanent redirect is meant to be remembered, not forbidden to cache"
    );
    assert!(body.is_empty(), "a redirect carries no body");
}

#[test]
fn every_row_shows_its_statistics_and_links_to_its_detail_page() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    assert!(page.contains("DROPS"), "the drop column has a header");
    assert!(!page.contains(">DOWN<"), "the down-time column is gone");
    assert!(!page.contains("UP %"), "the availability column is gone");
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
    assert!(detail.contains("1 faults"));
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
    assert!(
        detail.contains("<a class=\"back-button\" href=\"/\">"),
        "a way back to the timeline, whatever page the reader came from"
    );
    assert!(
        detail.find("class=\"back-button\"").unwrap() < detail.find("<svg").unwrap(),
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

/// What a page is showing and the selector that changes it share one line, with the
/// selector at its end.
#[test]
fn the_page_context_and_its_selector_share_one_line() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    assert!(page.contains("<div class=\"toolbar\">\n<p class=\"window\">"));
    assert!(page.contains("<nav class=\"windows\" aria-label=\"Time window\">"));

    let (status, detail) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);
    assert!(detail.contains("<div class=\"toolbar\">"));
    assert!(detail.contains("<a class=\"back-button\" href=\"/\">"));
    assert!(
        !detail.contains("class=\"hint\""),
        "the masthead states the target, so the toolbar needs no address of its own"
    );
    let toolbar_start = detail.find("<div class=\"toolbar\">").expect("a toolbar");
    let toolbar_end = detail[toolbar_start..]
        .find("</div>")
        .expect("the toolbar closes");
    let toolbar = &detail[toolbar_start..toolbar_start + toolbar_end];
    assert!(
        toolbar.contains("<nav class=\"windows\" aria-label=\"Period\">"),
        "the period selector shares the line with what the page is showing: {toolbar}"
    );
    assert!(
        toolbar.contains("class=\"back-button\""),
        "the way back opens that line: {toolbar}"
    );
    let masthead_end = detail.find("</header>").expect("a closed masthead");
    assert!(
        detail[..masthead_end].contains("<div class=\"target-info\">"),
        "the target of the page closes the masthead, before the toolbar"
    );
}

/// Both pages close with the same short manual: a rule narrower than the content, then
/// the notes. The timeline states nothing about itself above the chart.
#[test]
fn both_pages_close_with_a_short_manual() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    assert!(
        !page.contains("class=\"hint\""),
        "the top of the page carries no explanation"
    );
    assert!(page.contains("<footer>\n<hr>\n"));
    assert!(page.contains("A lane is one target over the chosen window"));
    assert!(page.contains("Times follow the time zone of your browser"));

    let (status, detail) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);
    assert!(detail.contains("<footer>\n<hr>\n"));
    assert!(detail.contains("Each bar is one day of the period"));
    assert!(detail.contains("Times follow the time zone of your browser"));
    assert!(
        !detail.contains("times in your time zone"),
        "the manual under the tables states the time zone, not the masthead"
    );
}

/// Both pages open with the mark of the application and advertise it to the browser,
/// and the server serves it in the two formats they name: the scalable one and the
/// PNG fallback.
#[test]
fn both_pages_show_the_mark_the_server_serves() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    assert!(page.contains(r#"<img class="mark" src="/icon.svg" alt="" width="36" height="36">"#));
    assert!(page.contains(r#"<h1>Network Monitor</h1>"#));
    assert!(page.contains(r#"<p class="tagline">Reachability monitoring</p>"#));
    assert!(page.contains(r#"type="image/svg+xml" href="/icon.svg""#));
    assert!(page.contains(r#"type="image/png" sizes="32x32" href="/icon.png""#));

    let (status, detail) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);
    assert!(
        detail.contains(r#"<div class="brand"><img class="mark" src="/icon.svg""#),
        "the detail page opens with the same mark, inside the brand block"
    );
    assert!(detail.contains(r#"href="/icon.svg""#));

    let (status, head, body) = server.get_bytes("/icon.svg");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: image/svg+xml"));
    assert!(body.starts_with(b"<svg"));

    let (status, head, body) = server.get_bytes("/icon.png");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: image/png"));
    assert_eq!(&body[..8], b"\x89PNG\r\n\x1a\n");
}

/// The pages link their stylesheet and their script, and the server serves both as files:
/// the browser names them in devtools with their own lines, instead of showing them as
/// text inside the page.
#[test]
fn the_pages_link_the_stylesheet_and_the_script_the_server_serves() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for path in ["/", "/target?target=1.1.1.1"] {
        let (status, page) = server.get(path);
        assert_eq!(status, 200);
        assert!(
            page.contains(r#"<link rel="stylesheet" href="/style.css">"#),
            "{path} links its stylesheet"
        );
        assert!(
            page.contains(r#"<script src="/app.js"></script>"#),
            "{path} loads its script"
        );
        assert!(
            !page.contains("<style>"),
            "{path} carries no stylesheet of its own"
        );
        assert!(
            !page.contains("<script>\n"),
            "{path} carries no script of its own"
        );
    }

    let (status, head, body) = server.get_bytes("/style.css");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: text/css; charset=utf-8"));
    assert!(String::from_utf8_lossy(&body).contains(".masthead"));

    let (status, head, body) = server.get_bytes("/app.js");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: text/javascript; charset=utf-8"));
    assert!(String::from_utf8_lossy(&body).contains("Intl.DateTimeFormat"));
}

/// The identity block at the head of the page — the mark, the name, and the tagline —
/// is the same on every page and always opens the masthead, flush left. A page may state
/// what it is showing at its right end, but the identity block never moves.
#[test]
fn every_page_opens_with_the_same_brand_block() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    let (status, detail) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);

    let brand = |html: &str| {
        let start = html.find(r#"<div class="brand">"#).expect("a brand block");
        let rest = &html[start..];
        let end = rest
            .find(r#"<div class="target-info">"#)
            .or_else(|| rest.find("</header>"))
            .expect("the end of the brand block");
        rest[..end].trim_end().to_string()
    };
    assert_eq!(
        brand(&page),
        brand(&detail),
        "one identity block on every page"
    );

    let after_header = |html: &str| {
        let start = html
            .find(r#"<header class="masthead">"#)
            .expect("a masthead");
        html[start..]
            .trim_start_matches(r#"<header class="masthead">"#)
            .trim_start()
            .to_string()
    };
    assert!(
        after_header(&page).starts_with(r#"<div class="brand">"#),
        "the identity block opens the masthead, without anything before it"
    );
    assert!(
        after_header(&detail).starts_with(r#"<div class="brand">"#),
        "the target of the detail page joins the masthead after the identity block"
    );

    let shared = brand(&page);
    assert!(shared.contains(r#"<img class="mark" src="/icon.svg""#));
    assert!(shared.contains("<h1>Network Monitor</h1>"));
    assert!(shared.contains(r#"<p class="tagline">Reachability monitoring</p>"#));
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

/// A key that carries a scheme and a query survives both the link the page generates and
/// the `?target=` round trip that opens its detail page.
#[test]
fn a_url_shaped_key_survives_the_link_and_the_query() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("events.db");
    let key = "https://example.com/a?b=1&c=2";
    {
        let mut store = Store::open_rw(&database).unwrap();
        store
            .record_transition(&event(
                key,
                TargetState::Unknown,
                TargetState::Up,
                now_ms() - 60_000,
            ))
            .expect("the transition is recorded");
    }
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    assert!(
        page.contains("target=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1%26c%3D2"),
        "the lane link must encode the key"
    );

    let (status, detail) =
        server.get("/target?target=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1%26c%3D2");
    assert_eq!(status, 200, "the encoded key must open its target");
    assert!(
        detail.contains("https://example.com/a?b=1&amp;c=2"),
        "the address of the target is shown escaped"
    );
}
