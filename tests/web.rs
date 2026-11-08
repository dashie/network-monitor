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
fn every_row_shows_its_statistics_and_links_to_its_detail_page() {
    let (_directory, database) = seeded_database();
    let server = start(&database);

    let (status, page) = server.get("/");
    assert_eq!(status, 200);
    assert!(page.contains("DROPS"), "the drop column has a header");
    assert!(page.contains("DOWN"), "the down-time column has a header");
    assert!(page.contains("href=\"/target?target=1.1.1.1\""));

    let (status, detail) = server.get("/target?target=1.1.1.1");
    assert_eq!(status, 200);
    assert!(detail.contains("<!doctype html>"));
    assert!(
        detail.contains("<svg"),
        "one bar per day of the last 30 days"
    );
    assert!(detail.contains("1 disconnections"));
    assert!(detail.contains("href=\"/\""), "a way back to the timeline");
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
