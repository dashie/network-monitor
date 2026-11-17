//! The channels against a local listener and a real file: no external network.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use assert_cmd::Command;
use network_monitor::config::{
    parse, ChannelSettings, Config, LogFile, LogFormat, NotificationConfig, Overrides, Webhook,
};
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

fn up(key: &str) -> Message {
    Message::target_up(TargetRef::new(key, None), 1_764_000_000_000)
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
        max_bytes: 0,
        max_files: 5,
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
        max_bytes: 0,
        max_files: 5,
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

#[test]
fn a_rotating_log_file_keeps_max_files_while_the_monitor_runs() {
    let directory = tempfile::tempdir().unwrap();
    let filename = directory.path().join("audit.log");
    let logfile = ChannelSettings::LogFile(LogFile {
        name: "audit".to_string(),
        filename: filename.clone(),
        logger: "notification".to_string(),
        level: Level::Info,
        format: LogFormat::Text,
        stabilize: Duration::ZERO,
        max_bytes: 200,
        max_files: 2,
    });
    let shutdown = Shutdown::new();
    let handle = start(&config(vec![logfile], &["audit"], KEY), &shutdown).unwrap();

    // The channel started line plus alternating transitions fill the file twice.
    for _ in 0..6 {
        handle.notifier.send(down(KEY));
        handle.notifier.send(up(KEY));
    }
    let second = directory.path().join("audit.log.2");
    wait_for(|| second.exists(), "the second rotation");
    handle.shutdown(None);

    assert!(filename.exists(), "the current file is always there");
    assert!(directory.path().join("audit.log.1").exists());
    assert!(second.exists());
    assert!(
        !directory.path().join("audit.log.3").exists(),
        "nothing is kept past max_files"
    );
    for path in [filename, directory.path().join("audit.log.1"), second] {
        let length = std::fs::metadata(&path).unwrap().len();
        assert!(length <= 200, "{} holds {length} bytes", path.display());
    }
}

#[test]
fn show_config_round_trips_the_rotation_keys() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("network-monitor.toml");
    std::fs::write(
        &path,
        concat!(
            "interval_ms = 5000\n",
            "[[notification]]\n",
            "name = \"audit\"\n",
            "type = \"logfile\"\n",
            "filename = \"/tmp/audit.log\"\n",
            "max_bytes = 4096\n",
            "max_files = 3\n",
            "[[targets]]\n",
            "ip = \"1.1.1.1\"\n",
        ),
    )
    .unwrap();

    let output = Command::cargo_bin("network-monitor")
        .unwrap()
        .args(["--config", path.to_str().unwrap(), "--show-config"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(text.contains("max_bytes = 4096"), "{text}");
    assert!(text.contains("max_files = 3"), "{text}");

    let again = Config::resolve(
        Some(parse(&text, &directory.path().join("shown.toml")).unwrap()),
        Overrides::default(),
    )
    .unwrap();
    match again.notify.channel("audit").unwrap() {
        ChannelSettings::LogFile(settings) => {
            assert_eq!(settings.max_bytes, 4096);
            assert_eq!(settings.max_files, 3);
        }
        other => panic!("not a log file: {other:?}"),
    }
}
