---
title: Add a channel trait with webhook and log-file channels and name every logger
date: 2026-11-16
status: draft
type: feature
impact: high
area: config, logging, monitor, notify, documentation
version_bump: minor
tags: [notifications, webhook, log-file, logging, telegram, config, event-model]
depends_on: []
branch_recommended: false
---

## Summary

Give notifications the same shape the probes already have: one published contract per
transport, one module per implementation, and a neutral event model every channel renders
in its own words. `notify::Channel` replaces the current `Sender`, and `notify::event`
defines `Event` — a target state change or a startup event — so that the monitor hands
facts to the notifier instead of pre-worded text. Telegram becomes one channel among
others, unchanged in what it sends; a `webhook` channel POSTs the event as JSON to an
operator-configured URL, with an optional bearer token; and a `log file` channel appends
the event to a file the operator names, with its own level and a choice of text or
JSON-lines format. Every channel can be general or bound to one target, with an
`include_general` flag for the startup event. In parallel, every log line the tool writes
gains a logger name — `root` for the application, `notification` for the notifier — so
stderr, the log file and any future sink share one record vocabulary. For an operator
nothing changes until a `[[webhooks]]` or `[[file_logs]]` block is written into the
configuration file, apart from the logger name that now appears on each stderr line.

## Why

Today the notification path knows one transport. `notify::Sender` is the only seam, the
startup announcement is a `String` the monitor formats, and the message text lives inside
`policy::Notification`. Adding a second transport therefore means changing the queue, the
policy, the delivery thread and the monitor — the opposite of the probe seam, where a new
method is one module and nothing else moves. The repository's own principle (one module
owns one responsibility behind a published contract) is violated by the current shape.

The owner needs webhooks now: a generic endpoint that can notify an arbitrary service,
and per-target endpoints that receive only their own target, both with a security token.
`ADR-0027` rejected a generic webhook on purpose — a URL read from the configuration is an
arbitrary outbound request — so this change reverses that decision deliberately and must
say so, and record the mitigations that make the reversal acceptable.

The log has the same problem from the other side. Every line is written by
`logging::write` with one fixed layout and no way to say who spoke, so a line about a
delivery attempt reads exactly like a line about a probe, and the only durable record of a
notification is the SQLite history or the chat itself. The owner wants the notification
stream in a file, at a level he chooses, in a format a collector can parse — which needs a
record model before it needs a sink.

## Goals

- `notify::Channel` is the only transport seam: `name`, `stabilize`, `render(event)` and
  `publish(event, message)`, so a channel that needs facts beyond the text (the log file
  needs the level and the target) has them. The policy, the queue and the delivery thread
  name no transport, and `notify::Sender` no longer exists.
- `notify::event::Event` is the model a channel receives: `Event::TargetState` carrying
  target, label, key, previous state, state, cause, RTT and instant; `Event::Startup`
  carrying configuration path, target keys, web address and instant.
- The monitor no longer formats notification text: it builds an `Event` and the channel
  renders it. `Notifier::announce` is gone.
- Telegram's output is byte-identical to 0.7.1: the `DOWN`/`UP` plain text, its
  `<state> <label> (<key>)`, `Cause: <code> — <message>` and RFC 3339 closing line, and
  the `READY` startup text, all keep the exact layout the user guide documents.
- A `[[webhooks]]` block is accepted in the configuration file, repeated, and identical in
  shape for a general and a target-specific webhook: `url` (required), `token`, `target`,
  `include_general` (default `true`), `stabilize_seconds` (default `60`), `timeout_ms`
  (default `2000`).
- A general webhook (no `target`) receives every target event and the startup event; a
  target webhook receives only its target's events plus the startup event when
  `include_general` is `true`. With both configured, both receive: the operator asked for
  both.
- A webhook POST carries `Content-Type: application/json`, the versioned envelope below,
  and `Authorization: Bearer <token>` when a token is configured. No other header carries
  the token.
- Delivery of a webhook reuses the existing policy: five attempts waiting 1, 2, 4, and 8
  seconds, `2xx` is success, `408`, `425`, `429` and `5xx` are retryable, `400`, `401`,
  `403`, `404`, `3xx` and a malformed answer are not, and a timeout is retryable.
- The token never appears in `--show-config`, in `Debug`, in a log line, or in an error
  message, and it never travels in the request path.
- No new crate: the dependency list of `Cargo.toml` is unchanged.
- A `[[file_logs]]` block is accepted, repeated, and identical in shape for a general and a
  target-specific channel: `path` (required), `level` (default `info`), `format` (default
  `text`, or `json`), `target`, `include_general` (default `true`).
- A log-file channel appends exactly one line per published event — the plain record in
  `text`, or one JSON object per line in `json` — and a level below its configured `level`
  is written nowhere. `down` is `WARN`, `up` and the startup event are `INFO`, so
  `level = "warn"` keeps the outages of that channel and nothing else.
- Every stderr line states its logger: `[root]` for the application and `[notification]`
  for the notifier, in the layout `<time> <LEVEL> [<logger>] [<target>] <message>`, with
  the target bracket absent when there is none.
- One log record renders both ways from the same instant: the stderr text line and the
  JSON line `{"time","level","logger","target","message"}`, so the file and the terminal
  never disagree about when something happened.
- A log-file channel writes without a stabilisation window (`stabilize() == 0`): a log
  line is not a chat message and does not wait.
- With neither a `[telegram]` section nor a `[[webhooks]]` block nor a `[[file_logs]]`
  block, the process starts no delivery thread and opens no connection beyond its probes
  (`BR-8`, `INV-12`).
- `docs/USER-GUIDE.md` states every webhook and file-log key with its type, default,
  accepted values and effect, shows the payloads and the record layouts, and has a
  troubleshooting row for a silent or failing channel.

## Non-goals

- Transports other than Telegram, webhook, and the log file: no mail, MQTT, Slack, or
  command hook.
- Selecting which event types a channel receives beyond "every event" and "the startup
  event": no per-code, per-state or quiet-hours filter.
- A per-logger level, or a level per channel other than the log file: `--log-level` stays
  one global level for stderr, and only `[[file_logs]]` states its own.
- Log rotation: the tool creates and appends the file, and `logrotate` or `newsyslog` owns
  the size, the count and the compression.
- A file sink for the application's own log lines (`root`): the log-file channel carries
  notification events only, and stderr stays the application log.
- A request signature (HMAC) or any authentication other than the bearer token.
- Configurable retry counts or backoff, beyond the existing fixed schedule.
- Applying a changed `[[webhooks]]` or `[[file_logs]]` block without a restart (same rule
  as `[telegram]`).
- A JSON Schema endpoint, an outbound proxy, or disabling TLS verification.
- Notification delivery from `simulate` and from `serve`: only `monitor` notifies, as today.
- Persisting unsent notifications, or re-sending what was missed while down.

## Affected modules

**Source**
- `src/notify/event.rs` — *(new)* the event model: `Event`, `TargetStateEvent`,
  `StartupEvent`, `Cause`, the accessors the policy and the channels share, and
  `Event::level()` — `down` is `WARN`, `up` and startup are `INFO`.
- `src/logging.rs` — `Logger` (`Root`, `Notification`), `Record`, `text_line`, `json_line`,
  and the four helpers taking the logger first; the `[<logger>]` field joins the line.
- `src/notify/mod.rs` — `Channel` trait, `Notification { event, at }`, `Filter`,
  `deliver`, `start(config, shutdown) -> Result<Handle, StartError>`, `start_with`,
  fan-out; `Sender` and `announce` removed.
- `src/notify/policy.rs` — the policy decides on `Event` instead of `Notification`, and its
  `text()` rendering moves to `telegram.rs` (the type stays free of I/O).
- `src/notify/queue.rs` — the queue carries `Notification` only; `Message`,
  `push_announcement` and the announcement test go.
- `src/notify/telegram.rs` — `Telegram` implements `Channel`; `render` holds the existing
  plain-text layout, `publish` the existing form-encoded `sendMessage`.
- `src/notify/webhook.rs` — *(new)* `Webhook` implements `Channel`: JSON envelope rendering
  and one bounded HTTP/1.1 `POST`, plain or over TLS.
- `src/notify/log_file.rs` — *(new)* `LogFile` implements `Channel`: it renders one-line
  event text, and appends that line as a text record or as a JSON-lines record through
  `logging`.
- `src/notify/fake.rs` — `FakeSender` becomes `FakeChannel` (replays scripted outcomes,
  records every rendered message); `FakeNotifier` records events and offers `startups()`.
- `src/config.rs` — `FileWebhook`, `Webhook`, `resolve_webhooks`, `FileLog`, `LogFile`,
  `resolve_file_logs`, `Config.webhooks` and `Config.file_logs`, redaction in `Debug` and
  in `to_toml` for the token, and unit tests.
- `src/monitor/mod.rs` — build `notify::Event::TargetState` after `record_transition`;
  `startup_message` becomes `startup_event`; `run` starts every configured channel and maps
  a `StartError` into `RunError`; the supervisor warns when `webhooks` or `file_logs`
  changed and keeps the running value; the log calls state `Logger::Root` (and
  `Logger::Notification` for `notification queued`).
- `src/main.rs` — the startup log names the configured channels (`telegram configured for
  chat …` stays, plus one line for webhooks and one for log files), the startup event is
  built instead of the text, and the log calls state their logger.
- `src/lib.rs` — no change expected, unless a re-export is needed for the tests (maybe).

**Tests**
- `src/logging.rs` — unit tests: the text line with and without a target, the JSON line with
  an escaped message and a `null` target, and one instant shared by both renderings.
- `tests/notify.rs` — migrated to `Channel`/`Event`; new cases: the startup event reaches a
  channel that includes general events and is skipped by one that does not; a channel is
  rendered once and the identical message is retried.
- `tests/webhook.rs` — *(new)* end-to-end against a local `TcpListener`: JSON body of a
  `down` and of an `up`, `Authorization` present/absent, retry on `500` then `2xx`, no retry
  on `400`, timeout classified retryable, target filter, `include_general` on and off.
- `tests/log_file.rs` — *(new)* against a `tempfile` directory: one text line per event with
  its logger name and level, the JSON line of the same event, `level = "warn"` dropping the
  `up` and the startup event, target scoping, append across two events, and a missing
  directory failing the start.
- `tests/monitor_e2e.rs` — the announcement test becomes a startup-event test; a jump of
  the target state still produces one notification per recorded transition.
- `tests/reload.rs` — a changed `[[webhooks]]` and a changed `[[file_logs]]` block are
  reported and not applied, mirroring the Telegram test.
- `tests/cli.rs` — every stderr assertion takes the `[root]` field into account;
  `--show-config` prints the webhook entries with `<redacted>` tokens and the log-file
  entries as they are; an unknown `target` key in either block fails at startup.
- `src/config.rs`, `src/notify/event.rs`, `src/notify/webhook.rs`, `src/notify/log_file.rs`
  — module unit tests.

**Documentation / configuration**
- `docs/USER-GUIDE.md` — new `### Webhook notifications` and `### File log notifications`
  under *Configuration* (key tables, examples, payloads, record layouts, retries, scoping,
  redaction), the log-line layout with its logger name in *Retention, logs, signals*, a
  `Troubleshooting` row per channel, a `Limits and deliberate choices` line, and the
  contents/`What the tool does` summary that today names Telegram as the only channel. Keep
  the three-line summary at three lines.
- `docs/WEBHOOKS.md`, `docs/LOG-FILES.md` — *(new)* runbooks: enable, verify, repair, and
  the rotation advice the tool deliberately does not implement.
- `docs/PRODUCT.md` — `FR-28` for the channel contract and the webhook behaviour, `FR-29`
  for the log-file channel, `FR-30` for the logger names and the two record formats; update
  `FR-15` (the startup log states which channels are configured, and every line states its
  logger), `FR-25` (it is the Telegram channel, one of the channels), the out-of-scope line
  that lists webhook among the refused transports, `BR-8` (egress), and the sentence that
  calls Telegram the only push channel.
- `docs/ARCHITECTURE.md` — context view (Webhook receiver and log file actors), external
  interfaces table, building-blocks table (`logging`, `notify::event`, `notify::Channel`,
  `webhook`, `log_file`), the `logging` and `notify` component contracts, the configuration
  section (`[[webhooks]]` and `[[file_logs]]` read from the file only, reload rule, the
  global `--log-level` filtering stderr only), the security section (token in a header, TLS
  never disabled, no redirect followed, the log file's directory and mode are the
  operator's), and an invariant for the egress rule.
- `docs/ADR.md` — `ADR-0033` "Add a webhook notification channel behind a channel trait",
  `ADR-0034` "Name a logger on every line and render one record as text or JSON lines", the
  index rows, and an `Amended by` line in `ADR-0027` (its rule 4 and its rejected
  "generic webhook" alternative).
- `docs/GLOSSARY.md` — `Notification` redefined as what a channel publishes, plus
  `Notification channel`, `Notification event`, `Logger name`, `Log record`,
  `Log file channel`, `Webhook`.
- `docs/DEVELOPER-GUIDE.md` — the module tree (`event.rs`, `webhook.rs`, `log_file.rs`, and
  the new `logging.rs` description), the log-record vocabulary, a new "Adding a
  notification channel" checklist, and the test note that names `FakeChannel`.
- `network-monitor.sample.toml` — a commented `[[webhooks]]` block and a commented
  `[[file_logs]]` block with every key.
- `README.md` — the quick-start notification paragraph and the documentation map rows for
  `docs/WEBHOOKS.md` and `docs/LOG-FILES.md`.
- `AGENTS.md` — project-documents table rows for `docs/WEBHOOKS.md` and `docs/LOG-FILES.md`.
- `site/index.html` — the two sentences that present Telegram as the only channel.
- `docs/TELEGRAM-NOTIFICATIONS.md` — one line stating that Telegram is now one channel of
  the same contract (no procedural change).

## Architecture impact

`notify` becomes a channel-oriented subsystem, mirroring `probe`: one contract
(`notify::Channel`), one neutral model (`notify::event::Event`), one module per transport,
and a delivery thread that knows only the contract. The monitor's dependency stays
`Notifier`, so recording is still never a condition of notifying (`INV-12`).

`logging` stops being a stderr formatter and becomes the place that owns the vocabulary of
a diagnostic record — level, logger name, target, message — and the two renderings of it.
`notify` depends on it for the log-file channel; nothing depends on `notify`. The module
stays a leaf, and the record type is the seam a future sink would use.

The architectural change outside `notify` is the egress rule: the process may now connect
to an operator-configured address. `docs/ARCHITECTURE.md` states the rule today as "one
optional egress, `api.telegram.org`"; it must become "the probes, plus the configured
channels: `api.telegram.org` for Telegram and the operator's URLs for webhooks, only when
the corresponding section exists". The log file adds no egress but adds a second file the
process writes, beside the database.

## Decision-record impact

Two decisions are required. `ADR-0033` records the channel abstraction, the event model,
the webhook payload contract, the token handling, and the deliberate reversal of
`ADR-0027`'s rejection of a generic webhook, with the mitigations (fixed `https`-first
transport, TLS verification never disabled, no redirect followed, bounded response, token
in a header only, section read from the file only, retries unchanged). `ADR-0034` records
the log-record vocabulary, the `[<logger>]` field that changes the layout of every line, the
text and JSON-lines formats, and the rule that only the log-file channel states its own
level. `ADR-0027` gains an `Amended by: ADR-0033` line; its decision 4 and its "generic
webhook or command hook" alternative become historical.

## Proposed solution

### Step 1 — Add the notification event model

Create `src/notify/event.rs` and declare it from `src/notify/mod.rs`. Nothing else changes
in this step, so the crate keeps compiling.

```rust
pub enum Event {
    TargetState(TargetStateEvent),
    Startup(StartupEvent),
}

pub struct TargetStateEvent {
    pub target: String,          // label when the target states one, else the key
    pub label: Option<String>,
    pub key: String,
    pub previous_state: TargetState,
    pub state: TargetState,
    pub cause: Option<Cause>,
    pub rtt_us: Option<u64>,
    pub occurred_at_ms: i64,
}

pub struct Cause { pub code: String, pub message: String }

pub struct StartupEvent {
    pub version: &'static str,   // env!("CARGO_PKG_VERSION")
    pub config: Option<PathBuf>,
    pub targets: Vec<String>,    // keys, in configuration order
    pub web: Option<String>,     // the bound address, as today
    pub occurred_at_ms: i64,
}
```

`Event` exposes `occurred_at_ms()`, `target_key() -> Option<&str>`, `is_startup()`,
`label()` and `level() -> logging::Level`, where `down` is `WARN` and `up` and startup are
`INFO`. The type answers questions; it does not word a message — wording belongs to the
channel. Document in the module header that `notify::Event` is the notification event and
`store::Event` the persisted row.

**Verify:** `cargo test --lib notify::event` covers: the target display falls back to the
key; `target_key` is `None` for a startup event; `occurred_at_ms` is the event's instant for
both variants; `level()` is `WARN` for a `down`, `INFO` for an `up` and for the startup
event.

### Step 2 — Name a logger on every record and render it as text or JSON

Rework `src/logging.rs` without touching `notify` yet. A record becomes four facts and one
instant, and the module renders it twice: the stderr line it writes today, and the JSON
line the log-file channel will write.

```rust
pub enum Logger { Root, Notification }      // as_str() -> "root", "notification"

pub struct Record<'a> {
    pub level: Level,
    pub logger: Logger,
    pub target: Option<&'a str>,
    pub message: &'a str,
}

pub fn text_line(instant_ms: i64, record: &Record) -> String;
pub fn json_line(instant_ms: i64, record: &Record) -> String;
```

- The text line keeps today's layout and gains one field:
  `<RFC 3339> <LEVEL:<5> [<logger>] [<target>] <message>`, the target bracket absent when
  there is none. Example:
  `2026-11-16T04:00:00.000Z WARN  [notification] [icmp://1.1.1.1] notification lost: …`.
- The JSON line is one object, keys in this order, every string through `crate::json::string`
  and an absent target as `null`:
  `{"time":"2026-11-16T04:00:00.000Z","level":"warn","logger":"notification","target":"icmp://1.1.1.1","message":"…"}`
- `Level` gains `as_str()` (`"error"`, `"warn"`, `"info"`, `"debug"`) for the JSON line; the
  printed label stays `ERROR`…`DEBUG`.
- The instant is passed in once, so both renderings of one record state the same time.
- `write(level, logger, target, message)` and the four helpers `error`/`warn`/`info`/`debug`
  take the logger first — one way to log, no overload with a default logger.
- `LEVEL` (`--log-level`) keeps filtering what reaches stderr, and nothing else.
- Update all 42 call sites: `Logger::Root` everywhere, except the lines `notify` writes
  (delivery attempts, a lost message, a sent message) and `monitor`'s `notification queued`,
  which state `Logger::Notification`.

**Verify:** `cargo test --lib logging` covers a text line with and without a target, a JSON
line with a quote and a newline in the message and a `null` target, and both renderings of
one record sharing the instant; `cargo test --lib` is green, which proves every call site
was migrated.

### Step 3 — Replace `Sender` with `Channel` and make the monitor pass events

Rework `src/notify/mod.rs`, `policy.rs`, `queue.rs`, `fake.rs`, `telegram.rs`,
`src/monitor/mod.rs` and `src/main.rs` in one step, because the seam changes across them.

- `Notification { event: Event, at: Instant }` with `target() -> Option<&TargetStateEvent>`.
- `Notifier::notify(&self, Notification)` only; `announce` is removed everywhere.
- `pub trait Channel: Send { fn name(&self) -> &'static str; fn stabilize(&self) -> Duration;
  fn render(&self, event: &Event) -> String; fn publish(&mut self, event: &Event,
  message: &str) -> Result<(), SendError> }`. `publish` receives the event as well as the
  rendered message, because the log-file channel needs the level and the target and re-reading
  them from the text would be a parse.
- `deliver(channel, event, shutdown)` renders once and retries the same `String`, exactly
  as today, so every attempt publishes identical bytes.
- `Filter::all()` and `Filter::for_target(key, include_general)`, with `allows(&Event)`
  deciding on `is_startup()` and `target_key()`; the fan-out and the per-channel thread
  structure stay as they are.
- `notify::start(config: &Config, shutdown: &Shutdown) -> Result<Handle, StartError>` starts
  one channel per configured destination. `StartError` stays, because opening a log file can
  fail while building a URL cannot; `monitor::run` maps it into a new
  `RunError::Notification(#[from] notify::StartError)` and stops at startup.
- The log lines of the delivery thread (`… notification attempt 1/5 failed`, `… notification
  lost`, `… notification sent`, `startup announcement sent`) move to
  `Logger::Notification`; everything else in `monitor` and `main` states `Logger::Root`.
- `Telegram` implements `Channel`: `stabilize()` returns the configured window, `render`
  produces the text `policy::Notification::text()` produced, `publish` the existing
  form-encoded POST. The plain-text layout is copied, not rewritten.
- `FakeSender` becomes `FakeChannel`; `FakeNotifier` records events and exposes
  `startups() -> Vec<StartupEvent>`.
- `monitor::startup_message` becomes `monitor::startup_event(config, source, serving) ->
  StartupEvent`, and `monitor::run` builds `notify::Event::TargetState { .. }` after
  `record_transition` returned, from the same facts it uses today.
- The reload path warns on a changed `[telegram]` section as today; Step 9 extends it.

The event model is in place but nothing reads it yet beyond Telegram, so this step keeps
behaviour identical.

**Verify:** `cargo test --lib && cargo test --test notify` — the notification assertions pass
with the Telegram text unchanged, which is the proof that the layout did not move, and
`cargo clippy --all-targets -- -D warnings` is clean. `tests/cli.rs` and the other
integration suites are expected to be red from Step 2 until Step 10 updates their
assertions; the plan says so rather than hiding it.

### Step 4 — Configure webhooks

Add to `src/config.rs`:

```toml
[[webhooks]]
url = "https://example.com/hooks/network-monitor"   # required, http:// or https://
token = "s3cret"                                    # optional, sent as a bearer token
target = "icmp://1.1.1.1"                           # optional, a configured target key
include_general = true                              # optional, default true, only with target
stabilize_seconds = 60                              # optional, default 60, 0 reports at once
timeout_ms = 2000                                   # optional, default 2000, one request budget
```

`FileWebhook` (serde, `deny_unknown_fields`) is resolved into `Webhook` by
`resolve_webhooks`, which refuses, with the key in the message:

- a missing or empty `url`, or one `probe::http::parse_url` cannot split;
- a `token` that is empty after trimming, or that holds whitespace or a control character
  (a header must not be built from it);
- a `target` that is not the key of a configured target;
- `include_general` on an entry without `target` — the flag would be a silent no-op;
- `stabilize_seconds` and `timeout_ms` of `0`… accepted: `stabilize_seconds = 0` reports at
  once, `timeout_ms = 0` is refused (a request without a budget cannot succeed).

`Webhook` holds the resolved values, `Debug` prints `token: Some("<redacted>")`, and
`to_toml` prints `<redacted>` — the same rule as `bot_token` (`ADR-0027`).

**Verify:** `cargo test --lib config` covers a full block, every default, each refusal above,
and that `Debug`/`to_toml` never contain the token; `network-monitor --show-config` with a
temporary file that has one webhook prints the block with `<redacted>`.

### Step 5 — Implement the webhook channel

Create `src/notify/webhook.rs`, holding one module with two responsibilities that cannot be
separated without a second seam: the JSON envelope of an event, and one bounded HTTP/1.1
POST to a configured URL. No crate is added.

Payload of a target event (one line, keys in this order, string escaping through
`crate::json::string`):

```json
{"version":1,"event":"target_state","occurredAt":"2025-11-24T16:00:00.000Z",
 "target":{"key":"icmp://1.1.1.1","label":"cloudflare-dns"},"state":"down",
 "previousState":"up","rttUs":null,
 "cause":{"code":"ICMP.NO_REPLY","message":"no reply within 2000 ms"}}
```

Payload of the startup event:

```json
{"version":1,"event":"startup","occurredAt":"2025-11-24T16:00:00.000Z",
 "monitor":{"version":"0.8.0"},"config":"/etc/network-monitor.toml",
 "targets":["icmp://1.1.1.1"],"web":"http://0.0.0.0:8642/"}
```

`label`, `cause`, `rttUs`, `config` and `web` are `null` when absent; `state` and
`previousState` use `TargetState::as_str()`.

The request:

```
POST <path> HTTP/1.1
Host: <host>[:<port when not the scheme default>]
Content-Type: application/json
Content-Length: <n>
User-Agent: network-monitor/<version>
Authorization: Bearer <token>        (only when a token is configured)
Connection: close
```

Transport rules: `parse_url` gives scheme, host, port and path; `TcpStream::connect_timeout`
and every read share the `timeout_ms` budget; `https` goes through `tls::config()` with no
way to disable verification; the answer is read until the head ends or 8 KiB, and only the
status line is classified; a redirect is never followed — it would forward the bearer token
to another host. Failure reasons name the host and the status, never the token, and every
reason passes through a `scrub` that replaces an accidentally echoed token with
`<redacted>`, as `telegram.rs` already does.

`Webhook::new(settings) -> Self` (infallible: the configuration validated the URL),
`name() -> "webhook"`, `stabilize()` the configured window, `render` the envelope,
`publish` the POST. When a token is configured for an `http://` URL, the constructor records
one `WARN` line at start and proceeds: the operator may be on a trusted link, and refusing
would break that case.

**Verify:** `cargo test --lib notify::webhook` covers the exact bytes of both envelopes
(including an absent label and an absent cause), the request head with and without
`Authorization`, classification of `200`, `400`, `500`, `429` and `302`, and that a token
never appears in a reason string.

### Step 6 — Start the webhooks and scope their events

- `notify::start` builds the vector of channels: Telegram with `Filter::all()` when
  `config.telegram` is set, then one `Webhook` per entry with `Filter::for_target(key,
  include_general)` when `target` is set and `Filter::all()` otherwise. The fan-out and
  `Handle` need no change.
- `src/monitor/mod.rs`: the supervisor compares `webhooks` as well as `telegram` on reload
  and warns `webhook settings changed; restart the monitor to use them`, keeping the running
  value — the same rule and the same wording pattern as the Telegram section.
- `src/main.rs`: after the Telegram line, log at `info` `N webhook channel(s) configured` or
  `no webhook channel`, so an operator sees at startup what will be contacted (`FR-15`).

**Verify:** `cargo test --test notify --test reload` — the new notify cases prove a
target-bound channel ignores another target's event and honours `include_general`; the
reload case proves a changed `[[webhooks]]` block leaves the monitor running with the old
settings.

### Step 7 — Configure log files

Add to `src/config.rs`:

```toml
[[file_logs]]
path = "/var/log/network-monitor/notifications.log"  # required; created and appended
level = "info"                                       # error|warn|info|debug, default info
format = "text"                                      # text (default) or json
target = "icmp://1.1.1.1"                            # optional, a configured target key
include_general = true                               # optional, default true, only with target
```

`FileLog` (serde, `deny_unknown_fields`) is resolved into `LogFile` by `resolve_file_logs`,
which refuses, with the key in the message:
- a missing or empty `path`;
- two entries with the same `path` (one line would be written twice by two threads);
- a `level` that `Level::parse` does not accept;
- a `format` other than `text` or `json` (`jsonl` is accepted as the same JSON lines);
- a `target` that is not the key of a configured target;
- `include_general` on an entry without `target` — the flag would be a silent no-op.

`LogFile` holds the resolved `path: PathBuf`, `level: Level`, `format: LogFormat` (a small
enum with `Text` and `Json`), `target: Option<String>` and `include_general: bool`. It holds
no secret, so `Debug` and `to_toml` print it as it is. The two names follow the
`FileTelegram`/`Telegram` and `FileWebhook`/`Webhook` pattern already in `config.rs`.

**Verify:** `cargo test --lib config` covers a full block, every default, each refusal above,
and a `format = "jsonl"` entry resolving to `LogFormat::Json`; `network-monitor --show-config`
with a temporary file that has one log prints the block.

### Step 8 — Implement the log-file channel

Create `src/notify/log_file.rs`. `LogFile` opens the path once — `OpenOptions::new()
.append(true).create(true)` — and keeps the handle: one line per published event, flushed per
line so the file is readable live. The type reuses the name `config::LogFile` through
`use crate::config::{LogFile as Settings}`, exactly as `telegram.rs` does with
`config::Telegram`.

- `name()` returns `"file log"`; `stabilize()` returns `Duration::ZERO`, because a log line
  needs no stabilisation window.
- `render(event)` words one line per event, the target already stated in the record field:
  - `DOWN {display} — {code}: {message}`, or `DOWN {display}` when the event carries no
    cause;
  - `UP {display}`;
  - `READY network-monitor {version} ({n} targets, config {config}, web {web})`, with `none`
    for an absent configuration path or address.
- `publish(event, message)` writes one record through `logging`: `level` is `event.level()`;
  when it is below the configured `level` nothing is written and the call succeeds (a
  filtered line is not a failure). The record is `Logger::Notification`, `target` is
  `event.target_key()`, and the line is `logging::text_line` or `logging::json_line`
  according to `format`, both rendered from one `now_ms()` reading.
- An I/O error (a full disk, a closed pipe) is a retryable `SendError` whose reason names the
  path and never the message body. The handle is not reopened per write, so a
  rotated-away file keeps receiving until the process restarts; rotation belongs to
  `logrotate`/`newsyslog`, and `docs/LOG-FILES.md` says so.
- `LogFile::new(settings) -> Result<Self, String>` fails when the path cannot be opened (a
  missing directory, a permission), so `notify::start` reports it as a `StartError` and the
  monitor stops instead of silently losing every line.

**Verify:** `cargo test --lib notify::log_file` covers the text line of a `down`, an `up` and
the startup event against a temporary file, the JSON line of the same events (logger
`notification`, level `warn`/`info`, target field), the level filter dropping an `up` and the
startup event at `level = "warn"`, and an unopenable path.

### Step 9 — Start the log files, scope them, and report them at startup

- `notify::start` builds the vector of channels in this order: Telegram with `Filter::all()`
  when `config.telegram` is set, then one `Webhook` per entry, then one `LogFile` per entry
  with `Filter::for_target(key, include_general)` when `target` is set and `Filter::all()`
  otherwise. A `LogFile::new` failure returns `StartError { channel: "file log", reason }`.
- `src/monitor/mod.rs`: the supervisor compares `file_logs` beside `telegram` and `webhooks`
  on reload and warns `log file settings changed; restart the monitor to use them`, keeping
  the running value.
- `src/main.rs`: after the webhook line, log at `info` `N file log channel(s) configured` or
  `no file log channel`, all under `Logger::Root`.

**Verify:** `cargo test --test notify --test log_file --test reload` — an event reaching a
target-bound log file writes one line and another target's event writes none; a changed
`[[file_logs]]` block leaves the monitor running with the old settings.

### Step 10 — Cover the channels end to end

Add `tests/webhook.rs` with a local `TcpListener` helper (the pattern used by
`probe/http.rs` tests) that answers a scripted status and captures the request head and body.
Cases: a `down` event produces the exact JSON body and a `Content-Type: application/json`;
with a token the head carries `Authorization: Bearer …` and without a token it carries no
such header; a `500` answer is retried and the second attempt succeeds; a `400` is not
retried; a receiver that never answers is a retryable timeout; a target-bound channel
receives only its target; `include_general = false` drops the startup event while
`include_general = true` (the default) delivers it.

Migrate `tests/monitor_e2e.rs` (startup event instead of an announcement string) and
`tests/cli.rs` (`--show-config` redaction, unknown `target` at startup, the new startup log
lines, and every stderr assertion taking the `[root]` field into account — today
`contains("INFO  telegram configured")` no longer matches).

Add `tests/log_file.rs` with a `tempfile` directory: one text line per event with the
`[notification]` field and the level; the JSON line of the same event; `level = "warn"`
keeping the `down` and dropping the `up` and the startup event; target scoping; two events
appending two lines; a missing directory failing the start with a message naming the path.

**Verify:** `cargo test --test webhook --test log_file --test notify --test monitor_e2e
--test cli`.

### Step 11 — Update the documentation and the sample configuration

In one pass, in this order:

1. `docs/USER-GUIDE.md`: `### Webhook notifications` and `### File log notifications` under
   *Configuration*, immediately after the Telegram section, each with the key table
   (`Key | Default | Meaning`), a TOML example, the payloads or record layouts, the retry
   and status rules, the target-scoping rule, the redaction rule, and the level mapping of
   the log file; the line layout with its logger name in *Retention, logs, signals*; a
   `Troubleshooting` row per channel; a `Limits and deliberate choices` line (no rotation,
   no per-logger level, no application log in the file); and the two summaries that present
   Telegram as the only channel.
2. `docs/WEBHOOKS.md` and `docs/LOG-FILES.md`: enable, verify (a local receiver and the
   expected body; a `tail -F` and the expected line), repair (status classes, timeout,
   token, target key; path, permission, level, rotation), and the `0600`/rotation advice.
3. `docs/PRODUCT.md`: `FR-28`, `FR-29`, `FR-30`, plus the `FR-15`, `FR-25`, `BR-8` and
   out-of-scope lines.
4. `docs/ARCHITECTURE.md`: the views, tables, `logging` and `notify` contracts,
   configuration and security sections, and the egress invariant.
5. `docs/ADR.md`: `ADR-0033`, `ADR-0034`, and the `ADR-0027` amendment.
6. `docs/GLOSSARY.md`: `Notification`, `Notification channel`, `Notification event`,
   `Logger name`, `Log record`, `Log file channel`, `Webhook`.
7. `docs/DEVELOPER-GUIDE.md`: the tree, the log-record vocabulary, "Adding a notification
   channel", the `FakeChannel` note.
8. `network-monitor.sample.toml`, `README.md`, `AGENTS.md` (table rows),
   `docs/TELEGRAM-NOTIFICATIONS.md` (one line), `site/index.html` (the two sentences).

**Verify:** `grep -rn "webhook\|file_logs\|notification\]" docs/ README.md AGENTS.md
network-monitor.sample.toml site/index.html` shows every document above; every key in each
user-guide table appears in `src/config.rs` (`FileWebhook`, `FileLog`) with the same default,
checked against the `unwrap_or` values and `DEFAULT_STABILIZE_SECONDS`; and the documented
line layout matches `logging::text_line` field for field.

### Step 12 — Run the quality gate and smoke the channels

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`,
then one manual run: a temporary configuration with one `[[webhooks]]` pointing at a local
Python receiver (`http.server` subclass that prints the body) and one `[[file_logs]]` writing
to a temporary path, a target that answers. Confirm the receiver gets the event JSON, the file
gets one `[notification]` line per event, a second run with the receiver stopped shows the
`WARN` lines while the probe keeps running, `--log-level error` still leaves the file being
written (its level is its own), and `--show-config` shows `<redacted>`.

## Security considerations

This change touches secrets and lets the operator name an outbound address, so it carries
a deliberate reversal of `ADR-0027`'s "no user-supplied URL".

- **Exposure:** the URL and the token come from a file the operator controls, read at
  start, with no flag, no environment variable and no inbound surface. The token is a
  credential for a service the operator owns.
- **Token handling:** it travels only in an `Authorization: Bearer` header — never in the
  path, a query string, a log line or an error string. Whitespace and control characters
  are refused so a header cannot be split. `Debug` and `to_toml` print `<redacted>`, and
  `Webhook` scrubs an echoed token from every reason, as `telegram.rs` does.
- **Transport:** `https` is verified through the trust store compiled into the binary
  (`ADR-0024`) and cannot be disabled; `http://` is accepted for a local receiver, and a
  token over `http://` produces one `WARN` at start so the exposure is visible. Redirects
  are never followed, which would otherwise forward the header to another host. The answer
  is bounded at 8 KiB and only its status line is interpreted.
- **Residual risk:** the operator can point a webhook at any address, including one inside
  their network; that is a capability they already have with a shell, and the mitigation
  is that the target list, the URL and the token are all in one reviewed file. Accepted by
  the owner through this plan.

The log-file channel adds no network surface but writes a file, and a notification line
carries a target key and a cause message, never a secret.

- **Exposure:** the path comes from the same reviewed file; the directory and its mode are
  the operator's, and `docs/LOG-FILES.md` asks for the `0600` the database already uses.
- **What lands in the file:** the same facts the database already stores — target, state,
  cause, instant — and, for the startup event, the configuration path and the listen
  address, exactly as the Telegram `READY` message already states them. No token, no bot
  token, no probe payload.
- **Failure:** an unwritable file retries like any channel and never blocks a probe; an
  unopenable path fails the start, so a mistyped directory cannot silently discard the
  stream.

## Verification plan

- **Automated:** the new `logging`, `notify::event`, `notify::webhook`, `notify::log_file`
  and `config` unit tests; `tests/webhook.rs` for the payload, the header, the retry classes
  and the scoping; `tests/log_file.rs` for the record layouts, the level filter and the
  scoping; `tests/notify.rs` for the channel contract and the stabilisation;
  `tests/monitor_e2e.rs` for one notification per recorded transition plus one startup
  event; `tests/reload.rs` for the unchanged-at-reload rule; `tests/cli.rs` for the logger
  field, `--show-config` redaction and the startup log lines.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test`.
- **Manual:** a local receiver prints the body of one `down`, one `up` after the window,
  and the startup event; the same run with the receiver stopped shows five `WARN` lines
  and a monitor that keeps probing; the log file shows the same three events as lines with
  `[notification]`; `--show-config` prints `<redacted>`.
- **Rollback:** remove the `[[webhooks]]` and `[[file_logs]]` blocks and the process behaves
  as 0.7.1 again, apart from the logger field on each stderr line; the code change is
  reversible by reverting the commit, because no schema, no file format and no stored data
  changes.

## Open questions

- [ ] Should a `3xx` answer be retryable? Current best guess: no — a redirect is a
      misconfiguration, and retrying it four times cannot fix it. Owner: maintainer.
- [ ] Should a token over `http://` be refused instead of warned? Current best guess: warn
      once at start, so a receiver on a trusted link keeps working. Owner: maintainer.
- [ ] Is `include_general` on a webhook or a log file without `target` an error? Current
      best guess: yes, it would be a silent no-op. Owner: maintainer.
- [ ] Should the startup event carry the tool version? Current best guess: yes, in
      `monitor.version`, from `env!("CARGO_PKG_VERSION")`, and in the log-file startup
      line. Owner: maintainer.
- [ ] Does the payload need a `schema` URL or only the integer `version`? Current best
      guess: the integer is enough for a v1 contract. Owner: maintainer.
- [ ] Should `[[webhooks]]` also be reachable from `simulate` for demos? Current best
      guess: no, `simulate` notifies nothing (`FR-27`). Owner: maintainer.
- [ ] Are the runbooks `docs/WEBHOOKS.md` and `docs/LOG-FILES.md` worth their maintenance
      cost next to the user-guide sections? Current best guess: yes, the Telegram channel
      has one and the failure modes are transport-specific. Owner: maintainer.
- [ ] Is `[root]` on every line too noisy? Current best guess: keep it — a machine parser
      then reads one layout, and the user guide documents the field. Owner: maintainer.
- [ ] Should a log-file channel own its rotation instead of `logrotate`? Current best
      guess: no, it is a second reimplementation of a solved problem. Owner: maintainer.
- [ ] Should `--log-level` accept a per-logger form (`notification=debug`)? Current best
      guess: no, one global level until a second logger actually needs its own. Owner:
      maintainer.

## Implementation checklist

- [ ] Step 1 — Event model in `src/notify/event.rs`
- [ ] Step 2 — Logger names and the text/JSON record renderings in `src/logging.rs`
- [ ] Step 3 — `Channel` contract, Telegram migration, monitor passes events
- [ ] Step 4 — `[[webhooks]]` in `src/config.rs` with validation and redaction
- [ ] Step 5 — `Webhook` channel in `src/notify/webhook.rs`
- [ ] Step 6 — Start webhooks, scope their events, reload warning, startup log
- [ ] Step 7 — `[[file_logs]]` in `src/config.rs` with validation
- [ ] Step 8 — `LogFile` channel in `src/notify/log_file.rs`
- [ ] Step 9 — Start log files, scope them, reload warning, startup log
- [ ] Step 10 — End-to-end tests and the migrated suites
- [ ] Step 11 — Documentation, runbooks and sample configuration
- [ ] Step 12 — Quality gate and manual smoke test
- [ ] All open questions resolved
- [ ] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [ ] Documentation listed under Affected modules is updated
- [ ] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [ ] `## Implementation Notes` added or updated in this plan
