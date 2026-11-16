//! The channels against a local listener and a real file: no external network.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use network_monitor::config::{ChannelSettings, LogFile, LogFormat, NotificationConfig, Webhook};
use network_monitor::logging::Level;
use network_monitor::notify::message::{Cause, TargetRef};
use network_monitor::notify::{start, Message};
use network_monitor::probe::http::parse_url;
use network_monitor::shutdown::Shutdown;

const KEY: &str = "icmp://1.1.1.1";

fn down(key: &str) -> Message {
    Message::target_down(
        TargetRef::new(key, Some("cloudflare-dns".to_string())),
        Cause::new("ICMP.NO_REPLY", "no reply within 2000 ms"),
        1_764_000_000_000,
    )
}

fn config(channels: Vec<ChannelSettings>, names: &[&str], target: &str) -> NotificationConfig {
    NotificationConfig {
        channels,
        global: names.iter().map(|name| name.to_string()).collect(),
        by_target: HashMap::from([(
            target.to_string(),
            names.iter().map(|name| name.to_string()).collect(),
        )]),
    }
}

fn wait_for(condition: impl Fn() -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn a_webhook_receives_the_json_document_over_http() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (sender, receiver) = mpsc::channel();
    let server = std::thread::spawn(move || {
        // The channel announces itself first, then the transition: two requests.
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buffer = [0u8; 8 * 1024];
            let read = stream.read(&mut buffer).unwrap();
            sender
                .send(String::from_utf8_lossy(&buffer[..read]).to_string())
                .unwrap();
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            stream.flush().unwrap();
        }
    });

    let webhook = ChannelSettings::Webhook(Webhook {
        name: "hook".to_string(),
        url: parse_url(&format!("http://{address}/notify?room=ops")).unwrap(),
        token: None,
        stabilize: Duration::ZERO,
    });
    let shutdown = Shutdown::new();
    let handle = start(&config(vec![webhook], &["hook"], KEY), &shutdown).unwrap();

    handle.notifier.send(down(KEY));
    let first = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    let second = receiver.recv_timeout(Duration::from_secs(5)).unwrap();
    server.join().unwrap();
    handle.shutdown(None);

    let request = [first, second]
        .into_iter()
        .find(|request| request.contains("\"event\":\"target_down\""))
        .expect("the transition request");
    assert!(request.starts_with("POST /notify?room=ops HTTP/1.1\r\n"));
    assert!(request.contains("Content-Type: application/json; charset=utf-8\r\n"));
    assert!(
        request.contains("\"target\":{\"key\":\"icmp://1.1.1.1\",\"label\":\"cloudflare-dns\"}")
    );
    assert!(request.contains("\"cause\":{\"code\":\"ICMP.NO_REPLY\""));
}

#[test]
fn a_log_file_receives_one_json_line_per_event() {
    let directory = tempfile::tempdir().unwrap();
    let filename = directory.path().join("audit.log");
    let logfile = ChannelSettings::LogFile(LogFile {
        name: "audit".to_string(),
        filename: filename.clone(),
        logger: "notification".to_string(),
        level: Level::Info,
        format: LogFormat::Jsonl,
        stabilize: Duration::ZERO,
    });
    let shutdown = Shutdown::new();
    let handle = start(&config(vec![logfile], &["audit"], KEY), &shutdown).unwrap();

    handle.notifier.send(down(KEY));
    wait_for(
        || {
            std::fs::read_to_string(&filename)
                .map(|text| text.contains("\"event\":\"target_down\""))
                .unwrap_or(false)
        },
        "the down line",
    );
    handle.shutdown(None);

    let text = std::fs::read_to_string(&filename).unwrap();
    assert!(text
        .lines()
        .any(|line| line.contains("\"event\":\"channel_started\"")));
    assert!(text
        .lines()
        .any(|line| line.contains("\"logger\":\"notification\"")));
    assert!(text
        .lines()
        .any(|line| line.contains("\"code\":\"ICMP.NO_REPLY\"")));
}

#[test]
fn a_target_scoped_log_file_receives_only_its_target() {
    let directory = tempfile::tempdir().unwrap();
    let filename = directory.path().join("router.log");
    let logfile = ChannelSettings::LogFile(LogFile {
        name: "router".to_string(),
        filename: filename.clone(),
        logger: "notification".to_string(),
        level: Level::Info,
        format: LogFormat::Text,
        stabilize: Duration::ZERO,
    });
    let mut config = config(vec![logfile], &["router"], KEY);
    config.global.clear();
    config
        .by_target
        .insert("icmp://8.8.8.8".to_string(), Vec::new());
    let shutdown = Shutdown::new();
    let handle = start(&config, &shutdown).unwrap();

    handle.notifier.send(down("icmp://8.8.8.8"));
    std::thread::sleep(Duration::from_millis(200));
    assert!(
        !std::fs::read_to_string(&filename)
            .unwrap_or_default()
            .contains("8.8.8.8"),
        "the file belongs to the other target"
    );

    handle.notifier.send(down(KEY));
    wait_for(
        || {
            std::fs::read_to_string(&filename)
                .map(|text| text.contains("1.1.1.1"))
                .unwrap_or(false)
        },
        "the enabled target line",
    );
    handle.shutdown(None);
}
