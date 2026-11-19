//! The notification message: what happened, in facts — never in text.
//!
//! Every channel renders one [`Message`] in its own form: plain text for Telegram, the
//! JSON document for a webhook, a line for a log file. The event carries the facts, so
//! no channel has to parse what another one wrote.

use std::time::Instant;

use crate::json;
use crate::logging::Level;
use crate::store::TargetState;
use crate::time::format_rfc3339;

/// The transport a channel speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Telegram,
    Webhook,
    LogFile,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Telegram => "telegram",
            Kind::Webhook => "webhook",
            Kind::LogFile => "logfile",
        }
    }
}

/// The failure that made a target unreachable, as the probe reported it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cause {
    pub code: String,
    pub message: String,
}

impl Cause {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Cause {
            code: code.into(),
            message: message.into(),
        }
    }
}

/// The target of an event: its identity and the label an operator sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetRef {
    pub key: String,
    pub label: Option<String>,
}

impl TargetRef {
    pub fn new(key: impl Into<String>, label: Option<String>) -> Self {
        TargetRef {
            key: key.into(),
            label,
        }
    }

    /// The label when there is one, the key otherwise.
    pub fn display(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.key)
    }

    /// `UP cloudflare-dns (icmp://1.1.1.1)`, or the key alone without a label.
    fn headline(&self, state: &str) -> String {
        let display = self.display();
        if display == self.key {
            format!("{state} {}", self.key)
        } else {
            format!("{state} {display} ({})", self.key)
        }
    }
}

/// A monitor absence reported by the startup message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GapSummary {
    pub from_ms: i64,
    pub to_ms: i64,
    pub targets: usize,
}

/// What a message reports: a target changing state, or the process itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Startup {
        config: Option<String>,
        targets: Vec<String>,
        web: Option<String>,
        gap: Option<GapSummary>,
    },
    Shutdown,
    ConfigReload {
        applied: bool,
        detail: String,
    },
    ChannelStarted {
        name: String,
        kind: Kind,
    },
    TargetDown {
        target: TargetRef,
        cause: Cause,
    },
    TargetDegraded {
        target: TargetRef,
        cause: Cause,
    },
    TargetUp {
        target: TargetRef,
    },
}

/// One message handed to every channel that should receive it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub event: Event,
    /// When the event happened, as epoch milliseconds.
    pub occurred_at_ms: i64,
    /// The instant the message was created, which starts a stabilisation window.
    pub at: Instant,
}

impl Message {
    pub fn target_down(target: TargetRef, cause: Cause, occurred_at_ms: i64) -> Self {
        Message::new(Event::TargetDown { target, cause }, occurred_at_ms)
    }

    pub fn target_degraded(target: TargetRef, cause: Cause, occurred_at_ms: i64) -> Self {
        Message::new(Event::TargetDegraded { target, cause }, occurred_at_ms)
    }

    pub fn target_up(target: TargetRef, occurred_at_ms: i64) -> Self {
        Message::new(Event::TargetUp { target }, occurred_at_ms)
    }

    pub fn startup(
        config: Option<String>,
        targets: Vec<String>,
        web: Option<String>,
        occurred_at_ms: i64,
    ) -> Self {
        Message::new(
            Event::Startup {
                config,
                targets,
                web,
                gap: None,
            },
            occurred_at_ms,
        )
    }

    /// Attach the monitor gap to a startup message; a non-startup message is unchanged.
    pub fn with_gap(mut self, gap: Option<GapSummary>) -> Self {
        if let Event::Startup { gap: slot, .. } = &mut self.event {
            *slot = gap;
        }
        self
    }

    pub fn shutdown(occurred_at_ms: i64) -> Self {
        Message::new(Event::Shutdown, occurred_at_ms)
    }

    pub fn config_reload(applied: bool, detail: impl Into<String>, occurred_at_ms: i64) -> Self {
        Message::new(
            Event::ConfigReload {
                applied,
                detail: detail.into(),
            },
            occurred_at_ms,
        )
    }

    pub fn channel_started(name: impl Into<String>, kind: Kind, occurred_at_ms: i64) -> Self {
        Message::new(
            Event::ChannelStarted {
                name: name.into(),
                kind,
            },
            occurred_at_ms,
        )
    }

    fn new(event: Event, occurred_at_ms: i64) -> Self {
        Message {
            event,
            occurred_at_ms,
            at: Instant::now(),
        }
    }

    /// The key of the target the event concerns, when it concerns one.
    pub fn target_key(&self) -> Option<&str> {
        match &self.event {
            Event::TargetDown { target, .. }
            | Event::TargetDegraded { target, .. }
            | Event::TargetUp { target } => Some(&target.key),
            _ => None,
        }
    }

    /// The stable name of the event, used by the JSON document.
    pub fn event_name(&self) -> &'static str {
        match self.event {
            Event::Startup { .. } => "startup",
            Event::Shutdown => "shutdown",
            Event::ConfigReload { .. } => "config_reload",
            Event::ChannelStarted { .. } => "channel_started",
            Event::TargetDown { .. } => "target_down",
            Event::TargetDegraded { .. } => "target_degraded",
            Event::TargetUp { .. } => "target_up",
        }
    }

    /// The diagnostic level a log file writes this event at.
    pub fn severity(&self) -> Level {
        match self.event {
            Event::TargetDown { .. } | Event::TargetDegraded { .. } => Level::Warn,
            _ => Level::Info,
        }
    }

    /// The plain-text rendering, one fact per line.
    ///
    /// `UP`, `DOWN`, and the startup block keep the layout the Telegram channel has
    /// always used; a client that parses them keeps working.
    pub fn text(&self) -> String {
        let instant = format_rfc3339(self.occurred_at_ms);
        match &self.event {
            Event::TargetDown { target, cause } => format!(
                "{}\nCause: {} — {}\n{instant}",
                target.headline("DOWN"),
                cause.code,
                cause.message
            ),
            Event::TargetDegraded { target, cause } => format!(
                "{}\nCause: {} — {}\n{instant}",
                target.headline("DEGRADED"),
                cause.code,
                cause.message
            ),
            Event::TargetUp { target } => format!("{}\n{instant}", target.headline("UP")),
            Event::Startup {
                config,
                targets,
                web,
                gap,
            } => {
                let mut text = String::from("READY uptime-monitor\n");
                text.push_str(&format!(
                    "Config: {}\n",
                    config.as_deref().unwrap_or("none")
                ));
                text.push_str(&format!("Targets: {}\n", targets.len()));
                for target in targets {
                    text.push_str(&format!("- {target}\n"));
                }
                text.push_str(&format!("Web: {}\n", web.as_deref().unwrap_or("none")));
                if let Some(gap) = gap {
                    text.push_str(&format!(
                        "Gap: {} to {} ({} target(s) marked unknown)\n",
                        format_rfc3339(gap.from_ms),
                        format_rfc3339(gap.to_ms),
                        gap.targets
                    ));
                }
                text.push_str(&instant);
                text
            }
            Event::Shutdown => format!("STOPPED uptime-monitor\n{instant}"),
            Event::ConfigReload { applied, detail } => {
                if *applied {
                    format!("CONFIG applied from {detail}\n{instant}")
                } else {
                    format!("CONFIG rejected: {detail}\n{instant}")
                }
            }
            Event::ChannelStarted { name, kind } => {
                format!("CHANNEL {name} ({}) started\n{instant}", kind.as_str())
            }
        }
    }

    /// The single-line rendering a log file writes.
    ///
    /// The instant is the log line's own timestamp, so the summary states the facts and
    /// leaves the time to the line. Every newline is flattened, because one line is one
    /// record.
    pub fn summary(&self) -> String {
        let text = match &self.event {
            Event::TargetDown { target, cause } => format!(
                "{} cause={}: {}",
                target.headline("DOWN"),
                cause.code,
                cause.message
            ),
            Event::TargetDegraded { target, cause } => format!(
                "{} cause={}: {}",
                target.headline("DEGRADED"),
                cause.code,
                cause.message
            ),
            Event::TargetUp { target } => target.headline("UP"),
            Event::Startup {
                config,
                targets,
                web,
                gap,
            } => {
                let mut text = format!(
                    "READY uptime-monitor config={} targets={} web={}",
                    config.as_deref().unwrap_or("none"),
                    targets.len(),
                    web.as_deref().unwrap_or("none")
                );
                if let Some(gap) = gap {
                    text.push_str(&format!(
                        " gap={}..{} targets={}",
                        gap.from_ms, gap.to_ms, gap.targets
                    ));
                }
                text
            }
            Event::Shutdown => "STOPPED uptime-monitor".to_string(),
            Event::ConfigReload { applied, detail } => {
                if *applied {
                    format!("CONFIG applied from {detail}")
                } else {
                    format!("CONFIG rejected: {detail}")
                }
            }
            Event::ChannelStarted { name, kind } => {
                format!("CHANNEL {name} ({}) started", kind.as_str())
            }
        };
        text.replace(['\n', '\r'], " ")
    }

    /// The canonical JSON document, one line, with an optional logger member.
    ///
    /// Schema 1 is the contract the webhook sends and the `jsonl` log file writes.
    pub fn json(&self, logger: Option<&str>) -> String {
        let mut out = String::from("{\"schema\":1");
        if let Some(logger) = logger {
            out.push_str(",\"logger\":");
            out.push_str(&json::string(logger));
        }
        out.push_str(",\"event\":");
        out.push_str(&json::string(self.event_name()));
        out.push_str(",\"occurred_at\":");
        out.push_str(&json::string(&format_rfc3339(self.occurred_at_ms)));
        match &self.event {
            Event::TargetDown { target, cause } => {
                out.push_str(",\"target\":");
                out.push_str(&target_json(target));
                out.push_str(",\"state\":");
                out.push_str(&json::string(TargetState::Down.as_str()));
                out.push_str(",\"cause\":{\"code\":");
                out.push_str(&json::string(&cause.code));
                out.push_str(",\"message\":");
                out.push_str(&json::string(&cause.message));
                out.push('}');
            }
            Event::TargetDegraded { target, cause } => {
                out.push_str(",\"target\":");
                out.push_str(&target_json(target));
                out.push_str(",\"state\":");
                out.push_str(&json::string(TargetState::Degraded.as_str()));
                out.push_str(",\"cause\":{\"code\":");
                out.push_str(&json::string(&cause.code));
                out.push_str(",\"message\":");
                out.push_str(&json::string(&cause.message));
                out.push('}');
            }
            Event::TargetUp { target } => {
                out.push_str(",\"target\":");
                out.push_str(&target_json(target));
                out.push_str(",\"state\":");
                out.push_str(&json::string(TargetState::Up.as_str()));
            }
            Event::Startup {
                config,
                targets,
                web,
                gap,
            } => {
                out.push_str(",\"startup\":{\"config\":");
                out.push_str(&json::option_string(config));
                out.push_str(",\"targets\":[");
                out.push_str(
                    &targets
                        .iter()
                        .map(|target| json::string(target))
                        .collect::<Vec<_>>()
                        .join(","),
                );
                out.push_str("],\"web\":");
                out.push_str(&json::option_string(web));
                if let Some(gap) = gap {
                    out.push_str(",\"gap\":{\"from\":");
                    out.push_str(&json::string(&format_rfc3339(gap.from_ms)));
                    out.push_str(",\"to\":");
                    out.push_str(&json::string(&format_rfc3339(gap.to_ms)));
                    out.push_str(",\"targets\":");
                    out.push_str(&gap.targets.to_string());
                    out.push('}');
                }
                out.push('}');
            }
            Event::ConfigReload { applied, detail } => {
                out.push_str(",\"reload\":{\"applied\":");
                out.push_str(if *applied { "true" } else { "false" });
                out.push_str(",\"detail\":");
                out.push_str(&json::string(detail));
                out.push('}');
            }
            Event::ChannelStarted { name, kind } => {
                out.push_str(",\"channel\":{\"name\":");
                out.push_str(&json::string(name));
                out.push_str(",\"kind\":");
                out.push_str(&json::string(kind.as_str()));
                out.push('}');
            }
            Event::Shutdown => {}
        }
        out.push('}');
        out
    }
}

fn target_json(target: &TargetRef) -> String {
    let mut out = String::from("{\"key\":");
    out.push_str(&json::string(&target.key));
    if let Some(label) = &target.label {
        out.push_str(",\"label\":");
        out.push_str(&json::string(label));
    }
    out.push('}');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const AT_MS: i64 = 1_764_000_000_000;

    fn down() -> Message {
        Message::target_down(
            TargetRef::new("icmp://1.1.1.1", Some("cloudflare-dns".to_string())),
            Cause::new("ICMP.NO_REPLY", "no reply within 2000 ms"),
            AT_MS,
        )
    }

    fn degraded() -> Message {
        Message::target_degraded(
            TargetRef::new("icmp://1.1.1.1", Some("cloudflare-dns".to_string())),
            Cause::new(
                "PROBE.SLOW",
                "the round-trip time stayed above 200 ms for 3 consecutive probes",
            ),
            AT_MS,
        )
    }

    fn up() -> Message {
        Message::target_up(TargetRef::new("icmp://1.1.1.1", None), 0)
    }

    fn startup() -> Message {
        Message::startup(
            Some("/etc/uptime-monitor.toml".to_string()),
            vec![
                "icmp://192.168.1.1".to_string(),
                "tcp://1.1.1.1:443".to_string(),
            ],
            Some("http://0.0.0.0:8642/".to_string()),
            AT_MS,
        )
    }

    fn all() -> Vec<Message> {
        vec![
            down(),
            degraded(),
            up(),
            startup(),
            Message::shutdown(AT_MS),
            Message::config_reload(true, "/etc/uptime-monitor.toml", AT_MS),
            Message::config_reload(false, "invalid configuration: interval_ms", AT_MS),
            Message::channel_started("audit", Kind::LogFile, AT_MS),
        ]
    }

    #[test]
    fn a_disconnection_keeps_the_layout_of_the_channel() {
        assert_eq!(
            down().text(),
            "DOWN cloudflare-dns (icmp://1.1.1.1)\n\
             Cause: ICMP.NO_REPLY — no reply within 2000 ms\n\
             2025-11-24T16:00:00.000Z"
        );
    }

    #[test]
    fn a_degradation_keeps_the_layout_of_a_fault() {
        assert_eq!(
            degraded().text(),
            "DEGRADED cloudflare-dns (icmp://1.1.1.1)\n\
             Cause: PROBE.SLOW — the round-trip time stayed above 200 ms for 3 consecutive probes\n\
             2025-11-24T16:00:00.000Z"
        );
        assert_eq!(
            degraded().summary(),
            "DEGRADED cloudflare-dns (icmp://1.1.1.1) cause=PROBE.SLOW: \
             the round-trip time stayed above 200 ms for 3 consecutive probes"
        );
    }

    #[test]
    fn a_recovery_without_a_label_states_the_key_once() {
        assert_eq!(up().text(), "UP icmp://1.1.1.1\n1970-01-01T00:00:00.000Z");
    }

    #[test]
    fn the_startup_block_states_one_fact_per_line() {
        assert_eq!(
            startup().text(),
            "READY uptime-monitor\n\
             Config: /etc/uptime-monitor.toml\n\
             Targets: 2\n\
             - icmp://192.168.1.1\n\
             - tcp://1.1.1.1:443\n\
             Web: http://0.0.0.0:8642/\n\
             2025-11-24T16:00:00.000Z"
        );
    }

    #[test]
    fn a_startup_gap_reaches_the_text_the_summary_and_the_json() {
        let gap = GapSummary {
            from_ms: AT_MS - 60_000,
            to_ms: AT_MS,
            targets: 2,
        };
        let message = startup().with_gap(Some(gap));

        assert!(message
            .text()
            .contains("Gap: 2025-11-24T15:59:00.000Z to 2025-11-24T16:00:00.000Z (2 target(s) marked unknown)\n"));
        assert!(message
            .summary()
            .ends_with(" gap=1763999940000..1764000000000 targets=2"));
        assert!(message.json(None).contains(
            "\"gap\":{\"from\":\"2025-11-24T15:59:00.000Z\",\
                      \"to\":\"2025-11-24T16:00:00.000Z\",\"targets\":2}"
        ));
    }

    #[test]
    fn a_startup_without_a_gap_is_unchanged() {
        let without = startup().with_gap(None);
        assert_eq!(without.text(), startup().text());
        assert_eq!(without.summary(), startup().summary());
        assert_eq!(without.json(None), startup().json(None));
    }

    #[test]
    fn a_gap_does_not_touch_a_non_startup_message() {
        let message = down().with_gap(Some(GapSummary {
            from_ms: 0,
            to_ms: 1,
            targets: 1,
        }));
        assert_eq!(message.event, down().event);
    }

    #[test]
    fn every_global_event_has_a_text() {
        assert_eq!(
            Message::shutdown(AT_MS).text(),
            "STOPPED uptime-monitor\n2025-11-24T16:00:00.000Z"
        );
        assert_eq!(
            Message::config_reload(true, "/etc/uptime-monitor.toml", AT_MS).text(),
            "CONFIG applied from /etc/uptime-monitor.toml\n2025-11-24T16:00:00.000Z"
        );
        assert_eq!(
            Message::config_reload(false, "no targets configured", AT_MS).text(),
            "CONFIG rejected: no targets configured\n2025-11-24T16:00:00.000Z"
        );
        assert_eq!(
            Message::channel_started("audit", Kind::LogFile, AT_MS).text(),
            "CHANNEL audit (logfile) started\n2025-11-24T16:00:00.000Z"
        );
    }

    #[test]
    fn a_summary_is_one_line_for_every_event() {
        for message in all() {
            let summary = message.summary();
            assert!(!summary.contains('\n'), "unexpected newline: {summary}");
            assert!(
                !summary.contains('\r'),
                "unexpected carriage return: {summary}"
            );
        }
        assert_eq!(
            down().summary(),
            "DOWN cloudflare-dns (icmp://1.1.1.1) cause=ICMP.NO_REPLY: no reply within 2000 ms"
        );
    }

    #[test]
    fn the_event_names_are_stable() {
        assert_eq!(down().event_name(), "target_down");
        assert_eq!(degraded().event_name(), "target_degraded");
        assert_eq!(up().event_name(), "target_up");
        assert_eq!(startup().event_name(), "startup");
        assert_eq!(Message::shutdown(0).event_name(), "shutdown");
        assert_eq!(
            Message::config_reload(true, "x", 0).event_name(),
            "config_reload"
        );
        assert_eq!(
            Message::channel_started("a", Kind::Webhook, 0).event_name(),
            "channel_started"
        );
    }

    #[test]
    fn a_disconnection_is_a_warning_and_the_rest_is_information() {
        assert_eq!(down().severity(), Level::Warn);
        assert_eq!(degraded().severity(), Level::Warn);
        assert_eq!(up().severity(), Level::Info);
        assert_eq!(startup().severity(), Level::Info);
    }

    #[test]
    fn the_target_key_is_present_only_for_target_events() {
        assert_eq!(down().target_key(), Some("icmp://1.1.1.1"));
        assert_eq!(degraded().target_key(), Some("icmp://1.1.1.1"));
        assert_eq!(up().target_key(), Some("icmp://1.1.1.1"));
        assert_eq!(startup().target_key(), None);
        assert_eq!(Message::shutdown(0).target_key(), None);
    }

    #[test]
    fn the_json_target_document_carries_key_label_state_and_cause() {
        assert_eq!(
            down().json(None),
            "{\"schema\":1,\"event\":\"target_down\",\
             \"occurred_at\":\"2025-11-24T16:00:00.000Z\",\
             \"target\":{\"key\":\"icmp://1.1.1.1\",\"label\":\"cloudflare-dns\"},\
             \"state\":\"down\",\
             \"cause\":{\"code\":\"ICMP.NO_REPLY\",\"message\":\"no reply within 2000 ms\"}}"
        );
    }

    #[test]
    fn the_json_degraded_document_states_the_state_and_the_cause() {
        assert_eq!(
            degraded().json(None),
            "{\"schema\":1,\"event\":\"target_degraded\",\
             \"occurred_at\":\"2025-11-24T16:00:00.000Z\",\
             \"target\":{\"key\":\"icmp://1.1.1.1\",\"label\":\"cloudflare-dns\"},\
             \"state\":\"degraded\",\
             \"cause\":{\"code\":\"PROBE.SLOW\",\
             \"message\":\"the round-trip time stayed above 200 ms for 3 consecutive probes\"}}"
        );
    }

    #[test]
    fn the_label_is_omitted_when_the_target_has_none() {
        assert_eq!(
            up().json(None),
            "{\"schema\":1,\"event\":\"target_up\",\
             \"occurred_at\":\"1970-01-01T00:00:00.000Z\",\
             \"target\":{\"key\":\"icmp://1.1.1.1\"},\"state\":\"up\"}"
        );
    }

    #[test]
    fn a_log_sink_adds_the_logger_member() {
        let document = down().json(Some("notification"));
        assert!(document
            .starts_with("{\"schema\":1,\"logger\":\"notification\",\"event\":\"target_down\""));
    }

    #[test]
    fn the_json_global_documents_carry_their_own_member() {
        assert_eq!(
            startup().json(None),
            "{\"schema\":1,\"event\":\"startup\",\
             \"occurred_at\":\"2025-11-24T16:00:00.000Z\",\
             \"startup\":{\"config\":\"/etc/uptime-monitor.toml\",\
             \"targets\":[\"icmp://192.168.1.1\",\"tcp://1.1.1.1:443\"],\
             \"web\":\"http://0.0.0.0:8642/\"}}"
        );
        assert_eq!(
            Message::config_reload(false, "no targets configured", AT_MS).json(None),
            "{\"schema\":1,\"event\":\"config_reload\",\
             \"occurred_at\":\"2025-11-24T16:00:00.000Z\",\
             \"reload\":{\"applied\":false,\"detail\":\"no targets configured\"}}"
        );
        assert_eq!(
            Message::channel_started("audit", Kind::LogFile, AT_MS).json(None),
            "{\"schema\":1,\"event\":\"channel_started\",\
             \"occurred_at\":\"2025-11-24T16:00:00.000Z\",\
             \"channel\":{\"name\":\"audit\",\"kind\":\"logfile\"}}"
        );
        assert_eq!(
            Message::shutdown(AT_MS).json(None),
            "{\"schema\":1,\"event\":\"shutdown\",\
             \"occurred_at\":\"2025-11-24T16:00:00.000Z\"}"
        );
    }
}
