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

    let legend_start = response.find("<div class=\"legend\">").expect("a legend");
    let legend = &response[legend_start..];
    let legend = &legend[..legend.find("</div>").expect("the legend closes")];
    let order = [
        legend.find(">up<").expect("up in the legend"),
        legend.find(">down<").expect("down in the legend"),
        legend
            .find(">fault too short to draw<")
            .expect("the short fault in the legend"),
        legend.find(">unknown<").expect("unknown in the legend"),
    ];
    assert!(
        order.windows(2).all(|pair| pair[0] < pair[1]),
        "the legend reads up, down, fault too short to draw, unknown: {legend}"
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

/// The download offered by a detail page carries that target's history only, and says so
/// in the name of the file.
#[test]
fn the_download_of_a_target_holds_only_that_target() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, body) = server.get_raw("/export/faults.csv.gz?target=9.9.9.9");
    assert_eq!(status, 200);
    assert!(
        head.contains("attachment; filename=\"faults-probe-"),
        "the file is named after the target: {head}"
    );
    assert!(head.contains(".csv.gz\""));

    let mut csv = String::new();
    flate2::read::GzDecoder::new(&body[..])
        .read_to_string(&mut csv)
        .expect("the response decompresses");

    assert!(
        csv.starts_with("target,label,interface,state,occurred_at,rtt_us,cause,cause_message\n"),
        "the header is the one the import reads"
    );
    assert!(csv.contains("9.9.9.9"), "got {csv}");
    assert!(
        !csv.contains("1.1.1.1"),
        "the history of the other target is not in the file: {csv}"
    );
}

#[test]
fn the_download_of_an_unknown_target_is_refused() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, _) = server.get("/export/faults.csv.gz?target=no-such-target");
    assert_eq!(status, 404, "an unknown target is answered plainly");
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
        detail.contains("<a class=\"download\" href=\"/export/faults.csv.gz?target=1.1.1.1\">"),
        "the download of the detail page covers that target only"
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
    assert!(
        page.contains("href=\"/api/v1/docs\""),
        "the footer leads to the API reference"
    );

    let (status, detail) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);
    assert!(detail.contains("<footer>\n<hr>\n"));
    assert!(detail.contains("Each bar is one day of the period"));
    assert!(detail.contains("Times follow the time zone of your browser"));
    assert!(
        detail.contains("href=\"/api/v1/docs\""),
        "the footer leads to the API reference"
    );
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
        detail.contains(r#"<a class="brand" href="/"><img class="mark" src="/icon.svg""#),
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
        let start = html
            .find(r#"<a class="brand" href="/">"#)
            .expect("a brand block");
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
        after_header(&page).starts_with(r#"<a class="brand" href="/">"#),
        "the identity block opens the masthead, without anything before it"
    );
    assert!(
        after_header(&detail).starts_with(r#"<a class="brand" href="/">"#),
        "the target of the detail page joins the masthead after the identity block"
    );

    let shared = brand(&page);
    assert!(shared.contains(r#"<img class="mark" src="/icon.svg""#));
    assert!(shared.contains("<h1>Network Monitor</h1>"));
    assert!(shared.contains(r#"<p class="tagline">Reachability monitoring</p>"#));
}

/// The identity block is the way home: on every page, the reference included, a selection
/// anywhere on the mark, the name, or the tagline returns to the timeline.
#[test]
fn the_identity_block_returns_to_the_timeline() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for path in ["/", "/target?target=1.1.1.1", "/api/v1/docs"] {
        let (status, page) = server.get(path);
        assert_eq!(status, 200, "{path} answers");
        assert!(
            page.contains(r#"<a class="brand" href="/">"#),
            "{path} opens its identity block with a link to the timeline"
        );
        let start = page
            .find(r#"<a class="brand" href="/">"#)
            .expect("a brand block");
        let block = &page[start..];
        let end = block.find("</a>").expect("the identity block closes");
        let block = &block[..end];
        assert!(
            block.contains(r#"<img class="mark""#) && block.contains("<h1>Network Monitor</h1>"),
            "the link covers the mark and the name: {block}"
        );
    }
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

// ---- the JSON API under /api/v1 ----

impl Server {
    /// The status and the body of a JSON answer, the headers left out.
    fn json(&self, path: &str) -> (u16, String) {
        let (status, head, body) = self.get_bytes(path);
        let body = if head.contains("Transfer-Encoding: chunked") {
            dechunk(&body)
        } else {
            body
        };
        (
            status,
            String::from_utf8(body).expect("a JSON answer is UTF-8"),
        )
    }
}

/// The integer a rendered record states for `name`.
fn field_number(body: &str, name: &str) -> i64 {
    let needle = format!("\"{name}\": ");
    let start = body.find(&needle).expect("the field is there") + needle.len();
    body[start..]
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '-')
        .collect::<String>()
        .parse()
        .expect("a number")
}

#[test]
fn every_api_route_answers_an_object_with_data_and_meta() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for path in ["/api/v1/targets", "/api/v1/faults", "/api/v1/faults/daily"] {
        let (status, body) = server.json(path);
        assert_eq!(status, 200, "{path} answered {status}: {body}");
        assert!(
            body.starts_with("{\n  \"data\": "),
            "an array is never the root of {path}: {body}"
        );
        assert!(body.contains("\"limit\": 100, \"offset\": 0"), "got {body}");
        assert!(body.contains("\"total\": "), "got {body}");
        assert!(body.contains("\"filters\": {"), "got {body}");
        assert!(body.contains("\"generatedAt\": \""), "got {body}");
    }
}

#[test]
fn the_targets_state_every_property_and_page_by_limit_and_offset() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server.json("/api/v1/targets");
    assert_eq!(status, 200);
    for key in [
        "\"target\"",
        "\"label\"",
        "\"interface\"",
        "\"state\"",
        "\"stateSince\"",
        "\"stateSinceMs\"",
        "\"firstSeenAt\"",
        "\"firstSeenAtMs\"",
        "\"lastRttUs\"",
        "\"upForMs\"",
    ] {
        assert!(body.contains(key), "{key} is missing from {body}");
    }
    assert!(body.contains("\"total\": 2"), "got {body}");
    assert!(body.contains("\"target\": \"1.1.1.1\""), "got {body}");
    assert!(body.contains("\"target\": \"9.9.9.9\""), "got {body}");

    let (_, page) = server.json("/api/v1/targets?limit=1&offset=1");
    assert!(page.contains("\"total\": 2"), "the total is the whole list");
    assert!(page.contains("\"limit\": 1, \"offset\": 1"), "got {page}");
    assert!(page.contains("\"target\": \"9.9.9.9\""), "got {page}");
    assert!(!page.contains("\"target\": \"1.1.1.1\""), "got {page}");
}

#[test]
fn the_faults_state_their_lifecycle_and_keep_from_inclusive_and_to_exclusive() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server.json("/api/v1/faults");
    assert_eq!(status, 200);
    for key in [
        "\"id\"",
        "\"target\"",
        "\"label\"",
        "\"interface\"",
        "\"occurredAt\"",
        "\"occurredAtMs\"",
        "\"cause\"",
        "\"causeMessage\"",
        "\"endedAt\"",
        "\"endedAtMs\"",
        "\"durationMs\"",
        "\"ongoing\"",
    ] {
        assert!(body.contains(key), "{key} is missing from {body}");
    }
    assert!(body.contains("\"total\": 1"), "got {body}");
    assert!(body.contains("\"ongoing\": true"), "still down: {body}");
    assert!(body.contains("\"endedAt\": null"), "no return: {body}");
    assert!(body.contains("\"durationMs\": null"), "open fault: {body}");

    let at = field_number(&body, "occurredAtMs");
    let (_, from) = server.json(&format!("/api/v1/faults?from={at}"));
    assert!(from.contains("\"total\": 1"), "from is inclusive: {from}");
    let (_, to) = server.json(&format!("/api/v1/faults?to={at}"));
    assert!(to.contains("\"total\": 0"), "to is exclusive: {to}");
}

#[test]
fn a_fault_filter_resolves_the_label_as_well_as_the_key() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (_, by_key) = server.json("/api/v1/faults?target=1.1.1.1");
    let (_, by_label) = server.json("/api/v1/faults?target=probe");
    assert!(by_key.contains("\"total\": 1"), "got {by_key}");
    assert_eq!(
        field_number(&by_key, "id"),
        field_number(&by_label, "id"),
        "the label names the same target"
    );
    assert!(
        by_label.contains("\"filters\": {\"target\": \"1.1.1.1\""),
        "the meta echoes the resolved key: {by_label}"
    );

    let (_, other) = server.json("/api/v1/faults?target=9.9.9.9");
    assert!(other.contains("\"total\": 0"), "no fault there: {other}");
}

#[test]
fn the_daily_totals_state_the_faults_the_down_time_and_the_up_runs() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server.json("/api/v1/faults/daily");
    assert_eq!(status, 200);
    for key in [
        "\"target\"",
        "\"day\"",
        "\"dayStart\"",
        "\"dayStartMs\"",
        "\"faults\"",
        "\"downMs\"",
        "\"avgUpRunMs\"",
        "\"longestUpRunMs\"",
        "\"partial\"",
    ] {
        assert!(body.contains(key), "{key} is missing from {body}");
    }
    assert!(body.contains("\"faults\": 1"), "the seeded fault: {body}");
    assert!(
        body.contains("\"partial\": true"),
        "the UTC day still running is marked: {body}"
    );
    assert!(
        body.contains("\"partial\": false"),
        "the closed days of the window: {body}"
    );
    assert!(
        body.contains("\"filters\": {\"target\": null, \"from\": \""),
        "the meta echoes the window it applied: {body}"
    );

    let (_, quiet) = server.json("/api/v1/faults/daily?target=9.9.9.9");
    assert!(
        !quiet.contains("\"faults\": 1"),
        "a target with no fault states zeros: {quiet}"
    );
}

#[test]
fn the_api_refuses_a_parameter_it_does_not_understand() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for (query, code) in [
        ("limit=0", "INVALID_PARAMETER"),
        ("limit=1001", "INVALID_PARAMETER"),
        ("limit=abc", "INVALID_PARAMETER"),
        ("offset=-1", "INVALID_PARAMETER"),
        ("bogus=1", "INVALID_PARAMETER"),
        ("from=yesterday", "INVALID_PARAMETER"),
        (
            "from=2026-11-15T00:00:00Z&to=2026-11-14T00:00:00Z",
            "INVALID_RANGE",
        ),
    ] {
        let (status, body) = server.json(&format!("/api/v1/faults?{query}"));
        assert_eq!(status, 400, "{query} answered {status}: {body}");
        assert!(body.contains(code), "{query} states {code}: {body}");
        assert!(body.contains("\"status\": 400"), "got {body}");
    }

    let (status, body) = server.json("/api/v1/faults?limit=0");
    assert_eq!(status, 400);
    assert!(
        body.contains("\"parameter\": \"limit\""),
        "the refused parameter is named: {body}"
    );

    let (status, body) = server.json("/api/v1/targets?target=1.1.1.1");
    assert_eq!(status, 400, "the route states its own parameters: {body}");
    assert!(body.contains("\"parameter\": \"target\""), "got {body}");

    let (status, body) = server.json("/api/v1/nope");
    assert_eq!(status, 404, "got {body}");
    assert!(body.contains("\"code\": \"NOT_FOUND\""), "got {body}");
}

#[test]
fn an_unknown_target_is_a_404_that_names_it() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for route in ["/api/v1/faults", "/api/v1/faults/daily"] {
        let (status, body) = server.json(&format!("{route}?target=nowhere"));
        assert_eq!(status, 404, "{route} answered {status}: {body}");
        assert!(body.contains("\"code\": \"UNKNOWN_TARGET\""), "got {body}");
        assert!(body.contains("\"parameter\": \"target\""), "got {body}");
        assert!(body.contains("nowhere"), "the answer names it: {body}");
    }
}

#[test]
fn the_api_states_cors_and_answers_the_preflight() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, response) = request(
        &server.address,
        "OPTIONS /api/v1/targets HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status, 204);
    assert!(
        response.contains("Access-Control-Allow-Origin: *"),
        "{response}"
    );
    assert!(
        response.contains("Access-Control-Allow-Methods: GET, HEAD, OPTIONS"),
        "{response}"
    );
    assert!(
        response.contains("Access-Control-Max-Age: 600"),
        "{response}"
    );

    let (_, plain) = server.get("/api/v1/targets");
    assert!(
        plain.contains("Access-Control-Allow-Origin: *"),
        "every answer is readable from another origin"
    );

    let (status, response) = request(
        &server.address,
        "POST /api/v1/targets HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status, 405);
    assert!(response.contains("Allow: GET, HEAD, OPTIONS"), "{response}");
    assert!(
        response.contains("\"code\": \"METHOD_NOT_ALLOWED\""),
        "{response}"
    );
}

#[test]
fn an_api_answer_is_gzipped_only_when_the_client_accepts_it() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, plain) = server.get_bytes("/api/v1/targets");
    assert_eq!(status, 200);
    assert!(!head.contains("Content-Encoding"), "{head}");
    assert!(head.contains("Vary: Accept-Encoding"), "{head}");
    assert!(head.contains("Content-Type: application/json; charset=utf-8"));

    assert!(String::from_utf8(dechunk(&plain))
        .expect("the plain answer is UTF-8")
        .contains("\"total\": 2"));

    let (status, head, compressed) =
        server.get_bytes_with("/api/v1/targets", "Accept-Encoding: gzip\r\n");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Encoding: gzip"), "{head}");
    assert!(head.contains("Transfer-Encoding: chunked"), "{head}");
    let decoded = gunzip(&dechunk(&compressed));
    assert!(
        decoded.starts_with("{\n  \"data\": ["),
        "the body decompresses to the document: {decoded}"
    );
    assert!(decoded.contains("\"total\": 2"), "got {decoded}");
}

#[test]
fn a_head_of_an_api_route_states_the_stream_and_sends_no_body() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, response) = request(
        &server.address,
        "HEAD /api/v1/targets HTTP/1.1\r\nHost: localhost\r\n\r\n",
    );
    assert_eq!(status, 200);
    assert!(
        response.contains("Content-Type: application/json; charset=utf-8"),
        "{response}"
    );
    assert!(
        response.contains("Transfer-Encoding: chunked"),
        "{response}"
    );
    assert!(
        !response.contains("Content-Length"),
        "a streamed answer states no length: {response}"
    );
    let (_, body) = response.split_once("\r\n\r\n").expect("the headers end");
    assert!(body.is_empty(), "HEAD sends no body");
}

#[test]
fn the_openapi_document_names_the_routes_and_every_property_they_answer() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    const TARGET_KEYS: [&str; 10] = [
        "target",
        "label",
        "interface",
        "state",
        "stateSince",
        "stateSinceMs",
        "firstSeenAt",
        "firstSeenAtMs",
        "lastRttUs",
        "upForMs",
    ];
    const FAULT_KEYS: [&str; 12] = [
        "id",
        "target",
        "label",
        "interface",
        "occurredAt",
        "occurredAtMs",
        "cause",
        "causeMessage",
        "endedAt",
        "endedAtMs",
        "durationMs",
        "ongoing",
    ];
    const DAY_KEYS: [&str; 9] = [
        "target",
        "day",
        "dayStart",
        "dayStartMs",
        "faults",
        "downMs",
        "avgUpRunMs",
        "longestUpRunMs",
        "partial",
    ];

    let (status, head, bytes) = server.get_bytes("/api/v1/openapi.json");
    assert_eq!(status, 200);
    assert!(head.contains("Content-Type: application/json; charset=utf-8"));
    let document = String::from_utf8(bytes).expect("the document is UTF-8");
    assert!(
        document.contains("\"openapi\": \"3.1.0\""),
        "got {document}"
    );
    for schema in [
        "TargetEnvelope",
        "FaultEnvelope",
        "DayTotalEnvelope",
        "Meta",
        "ErrorEnvelope",
    ] {
        assert!(document.contains(schema), "{schema} is missing");
    }

    let expected: [(&str, &[&str]); 3] = [
        ("/api/v1/targets", &TARGET_KEYS),
        ("/api/v1/faults", &FAULT_KEYS),
        ("/api/v1/faults/daily", &DAY_KEYS),
    ];
    for (path, keys) in expected {
        assert!(
            document.contains(&format!("\"{path}\"")),
            "the document does not name {path}"
        );
        let (status, body) = server.json(path);
        assert_eq!(status, 200, "the documented {path} must answer");
        for key in keys {
            assert!(
                document.contains(&format!("\"{key}\"")),
                "the document does not state {key} of {path}"
            );
            assert!(
                body.contains(&format!("\"{key}\":")),
                "{path} answers no {key}: {body}"
            );
        }
    }
}

#[test]
fn the_docs_page_renders_the_contract_the_api_serves() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, bytes) = server.get_bytes("/api/v1/docs");
    assert_eq!(status, 200);
    assert!(
        head.contains("Content-Type: text/html; charset=utf-8"),
        "{head}"
    );
    let page = String::from_utf8(bytes).expect("the page is UTF-8");
    assert!(
        page.contains("/api/v1/openapi.json"),
        "the page renders the contract the API serves: {page}"
    );
    assert!(
        page.contains("/style.css"),
        "the reference opens with the masthead of the page: {page}"
    );

    let files: [(&str, &str); 4] = [
        ("/style.css", "text/css; charset=utf-8"),
        ("/api/v1/docs/swagger-ui.css", "text/css; charset=utf-8"),
        ("/api/v1/docs/theme.css", "text/css; charset=utf-8"),
        (
            "/api/v1/docs/swagger-ui-bundle.js",
            "text/javascript; charset=utf-8",
        ),
    ];
    for (file, content_type) in files {
        assert!(page.contains(file), "the page loads {file}");
        let (status, head, body) = server.get_bytes(file);
        assert_eq!(status, 200, "{file} is served");
        assert!(head.contains(content_type), "{file}: {head}");
        assert!(!body.is_empty(), "{file} is not empty");
    }

    // The bundle is larger than a socket buffer, which is what a non-blocking socket
    // would cut short: the whole body must arrive.
    let bundle = include_str!("../assets/api/swagger-ui/swagger-ui-bundle.js");
    let (_, _, body) = server.get_bytes("/api/v1/docs/swagger-ui-bundle.js");
    assert_eq!(
        body.len(),
        bundle.len(),
        "a body larger than the socket buffer arrives whole"
    );

    assert_eq!(
        server.get("/api/v1/docs/missing.js").0,
        404,
        "an unknown file of the reference is refused"
    );
}

/// The reference is a megabyte of script: it must travel compressed, like every page.
#[test]
fn the_reference_script_is_gzipped_when_the_client_accepts_it() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, compressed) = server.get_bytes_with(
        "/api/v1/docs/swagger-ui-bundle.js",
        "Accept-Encoding: gzip\r\n",
    );
    assert_eq!(status, 200);
    assert!(head.contains("Content-Encoding: gzip"), "{head}");
    let text = gunzip(&compressed);
    assert!(
        text.contains("SwaggerUIBundle"),
        "the bundle defines the renderer the page calls"
    );
    assert!(
        compressed.len() < text.len() / 2,
        "a bundle of {} bytes gzips to {}",
        text.len(),
        compressed.len()
    );
}

/// A history of `faults` faults: an `up` and a `down` per fault, one second apart.
fn database_with_faults(faults: usize) -> (tempfile::TempDir, PathBuf) {
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("events.db");
    let mut store = Store::open_rw(&path).expect("the database opens");
    let mut previous = TargetState::Unknown;
    for step in 0..faults * 2 {
        let state = if step % 2 == 0 {
            TargetState::Up
        } else {
            TargetState::Down
        };
        store
            .record_transition(&event(
                "1.1.1.1",
                previous,
                state,
                1_000 + (step as i64) * 1_000,
            ))
            .expect("the transition is recorded");
        previous = state;
    }
    (directory, path)
}

#[test]
fn a_data_route_is_streamed_without_a_length() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, head, body) = server.get_bytes("/api/v1/targets");
    assert_eq!(status, 200);
    assert!(head.contains("Transfer-Encoding: chunked"), "{head}");
    assert!(
        !head.contains("Content-Length"),
        "a streamed answer states no length: {head}"
    );

    let text = String::from_utf8(dechunk(&body)).expect("the answer is UTF-8");
    assert!(text.starts_with("{\n  \"data\": [\n"), "got {text}");
    assert!(text.ends_with("}\n"), "the document closes: {text}");
    assert!(text.contains("\"total\": 2"), "got {text}");
    assert!(text.contains("\"target\": \"1.1.1.1\""), "got {text}");
}

#[test]
fn a_page_of_faults_larger_than_one_keyset_read_comes_back_whole() {
    const FAULTS: usize = 250;
    let (_directory, database) = database_with_faults(FAULTS);
    let server = start(&database);

    let (status, body) = server.json(&format!("/api/v1/faults?limit={FAULTS}"));
    assert_eq!(status, 200, "got {body}");
    assert!(
        body.contains(&format!("\"total\": {FAULTS}")),
        "the total is the whole history: {body}"
    );

    let ids: Vec<i64> = body
        .split("{\"id\": ")
        .skip(1)
        .filter_map(|rest| rest.split(',').next()?.trim().parse().ok())
        .collect();
    assert_eq!(ids.len(), FAULTS, "every fault is written once");
    assert!(
        ids.windows(2).all(|pair| pair[0] > pair[1]),
        "the faults arrive newest first"
    );

    let (_, page) = server.json(&format!("/api/v1/faults?limit=10&offset={}", FAULTS - 10));
    let last: Vec<i64> = page
        .split("{\"id\": ")
        .skip(1)
        .filter_map(|rest| rest.split(',').next()?.trim().parse().ok())
        .collect();
    assert_eq!(last.len(), 10, "the last page holds the oldest ten");
    assert_eq!(
        last[9],
        ids[FAULTS - 1],
        "the offset still reaches the tail"
    );
}

#[test]
fn a_store_failure_states_a_fixed_message_and_no_path() {
    let (_directory, database) = seeded_database();
    let server = start(&database);
    std::fs::remove_file(&database).expect("the database is removed");

    let (status, body) = server.json("/api/v1/faults");
    assert_eq!(status, 503, "got {body}");
    assert!(
        body.contains("\"code\": \"DATABASE_UNAVAILABLE\""),
        "got {body}"
    );
    assert!(
        body.contains("\"message\": \"cannot read the event database\""),
        "got {body}"
    );
    assert!(!body.contains(".db"), "no path reaches the reader: {body}");
    assert!(
        !body.contains("does not exist"),
        "no store detail reaches the reader: {body}"
    );
}

#[test]
fn an_unknown_api_route_does_not_echo_the_request() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server.json("/api/v1/secret-looking-path");
    assert_eq!(status, 404, "got {body}");
    assert!(
        body.contains("\"message\": \"no such route\""),
        "got {body}"
    );
    assert!(
        !body.contains("secret-looking-path"),
        "the request path is not echoed: {body}"
    );
}

#[test]
fn the_daily_totals_name_the_target_or_state_null() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (_, filtered) = server.json("/api/v1/faults/daily?target=1.1.1.1");
    assert!(
        filtered.contains("{\"target\": \"1.1.1.1\", \"day\": "),
        "a filtered answer names the target on every record: {filtered}"
    );

    let (_, all) = server.json("/api/v1/faults/daily");
    assert!(
        all.contains("{\"target\": null, \"day\": "),
        "an aggregate answer states null: {all}"
    );
}

#[test]
fn the_api_root_and_a_repeated_parameter_answer_the_envelope() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server.json("/api/v1");
    assert_eq!(
        status, 404,
        "the prefix without a route is not a page: {body}"
    );
    assert!(body.contains("\"code\": \"NOT_FOUND\""), "got {body}");
    assert!(
        body.contains("\"message\": \"no such route\""),
        "got {body}"
    );

    let (status, body) = server.json("/api/v1/faults?limit=5&limit=6");
    assert_eq!(status, 400, "a repeated parameter is refused: {body}");
    assert!(body.contains("\"parameter\": \"limit\""), "got {body}");
    assert!(body.contains("more than once"), "got {body}");
}

#[test]
fn the_daily_page_holds_only_the_days_it_states() {
    const DAY: i64 = 86_400_000;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("events.db");
    let first_day = 1_700_006_400_000i64.div_euclid(DAY) * DAY;
    {
        let mut store = Store::open_rw(&path).expect("the database opens");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                first_day - 60_000,
            ))
            .expect("the first transition is recorded");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Up,
                TargetState::Down,
                first_day + 3_600_000,
            ))
            .expect("the fault is recorded");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Down,
                TargetState::Up,
                first_day + 4_200_000,
            ))
            .expect("the return is recorded");
    }
    let server = start(&path);

    let to = first_day + 10 * DAY;
    let (status, body) = server.json(&format!(
        "/api/v1/faults/daily?from={first_day}&to={to}&limit=10"
    ));
    assert_eq!(status, 200, "got {body}");
    assert_eq!(
        field_number(&body, "total"),
        10,
        "the window holds ten days"
    );
    assert_eq!(
        body.matches("\"day\": \"").count(),
        10,
        "one record per day of the page: {body}"
    );
    assert!(
        body.contains("\"faults\": 1, \"downMs\": 600000"),
        "the only fault is on the first day: {body}"
    );

    let (_, later) = server.json(&format!(
        "/api/v1/faults/daily?from={first_day}&to={to}&limit=3&offset=1"
    ));
    assert!(
        later.contains("\"faults\": 0"),
        "the day after states no fault: {later}"
    );
    assert_eq!(field_number(&later, "total"), 10, "the total is the window");
}

#[test]
fn a_window_since_1970_states_its_total_and_returns_only_the_page() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server.json("/api/v1/faults/daily?from=0&limit=5");
    assert_eq!(status, 200, "got {body}");
    assert!(
        field_number(&body, "total") > 20_000,
        "the total counts every day of the window: {body}"
    );
    assert_eq!(
        body.matches("\"day\": \"").count(),
        5,
        "the answer holds the page, not the window: {body}"
    );
    assert!(
        body.contains("\"day\": \"1970-01-01\""),
        "the page starts at the oldest day: {body}"
    );
}

#[test]
fn a_daily_page_past_the_end_still_states_the_total_of_the_window() {
    const DAY: i64 = 86_400_000;
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let first_day = 1_700_006_400_000i64.div_euclid(DAY) * DAY;
    let to = first_day + 10 * DAY;
    let (status, body) = server.json(&format!(
        "/api/v1/faults/daily?from={first_day}&to={to}&limit=10&offset=1000"
    ));

    assert_eq!(status, 200, "got {body}");
    assert!(
        body.contains("\"data\": []"),
        "the page past the end is empty: {body}"
    );
    assert_eq!(
        field_number(&body, "total"),
        10,
        "the total is the window, whatever the page holds: {body}"
    );
}

#[test]
fn the_first_day_of_a_daily_page_is_clipped_to_the_requested_from() {
    const DAY: i64 = 86_400_000;
    const HOUR: i64 = 3_600_000;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("events.db");
    let first_day = 1_700_006_400_000i64.div_euclid(DAY) * DAY;
    {
        let mut store = Store::open_rw(&path).expect("the database opens");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                first_day + 8 * HOUR,
            ))
            .expect("the first transition is recorded");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Up,
                TargetState::Down,
                first_day + 9 * HOUR,
            ))
            .expect("the fault is recorded");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Down,
                TargetState::Up,
                first_day + 11 * HOUR,
            ))
            .expect("the return is recorded");
    }
    let server = start(&path);

    // The window opens at 10:00, inside the fault: only 10:00-11:00 is down time.
    let (status, body) = server.json(&format!(
        "/api/v1/faults/daily?from={}&to={}&limit=2",
        first_day + 10 * HOUR,
        first_day + DAY
    ));

    assert_eq!(status, 200, "got {body}");
    assert_eq!(
        field_number(&body, "faults"),
        0,
        "the fault opened before the window, so it is not a drop inside it: {body}"
    );
    assert_eq!(
        field_number(&body, "downMs"),
        HOUR,
        "only the part after `from` is down time: {body}"
    );
    assert_eq!(
        field_number(&body, "avgUpRunMs"),
        13 * HOUR,
        "the up run starts at 11:00, not at the day start: {body}"
    );
}

#[test]
fn the_current_utc_day_states_the_up_runs_of_the_last_24_hours() {
    const HOUR: i64 = 3_600_000;
    let directory = tempfile::tempdir().expect("a temporary directory");
    let path = directory.path().join("events.db");
    let now = now_ms();
    {
        let mut store = Store::open_rw(&path).expect("the database opens");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Unknown,
                TargetState::Up,
                now - 2 * HOUR,
            ))
            .expect("the first transition is recorded");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Up,
                TargetState::Down,
                now - HOUR,
            ))
            .expect("the fault is recorded");
        store
            .record_transition(&event(
                "1.1.1.1",
                TargetState::Down,
                TargetState::Up,
                now - HOUR / 2,
            ))
            .expect("the return is recorded");
    }
    let server = start(&path);

    // A one-hour window that holds only the UTC day still running.
    let (status, body) = server.json(&format!(
        "/api/v1/faults/daily?from={}&to={}",
        now - HOUR / 2,
        now
    ));
    assert_eq!(status, 200, "got {body}");

    let day = body
        .lines()
        .find(|line| line.contains("\"partial\": true"))
        .expect("the day still running is marked");

    // The runs of the last 24 hours: one hour, then the half hour still open.
    assert_eq!(
        field_number(day, "longestUpRunMs"),
        HOUR,
        "the longest run of the last 24 hours: {day}"
    );
    let average = field_number(day, "avgUpRunMs");
    assert!(
        (2_700_000..2_700_000 + 30_000).contains(&average),
        "the mean of one hour and the half hour still running: {average} in {day}"
    );
}

#[test]
fn an_instant_outside_the_accepted_domain_is_refused() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    // The domain is `1970-01-01T00:00:00Z .. 2100-01-01T00:00:00Z`. The last shape — a `to` with
    // no `from`, at the bottom of the range — is the one whose default window used to run off
    // the end of the range.
    for (path, parameter) in [
        ("/api/v1/faults/daily?from=-1", "from"),
        ("/api/v1/faults/daily?to=4102444800001", "to"),
        ("/api/v1/faults/daily?from=1900-01-01T00:00:00Z", "from"),
        ("/api/v1/faults/daily?to=2100-01-02T00:00:00Z", "to"),
        ("/api/v1/faults/daily?from=-9223372036854775808", "from"),
        ("/api/v1/faults/daily?to=9223372036854775807", "to"),
        ("/api/v1/faults/daily?to=-9223372036854775800", "to"),
        ("/api/v1/faults?from=-1", "from"),
    ] {
        let (status, body) = server.json(path);
        assert_eq!(status, 400, "{path} answered {status}: {body}");
        assert!(
            body.contains("\"code\": \"INVALID_PARAMETER\""),
            "{path}: {body}"
        );
        assert!(
            body.contains(&format!("\"parameter\": \"{parameter}\"")),
            "{path} names {parameter}: {body}"
        );
        assert!(
            body.contains("1970-01-01T00:00:00.000Z") && body.contains("2100-01-01T00:00:00.000Z"),
            "{path} states the domain: {body}"
        );
    }
}

#[test]
fn the_two_ends_of_the_domain_are_accepted() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for path in [
        "/api/v1/faults/daily?from=1970-01-01T00:00:00Z&limit=1",
        "/api/v1/faults/daily?to=2100-01-01T00:00:00Z&limit=1",
        "/api/v1/faults/daily?from=0&to=4102444800000&limit=1",
        "/api/v1/faults?from=1970-01-01T00:00:00Z&to=2100-01-01T00:00:00Z&limit=1",
    ] {
        let (status, body) = server.json(path);
        assert_eq!(status, 200, "{path} answered {status}: {body}");
        assert!(body.starts_with("{\n  \"data\": ["), "got {body}");
        assert!(body.contains("\"meta\": "), "got {body}");
    }
}

/// The whole domain is the widest window a client may ask for, and it must cost what the page
/// costs, not what the span costs: 47 482 days exist, 1 000 are read.
#[test]
fn the_widest_window_answers_only_the_page_it_was_asked_for() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server
        .json("/api/v1/faults/daily?from=1970-01-01T00:00:00Z&to=2100-01-01T00:00:00Z&limit=1000");
    assert_eq!(status, 200, "got {body}");
    assert_eq!(
        field_number(&body, "total"),
        47_482,
        "the days of the domain"
    );
    assert_eq!(
        body.matches("\"dayStartMs\"").count(),
        1000,
        "only the requested page is stated"
    );
}

/// The windows people actually write end in the future: "this month", "this year". A `to` after
/// today is valid, and the days that have not begun yet state nothing and are not `partial`.
#[test]
fn a_window_that_ends_in_the_future_is_answered_whole() {
    const DAY: i64 = 86_400_000;
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let today = now_ms().div_euclid(DAY) * DAY;
    // Ten days before today to twenty days after it: the 30 days of a month, ending in the future.
    let (status, body) = server.json(&format!(
        "/api/v1/faults/daily?from={}&to={}&limit=40",
        today - 10 * DAY,
        today + 20 * DAY
    ));
    assert_eq!(status, 200, "got {body}");
    assert_eq!(field_number(&body, "total"), 30, "the days of the window");
    assert_eq!(body.matches("\"dayStartMs\"").count(), 30, "one per day");
    assert!(
        body.contains(&format!("\"dayStartMs\": {}", today + 19 * DAY)),
        "the last day is in the future: {body}"
    );

    let future_day = body
        .lines()
        .find(|line| line.contains(&format!("\"dayStartMs\": {}", today + 19 * DAY)))
        .expect("the last day of the window");
    assert_eq!(field_number(future_day, "faults"), 0, "{future_day}");
    assert_eq!(field_number(future_day, "downMs"), 0, "{future_day}");
    assert!(
        future_day.contains("\"partial\": false"),
        "only the day still running is partial: {future_day}"
    );

    // A year, the same way: 365 days ending five days after today.
    let (status, body) = server.json(&format!(
        "/api/v1/faults/daily?from={}&to={}&limit=1000",
        today - 360 * DAY,
        today + 5 * DAY
    ));
    assert_eq!(status, 200, "got {body}");
    assert_eq!(field_number(&body, "total"), 365, "the days of the year");
}

/// A `to` at the bottom of the domain leaves no room for the default 30-day window: that is an
/// empty range, stated as such, not an underflowed `from`.
#[test]
fn a_to_at_the_lower_bound_of_the_domain_is_an_empty_range() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, body) = server.json("/api/v1/faults/daily?to=1970-01-01T00:00:00Z");
    assert_eq!(status, 400, "got {body}");
    assert!(body.contains("\"code\": \"INVALID_RANGE\""), "got {body}");
}

#[test]
fn an_instant_beyond_the_integer_range_is_refused() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for path in [
        "/api/v1/faults/daily?from=9223372036854775808",
        "/api/v1/faults/daily?to=-9223372036854775809",
    ] {
        let (status, body) = server.json(path);
        assert_eq!(status, 400, "{path} answered {status}: {body}");
        assert!(
            body.contains("\"code\": \"INVALID_PARAMETER\""),
            "got {body}"
        );
    }
}

/// Every answered request gives its connection slot back, so a long run of requests — the
/// widest window the daily route accepts included — never leaves the server silent.
#[test]
fn a_run_of_requests_never_consumes_a_connection_slot_for_good() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    for round in 0..40 {
        let (status, body) =
            server.json("/api/v1/faults/daily?from=1970-01-01T00:00:00Z&limit=100");
        assert_eq!(status, 200, "round {round} answered {status}: {body}");
        // A refused request must not leak a slot either.
        let (status, body) = server.json("/api/v1/faults/daily?to=9223372036854775807");
        assert_eq!(status, 400, "round {round} answered {status}: {body}");
    }

    let (status, _) = server.get("/healthz");
    assert_eq!(status, 200, "the health check still answers");
    let (status, page) = server.get("/");
    assert_eq!(status, 200, "the page still answers");
    assert!(page.contains("1.1.1.1"), "and it still holds the history");
}
