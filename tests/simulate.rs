//! The simulated mode end to end: the binary serves the page over a generated history.
//!
//! The test drives the process as a client, because what has to hold is the promise the
//! operator sees — same seed, same history, nothing written — and not the internals of any
//! one module. Answers that state the clock are left out of the comparisons: `generatedAt`
//! is not part of the simulation, and both the HTML page and `/api/v1/targets` state "up for",
//! the age of a state, which moves between two requests of the same process.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tempfile::TempDir;

const DAY_MS: i64 = 86_400_000;

/// One simulated server, killed when the test ends.
struct Sim {
    child: Child,
    address: String,
    directory: TempDir,
}

impl Sim {
    fn start(seed: u64, history: &str) -> Sim {
        Sim::start_with(seed, history, None)
    }

    /// The same server, with the fault rate stated when the test wants a quiet board.
    fn start_with(seed: u64, history: &str, fault_rate: Option<&str>) -> Sim {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let address = format!("127.0.0.1:{}", free_port());
        let mut arguments = vec![
            "simulate".to_string(),
            "--listen".to_string(),
            address.clone(),
            "--history".to_string(),
            history.to_string(),
            "--seed".to_string(),
            seed.to_string(),
        ];
        if let Some(rate) = fault_rate {
            arguments.push("--fault-rate".to_string());
            arguments.push(rate.to_string());
        }
        for target in [
            "icmp:1.1.1.1",
            "icmp:192.168.1.1",
            "tcp:1.1.1.1:443",
            "dns:example.com@9.9.9.9",
            "http:https://example.com/health",
        ] {
            arguments.push("--target".to_string());
            arguments.push(target.to_string());
        }
        let child = Command::new(assert_cmd::cargo::cargo_bin("network-monitor"))
            .args(&arguments)
            .current_dir(directory.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("the binary starts");

        let sim = Sim {
            child,
            address,
            directory,
        };
        sim.wait_ready();
        sim
    }

    /// The status and the body of one request, without its headers.
    fn response(&self, path: &str) -> (u16, String) {
        let mut stream = TcpStream::connect(&self.address).expect("a connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(20)))
            .expect("a timeout");
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.address
        );
        stream
            .write_all(request.as_bytes())
            .expect("the request is sent");

        let mut answer = Vec::new();
        stream.read_to_end(&mut answer).expect("the answer arrives");
        let answer = String::from_utf8_lossy(&answer).to_string();
        let (head, body) = answer.split_once("\r\n\r\n").expect("headers and a body");
        let status = head
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .and_then(|code| code.parse::<u16>().ok())
            .expect("a status code");
        (status, body.to_string())
    }

    fn body(&self, path: &str) -> String {
        let (status, body) = self.response(path);
        assert_eq!(status, 200, "{path} answered {status}: {body}");
        body
    }

    /// The first bytes of a streamed answer, and how many arrived.
    ///
    /// An export of ten years is gigabytes: what has to be fast is its first page, not its
    /// whole stream, so the test reads a head and stops.
    fn head(&self, path: &str, limit: usize) -> usize {
        let mut stream = TcpStream::connect(&self.address).expect("a connection");
        stream
            .set_read_timeout(Some(Duration::from_secs(15)))
            .expect("a timeout");
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            self.address
        );
        stream
            .write_all(request.as_bytes())
            .expect("the request is sent");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut buffer = vec![0u8; 8 * 1024];
        let mut read = 0;
        while read < limit && Instant::now() < deadline {
            match stream.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => read += count,
                Err(_) => break,
            }
        }
        read
    }

    fn wait_ready(&self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if TcpStream::connect(&self.address).is_ok() {
                let (status, body) = self.response("/healthz");
                if status == 200 && !body.trim().is_empty() {
                    return;
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("the simulated server never answered on {}", self.address);
    }
}

impl Drop for Sim {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn free_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("an ephemeral port");
    listener.local_addr().expect("the bound address").port()
}

/// The start of the UTC day that contains now, in epoch milliseconds.
fn start_of_today() -> i64 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_millis() as i64;
    now.div_euclid(DAY_MS) * DAY_MS
}

/// The answer without the one line that states the instant it was generated at.
fn without_generated_at(body: &str) -> String {
    body.lines()
        .filter(|line| !line.contains("\"generatedAt\""))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_route_answers_over_the_simulated_history() {
    let sim = Sim::start(0, "30d");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("a clock after the epoch")
        .as_millis() as i64;
    let from = now - 2 * DAY_MS;

    for path in [
        "/".to_string(),
        "/target?target=icmp%3A%2F%2F1.1.1.1".to_string(),
        "/api/v1/targets".to_string(),
        "/api/v1/faults".to_string(),
        "/api/v1/faults/daily".to_string(),
        "/api/v1/docs".to_string(),
        "/api/v1/openapi.json".to_string(),
        "/healthz".to_string(),
        format!("/?from={from}&to={now}"),
        format!("/api/v1/faults?from={from}&to={now}"),
        format!("/api/v1/faults/daily?from={from}&to={now}"),
    ] {
        let (status, body) = sim.response(&path);
        assert_eq!(status, 200, "{path} answered {status}");
        assert!(!body.trim().is_empty(), "{path} answered an empty body");
    }
}

#[test]
fn two_processes_of_the_same_seed_serve_the_same_history() {
    let first = Sim::start(7, "3d");
    let second = Sim::start(7, "3d");

    // A window of whole days that has already happened, so nothing in the answer depends
    // on the instant the request arrives.
    let to = start_of_today();
    let from = to - 2 * DAY_MS;
    for path in [
        format!("/api/v1/faults?from={from}&to={to}&limit=200"),
        format!("/api/v1/faults/daily?from={from}&to={to}"),
    ] {
        let once = without_generated_at(&first.body(&path));
        assert!(!once.contains("\"generatedAt\""));
        assert_eq!(
            once,
            without_generated_at(&second.body(&path)),
            "{path} differs between two processes of the same seed"
        );
        assert_eq!(
            once,
            without_generated_at(&first.body(&path)),
            "{path} differs on a second request"
        );
    }
}

#[test]
fn another_seed_serves_another_history() {
    let first = Sim::start(0, "3d");
    let second = Sim::start(1, "3d");
    let to = start_of_today();
    let from = to - 2 * DAY_MS;
    let path = format!("/api/v1/faults?from={from}&to={to}&limit=200");

    let once = without_generated_at(&first.body(&path));
    assert!(
        once.contains("\"occurredAt\""),
        "the fixture seed has no fault to compare: {once}"
    );
    assert_ne!(
        once,
        without_generated_at(&second.body(&path)),
        "the seed does not change the history"
    );
}

#[test]
fn a_window_before_the_simulated_past_states_no_fault() {
    let sim = Sim::start(0, "2d");
    let to = start_of_today() - 10 * DAY_MS;
    let from = to - DAY_MS;

    let faults = sim.body(&format!("/api/v1/faults?from={from}&to={to}"));
    assert!(
        !faults.contains("\"occurredAt\""),
        "a window older than the simulated past states a fault: {faults}"
    );
    assert!(
        faults.contains("\"total\": 0"),
        "the count of that window is not zero: {faults}"
    );

    let days = sim.body(&format!("/api/v1/faults/daily?from={from}&to={to}"));
    assert!(
        days.contains("\"faults\": 0"),
        "a day older than the simulated past states a fault: {days}"
    );
}

#[test]
fn a_fault_rate_of_zero_serves_a_board_where_nothing_goes_down() {
    let sim = Sim::start_with(0, "30d", Some("0"));
    let to = start_of_today();
    let from = to - 20 * DAY_MS;

    let faults = sim.body(&format!("/api/v1/faults?from={from}&to={to}"));
    assert!(
        faults.contains("\"total\": 0"),
        "a quiet board states a fault: {faults}"
    );

    let days = sim.body(&format!("/api/v1/faults/daily?from={from}&to={to}"));
    assert!(
        !days.contains("\"faults\": ") || !days.contains("\"faults\": 1"),
        "a quiet board states a fault in a day: {days}"
    );
    assert!(
        days.contains("\"downMs\": 0"),
        "a quiet board states down time: {days}"
    );

    let targets = sim.body("/api/v1/targets");
    assert!(
        !targets.contains("\"state\": \"down\""),
        "a quiet board has a target down: {targets}"
    );
}

/// Every answer costs what it returns, whatever the widest settings say.
///
/// The reads used to regenerate the whole covered past on every request — 70 s and 1.7 GB
/// for `/`, and `/api/v1/faults` and the export did not answer at all — so each route is
/// asked here with `--history 3660d --fault-rate 1000`: ten years of a board that fails a
/// thousand times a day, and every answer must be a page in seconds.
#[test]
fn every_answer_costs_what_it_returns_at_the_widest_settings() {
    let sim = Sim::start_with(0, "3660d", Some("1000"));

    for path in [
        "/",
        "/api/v1/targets",
        "/target?target=icmp%3A%2F%2F1.1.1.1",
        "/api/v1/faults/daily",
        &format!(
            "/api/v1/faults?from={}&to={}",
            start_of_today() - 30 * DAY_MS,
            start_of_today()
        ),
    ] {
        let started = Instant::now();
        let (status, body) = sim.response(path);
        let elapsed = started.elapsed();
        assert_eq!(status, 200, "{path}");
        assert!(!body.trim().is_empty(), "{path}");
        assert!(
            elapsed < Duration::from_secs(10),
            "{path} took {elapsed:?} at the widest settings"
        );
    }

    // A fault list with no window asks for the count of every fault of the past it was given
    // — ten years of a board that fails a thousand times a day — and that count is a scan. It
    // must still be a scan of arithmetic and not of events: the page itself is instant, and
    // the process must not grow.
    let started = Instant::now();
    let (status, body) = sim.response("/api/v1/faults");
    let elapsed = started.elapsed();
    assert_eq!(status, 200);
    assert!(
        body.contains("\"total\""),
        "the answer states a total: {body}"
    );
    assert!(
        elapsed < Duration::from_secs(60),
        "counting every fault of the widest board took {elapsed:?}"
    );

    // The streamed export is as long as the history it exports, so the guard is its first
    // page: it used to produce nothing at all for a minute.
    let started = Instant::now();
    let head = sim.head("/export/faults.csv.gz", 64 * 1024);
    let elapsed = started.elapsed();
    assert!(
        head > 8 * 1024,
        "the export sent {head} bytes at the widest settings"
    );
    assert!(
        elapsed < Duration::from_secs(10),
        "the first page of the export took {elapsed:?} at the widest settings"
    );
}

#[test]
fn the_simulated_mode_writes_nothing() {
    let sim = Sim::start(0, "30d");
    let _ = sim.body("/");
    let _ = sim.body("/api/v1/faults");
    let _ = sim.body("/api/v1/faults/daily");

    let written: Vec<String> = std::fs::read_dir(sim.directory.path())
        .expect("the working directory")
        .map(|entry| format!("{:?}", entry.expect("an entry").path()))
        .collect();
    assert!(written.is_empty(), "the simulation wrote {written:?}");
}
