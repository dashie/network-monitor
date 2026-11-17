---
title: Generalise notifications into typed messages delivered by pluggable channels
date: 2026-11-16
status: done
type: feature
impact: high
area: notify, config, logging, monitor
version_bump: minor
tags: [notifications, channels, webhook, logfile, logging, configuration]
depends_on: []
branch_recommended: true
---

## Summary

Replace the Telegram-only notifier with a channel contract. One typed `Message` is rendered
by each channel in its own form — plain text for Telegram, a JSON document for a webhook, one
log line per event in text or JSON-line form for a file — and each channel owns its queue,
thread, stabilisation policy, retries and lifetime. A channel is declared once in a
`[[notification]]` array under a name, and enabled by that name: globally with the root
`notify` list, or per target with `notify` on a `[[targets]]` entry, where the target list
replaces the global one. No destination is declared twice and no target repeats a channel's
configuration. Every log line gains a logger name (`monitor`, `notification`). Recording, the
schema, the page, the API and the query output stay exactly as they are.

## Why

- The transport is welded to the decision today: `Notifier::notify(Notification)` carries text
  already rendered by `policy::Notification::text`, `notify::start` hard-codes
  `telegram::Telegram`, and `Notifier::announce(String)` is a free-text pass-through. A second
  destination means editing the policy, the queue, the runner, the monitor and `main`.
- The operator needs more than one destination, with different scopes: the same incident may
  go to a chat, to an automation endpoint, and to a durable file; a channel may watch one
  target while another watches all of them. `FR-25`/ADR-0027 admitted one transport on
  purpose; the reason for it — a push that never delays a probe and never touches the record —
  applies unchanged to a webhook and a log file.
- Scoping a channel by repeating its block on each target duplicates the destination and forces
  merge rules: which scope wins, which level, which format. Declaring a channel once and
  enabling it by name removes that whole class of ambiguity and makes the file read as a list
  of channels plus a routing table.
- Log lines carry no logger name (`logging::write`), so an operator cannot filter
  notification diagnostics from process lines, and a file sink could not say which stream a
  line belongs to.
- Evidence: `src/notify/mod.rs` exposes one `Sender`, one `start` and one `start_with`;
  `src/config.rs` has one `FileTelegram`; `docs/PRODUCT.md` lists a webhook as out of scope
  and `ADR-0027` rejected it explicitly.
- The cost of leaving it: every new transport rewrites the same three modules, the audit trail
  stays SQLite-only, and notifications cannot be silenced per target.

## Goals

- `[[notification]]` declares 0..N channels once; `name` and `type` are required, `type` is
  `telegram`, `webhook` or `logfile`, and `stabilize_seconds` defaults to `60`. A channel is
  enabled by name, never configured twice.
- One typed `notify::Message` with `Event::Startup`, `Shutdown`, `ConfigReload`,
  `ChannelStarted`, `TargetDown`, `TargetUp`; a target event carries key, label, state and
  cause; a global event carries its own facts. No channel parses text produced by another.
- Telegram sends exactly today's bytes for `DOWN`, `UP` and `READY`, so the existing assertions
  in `tests/notify.rs` and the examples in the user guide stay true.
- A webhook channel POSTs the same message as JSON with an optional `Authorization: Bearer`
  token, retries `429`/`5xx`/transport failures, never follows a redirect, and never prints the
  token or the URL path.
- A log file channel appends one line per event — plain text in the stderr format, or one JSON
  document per line — with a selectable logger name, level and format.
- Enablement is a reference by name: the root `notify = [...]` applies to global events and to
  every target, a target's `notify = [...]` replaces that list for it, an absent key inherits,
  and `notify = []` silences the target. An unknown name, a duplicate name, a declared channel
  that nothing enables, or two channels on one destination is a configuration error that stops
  the process before the first probe.
- Every log line is `<RFC 3339> <LEVEL> <logger> [<target>] <message>`: `monitor` for the
  process, `notification` for notification diagnostics.
- A channel that cannot open is fatal at startup with one line naming it; a live delivery
  failure is a warning, never touches recording (INV-12) and never stops another channel.
- Shutdown sends one `STOPPED` message per channel with a single attempt; a changed
  notification configuration is reported on reload and takes effect at the next start, exactly
  as `database` and `[telegram]` do today.

## Non-goals

- Mail, MQTT, Slack-specific payloads, a command hook, and any plugin or dynamic-loading
  mechanism.
- Log rotation, compression or retention by the tool: the operator rotates files.
- Per-target rendering of one destination: a channel has one configuration, so two targets that
  need different formats or levels use two channels with two destinations.
- A channel that receives global events but none of its targets' events, or the reverse: the
  root list enables both, a target list enables only that target's events.
- Following redirects, disabling certificate verification, or a proxy.
- A settings screen, an API endpoint, or any new inbound surface for channels.
- Changing the SQLite schema, the page, the JSON API or the query output.
- Removing `[telegram]` in this release.

## Affected modules

Every file the implementation is expected to touch, grouped by category. This list is the
scope boundary: anything outside it requires explicit user permission.

**Source**
- `src/notify/mod.rs` — `Notifier::send(Message)`; fan-out notifier; one queue, thread and
  policy per channel; `start`, `start_with`, `Handle::shutdown(farewell)`.
- `src/notify/message.rs` — *(new)* `Message`, `Event`, `TargetRef`, `Cause`, `Kind`, `text()`,
  `summary()`, `event_name()`, `severity()`, `target_key()`.
- `src/notify/channel.rs` — *(new)* `Channel` trait, `Payload`, `Routing`, `SendError`,
  `deliver`, `deliver_once`.
- `src/notify/policy.rs` — engine and policy over `Message`; the same rules, no rendering.
- `src/notify/queue.rs` — carries `Message` only; the announcement variant disappears.
- `src/notify/telegram.rs` — `impl Channel`; keeps the request, classification and scrubbing.
- `src/notify/webhook.rs` — *(new)* JSON payload, HTTP(S) POST, Bearer token, classification,
  scrubbing.
- `src/notify/logfile.rs` — *(new)* append-only sink, text or JSON-line rendering, logger and
  level, flush.
- `src/notify/fake.rs` — `FakeNotifier::send`; new `FakeChannel` for the runner tests.
- `src/config.rs` — `FileNotification` (required `name`, no per-channel scope), root `notify`,
  per-target `notify`, `NotificationConfig`, `ChannelSettings`, `Destination`, `LogFormat`,
  validation, routing lists, legacy `[telegram]`, `to_toml`.
- `src/cli.rs` — the target grammar is unchanged; the channel array and the `notify` lists are
  file-only, like the secrets they carry.
- `src/logging.rs` — logger name, `format_line_at`, `Level::allows`, `MONITOR`,
  `NOTIFICATION`.
- `src/tls.rs` — reusable `connect(host, port, timeout)` for the TLS clients (Telegram and an
  https webhook).
- `src/monitor/mod.rs` — start and stop the channels, `RunError::Notification`,
  `startup_event`, the shutdown message, the reload message, the settings comparison.
- `src/main.rs` — `log_notifications` instead of `log_telegram`; the startup event.

**Tests**
- `tests/notify.rs` — rewritten against `FakeChannel` and `start_with`: routing, per-channel
  window, retry, shutdown message, open failure.
- `tests/monitor_e2e.rs`, `tests/reload.rs` — `Notifier::send`; a reload message for a changed
  channel declaration or routing list.
- `tests/cli.rs` — the new line prefix (`INFO  monitor …`), `--show-config` with redacted
  secrets, one startup line per channel.
- `tests/config_precedence.rs` — the channel array and the `notify` lists are file-only; assert
  that exception explicitly.
- `tests/notify_channels.rs` — *(new)* a webhook against a `127.0.0.1:0` listener and a log
  file in a temporary directory.
- `tests/simulate.rs` — *(added by the user during review round 3)* the client read timeout of
  the simulated-mode harness, aligned with the budget its widest-settings test asserts.

**Documentation / configuration**
- `docs/PRODUCT.md` — `FR-2` exception, `FR-15` log format, `FR-25` rewritten, new `FR-28`
  (webhook) and `FR-29` (log file, with its `format`), `BR-8`, scope, assumptions, the resolved
  alerting question.
- `docs/ARCHITECTURE.md` — constraints (egress), context diagram and table, solution strategy,
  component table and contracts (`notify`, `logging`), Logging, Configuration, Security,
  Reliability, `INV-14`.
- `docs/ADR.md` — new `ADR-0033`; `ADR-0027` marked `superseded by ADR-0033`.
- `docs/GLOSSARY.md` — channel, channel name, notification channel, output destination,
  enablement (global and per target), message (global and target), scope, webhook channel, log
  file channel, logger name; revise notification and bot token.
- `docs/USER-GUIDE.md` — `Configuration → Notifications` rewritten with a declaration table per
  channel type, the root and per-target `notify` lists with inheritance and override, the log
  line format under `Operations`, troubleshooting rows, limits.
- `docs/NOTIFICATIONS.md` — *(renamed from `docs/TELEGRAM-NOTIFICATIONS.md`)* one runbook for
  the three channels.
- `docs/DEVELOPER-GUIDE.md` — the file-only exception, a new "Adding a notification channel"
  recipe, the test list.
- `README.md`, `AGENTS.md` (the project-document table), `NOTES.md` (a writable path inside the
  container), `uptime-monitor.sample.toml`, `site/index.html`.
- `Cargo.toml` — version `0.8.0`, applied at close through `version_bump: minor`; no new
  dependency, so `Cargo.lock` changes only in the package version (maybe).

## Architecture impact

- `notify` is split like `probe`: a contract (`message`, `channel`), a policy, a fan-out and
  three transports, each in its own file.
- Egress stops being fixed: a webhook reaches an operator-supplied host, so ADR-0027's "one
  fixed host" constraint is replaced. TLS verification, no proxy, no redirect stay.
- The log line format changes for every line, so the logging contract and `FR-15` change in the
  same commit.
- The configuration gains a routing table instead of per-channel scopes: channels are declared
  once and referenced by name, globally and per target.
- New invariant `INV-14`: one event reaches a destination at most once. A destination is
  declared by at most one channel, a channel is enabled by name, and the fan-out holds one
  queue per declared channel; a duplicate destination, an unknown name or a channel nothing
  enables is refused before the first probe.
- Sections to update: 2 (constraints), 3 (context), 4 (solution strategy), 5 (component table
  and contracts), Logging, Configuration, Security, Reliability, Invariants.

## Decision-record impact

- New `ADR-0033 — Deliver notifications through pluggable channels with a typed message`,
  superseding `ADR-0027`: the channel contract, the message model, one queue/thread/policy per
  channel, channels declared once and enabled by name (root and per-target lists), one
  destination declared by one channel, the webhook with arbitrary egress and a bearer token,
  the log file with text or JSON-line output, the logger names, and hand-written HTTP/JSON with
  no new crate. Alternatives to reject in the record: a single fan-out thread, repeating a
  channel block per target, per-channel `targets` lists with merge rules, `serde_json`, plugin
  loading, and built-in log rotation.
- `ADR-0027` becomes `superseded by ADR-0033` in the index and in its status line; its text is
  not rewritten (append-only).

## Proposed solution

Numbered steps, each independently verifiable. For every step the files, the observable result
and the check are stated.

### Step 1 — Name the loggers and carry the name in every line

`src/logging.rs` gains `pub const MONITOR: &str = "monitor"` and
`pub const NOTIFICATION: &str = "notification"`, `pub fn format_line_at(at_ms: i64, level: Level,
logger: &str, target: Option<&str>, message: &str) -> String`, `pub fn notification(level: Level,
target: Option<&str>, message: &str)`, and `Level::allows(self, configured: Level) -> bool`,
which is `configured >= self` in `Level`'s order — the configured level is the most verbose one
admitted, exactly as `--log-level` behaves.
`write` becomes a thin caller of `format_line_at` with `now_ms()` and `MONITOR`. The line is
`<RFC 3339> <LEVEL padded to 5> <logger> [<target>] <message>`, the bracket omitted when there
is no target. Logger names are constants, so no call site outside `notify` changes.

**Verify:** `cargo test --lib logging` (with and without a target, both loggers, `allows`);
`cargo test --test cli` after step 11; a manual line reads
`2026-11-16T10:00:00.000Z INFO  monitor initializing`.

### Step 2 — Define the message

`src/notify/message.rs` (new) holds `Message { event: Event, occurred_at_ms: i64, at: Instant }`,
`Event::{Startup { config: Option<String>, targets: Vec<String>, web: Option<String> }, Shutdown,
ConfigReload { applied: bool, detail: String }, ChannelStarted { name: String, kind: Kind },
TargetDown { target: TargetRef, cause: Cause }, TargetUp { target: TargetRef }}`,
`TargetRef { key, label }` with `display()`, `Cause { code, message }`, and
`Kind { Telegram, Webhook, LogFile }` with `as_str()`. `Message` derives
`Debug, Clone, PartialEq, Eq`, because the fan-out clones it once per channel. Constructors
`target_down`, `target_up`, `shutdown`, `config_reload`, `channel_started`; accessors
`target_key(&self) -> Option<&str>`, `event_name`, `severity` (`TargetDown` is `Warn`,
everything else `Info`), `text` (multi-line, byte-identical to today's `DOWN`/`UP`/`READY`
blocks), `summary` (one line, every newline flattened to a space) and
`json(&self, logger: Option<&str>) -> String` — the canonical one-line document defined in
step 7, with an optional top-level `logger` member that a log sink supplies.
`ConfigReload::detail` holds the path for an applied reload and the rendered error for a
rejected one.

**Verify:** `cargo test --lib notify::message` — a golden string per event, including the
labelled `DOWN` with a cause and the `READY` block.

### Step 3 — Move the policy onto messages

`src/notify/policy.rs`: `Notification` is deleted; `Policy::observe(Message)` and
`Engine::observe`/`due_all`/`next_deadline` key on `Message::target_key()`. The rules are
unchanged: a `down` is immediate, an `up` waits for the window, a `down` inside the window
cancels it, and the first verdict is reported under the same rules. Global events are never
given to the engine.

**Verify:** `cargo test --lib notify::policy` — the eight existing tests adapted, with the same
assertions on event and instant.

### Step 4 — Publish the channel contract

`src/notify/channel.rs` (new) holds `SendError`, `Payload { body: String, media_type:
&'static str }`, the trait

```rust
pub trait Channel: Send {
    fn name(&self) -> &str;
    fn kind(&self) -> Kind;
    fn open(&mut self) -> Result<(), String>;
    fn payload(&self, message: &Message) -> Option<Payload>;
    fn send(&mut self, payload: &Payload) -> Result<(), SendError>;
    fn close(&mut self);
}
```

and `Routing { global: Vec<usize>, by_target: HashMap<String, Vec<usize>> }` with
`channels_for(&Message) -> &[usize]` — the channel indices, in declaration order, that a global
event or a target event reaches; an event that no channel enables yields an empty slice.
`deliver(channel, message, shutdown)` renders the payload once and attempts it five times with
the 1, 2, 4 and 8 second waits, stopping on a non-retryable answer or a shutdown;
`deliver_once` attempts once. A `None` payload counts as delivered. `ATTEMPTS` and the backoff
live here.

**Verify:** `cargo test --lib notify::channel` — the routing table (a global list, a target
that inherits it, a target with its own list, a silenced target), a payload built once, the
three outcomes, and `None` counting as delivered.

### Step 5 — Run one queue and one thread per channel

`src/notify/mod.rs`: `Notifier` becomes `fn send(&self, message: Message)`. `FanoutNotifier`
holds `Vec<Arc<Queue>>` in declaration order and the `Routing`, and pushes a clone to every
index the message resolves to: a global event reaches the root list, a target event reaches its
effective list, and a message that resolves to no channel is dropped silently — a silenced
target is a choice, not a fault. `NoopNotifier` stays. `Queue` carries `Message`. A `Runner`
owns `Arc<Mutex<Box<dyn Channel>>>`, its queue, its stop flag, its thread and its `stabilize`.
`start(&NotificationConfig, &Shutdown) -> Result<Handle, StartError>` builds every channel from
the configuration in declaration order, calls `open` on each (a failure is
`StartError { channel, reason }`, with the channels already opened closed again), spawns one
thread per channel, builds the `Routing` from the enablement lists, and delivers
`ChannelStarted` on that channel as its first message. `Handle::shutdown(farewell)` sends the
farewell once to every channel the shutdown event routes to (the root list), under the channel
mutex, then closes the queues, stops and joins the threads, then closes each channel.

**Verify:** `cargo test --lib notify` — routing (global, inherited, overridden, silenced),
per-channel window, the start message first, an open failure naming the channel, the farewell
delivered once.

### Step 6 — Make Telegram a channel

`src/notify/telegram.rs`: `impl Channel for Telegram` (`open`/`close` no-ops, `payload` the
`form_encode`d body of `Message::text()`, `send` today's `exchange`). `src/tls.rs` gains
`connect(host, port, timeout)` extracted from the current local `connect()`, used by Telegram
and by an https webhook. Token `Debug` and `scrub` are unchanged.

**Verify:** `cargo test --lib notify::telegram` — the request, the classification, the
redaction, and delivery through the fake.

### Step 7 — Add the webhook channel

`src/notify/webhook.rs` (new): `Webhook { name, url: UrlParts, token: Option<String> }`;
`payload` is `message.json(None)` — one line on the wire — and `send` opens a `TcpStream` with
the same 5 second read and write timeouts for `http`, or calls `tls::connect` for `https`, then
writes `POST <target> HTTP/1.1` with `Host`,
`Content-Type: application/json; charset=utf-8`, `Authorization: Bearer …` when a token is
configured, `Content-Length` and `Connection: close`, reads at most 8 KiB, and classifies
`2xx` as success, `3xx` and `4xx` as non-retryable, `429`, `5xx` and transport errors as
retryable. Every reason is scrubbed of the token and never contains the URL path or query.

JSON, schema 1, shown indented; the body is one line:

```json
{
  "schema": 1,
  "event": "target_down",
  "occurred_at": "2026-11-16T10:00:00.000Z",
  "target": { "key": "icmp://1.1.1.1", "label": "cloudflare-dns" },
  "state": "down",
  "cause": { "code": "ICMP.NO_REPLY", "message": "no reply within 2000 ms" }
}
```

`target`, `state` and `cause` appear only when the event carries them, and `label` is omitted
when the target has none. A global event adds its own member and nothing else: `startup` adds
`{"config": …, "targets": […], "web": …}`, `config_reload` adds
`{"applied": …, "detail": …}`, `channel_started` adds `{"name": …, "kind": …}`, and `shutdown`
adds nothing. `src/config.rs` refuses a token on an `http://` URL in step 9.

**Verify:** `cargo test --lib notify::webhook` — a golden payload per event, the headers with
and without a token, the status classification, and the token absent from every rendered
error; `cargo test --test notify_channels` after step 11 posts to a local listener.

### Step 8 — Add the log file channel

`src/notify/logfile.rs` (new): `LogFile { name, filename: PathBuf, logger: String, level: Level,
format: LogFormat }`, with `LogFormat { Text, Jsonl }` defined in `config`.
`open` appends (`create(true).append(true)`, mode `0o600` on Unix, the parent directory must
already exist and is never created). `payload` returns `None` when `!level.allows(self.level)`.
With `Text` it returns one line built by `logging::format_line_at(message.occurred_at_ms,
message.severity(), &self.logger, message.target_key(), &message.summary())` and the media type
`text/plain; charset=utf-8`. With `Jsonl` it returns `message.json(Some(&self.logger))` — the
webhook document plus a `logger` member, one compact document per line — and the media type
`application/x-ndjson`. `send` writes the line with its newline and flushes, so a follower sees
it at once; an I/O error is non-retryable. `close` flushes. The channel's own transport errors
go to stderr through `logging::notification`, never into its own file.

**Verify:** `cargo test --lib notify::logfile` — the line shape for all six events in both
formats, the `logger` member in JSONL, the level filter, an append preserving what was there,
and mode `0600` on Unix.

### Step 9 — Declare the channels and route the events

`src/config.rs`: `FileConfig.notification: Option<Vec<FileNotification>>`,
`FileConfig.notify: Option<Vec<String>>`, `FileTarget.notify: Option<Vec<String>>` (the legacy
`telegram: Option<FileTelegram>` stays); `FileNotification { name, type, stabilize_seconds,
bot_token, chat_id, url, token, filename, logger, level, format }` — a channel has no scope of
its own. `TargetSpec` carries `notify: Option<Vec<String>>` through the configuration merge;
`Target` does not, because routing is a notification concern and not part of a target's
identity. The resolved side is `Config.notify: NotificationConfig { channels:
Vec<ChannelSettings>, global: Vec<String>, by_target: HashMap<String, Vec<String>> }` with
`ChannelSettings::{Telegram, Webhook, LogFile}` and
`Destination::{TelegramChat(String), Url(String), File(PathBuf)}`; every channel exposes
`name()` and `destination()`.

Declarations, then routing, then validation:

- `name` is required, matches `[A-Za-z0-9._-]+` and is unique; it is the only way a channel is
  referenced anywhere;
- `type` is required and is one of the three; a key that belongs to another type is refused
  rather than ignored, the way `FileTarget` already treats a foreign method key;
- telegram requires `bot_token` (the same no-whitespace and no-separator rule) and `chat_id`;
- webhook requires `url`, parsed by `probe::http::parse_url`; a `token` on an `http://` URL is
  an error naming the key;
- logfile requires `filename`; `logger` defaults to `notification` and matches
  `[A-Za-z0-9._-]+`; `level` defaults to `info` and accepts the four level names; `format`
  defaults to `text` and accepts `text` or `jsonl`;
- every channel has one destination, and two channels may not share one: the trimmed `chat_id`,
  the canonical URL (lower-case scheme and host, the resolved port, the path and the query) or
  the filename made absolute against the working directory and lexically normalised (`.` and
  `..` folded, no symlink resolution). `./a.log` and `a.log` therefore collide while two
  symlinked paths are two destinations; a duplicate is a `ConfigError` naming both channels and
  the destination without exposing a secret (a webhook is `scheme://host[:port]`);
- the root `notify` list enables channels for global events and for every target that does not
  override it; a target's `notify` list replaces it for that target; an absent key inherits and
  `notify = []` silences the target. Both are sets, so a repeated name is one subscription;
- a channel enabled only by target lists receives those targets' events and no global event:
  the root list is what puts a channel on the startup, shutdown and config-reload stream (a
  channel's own `ChannelStarted` line is written on the channel itself, as in step 5);
- every name in either list must be a declared channel, and every declared channel must be
  enabled by the root list or by at least one target of the file: a typo in a name and an
  orphan declaration are errors, not silence. The check uses the file's targets, so `--target`
  never turns a valid file into an error; a channel that only a file target enables then starts
  but receives nothing, which the guide states;
- the effective list of every resolved target is computed once and stored in
  `NotificationConfig.by_target` under the target key; a `--target` override replaces the file
  target list and with it the per-target lists, leaving the root list in force;
- the legacy `[telegram]` declares a channel named `telegram` and adds it to the root list
  (deduplicated); it is an error when that name or that `chat_id` is already declared.

`Debug` on `ChannelSettings` redacts every token as `Telegram` does today, and `Debug`/`Display`
on `Destination` renders a webhook as `scheme://host[:port]` without the path, so no log line
or error can leak a secret. `to_toml` declares the root `notify` value before the
`[[notification]]` and `[[targets]]` arrays, because TOML requires values before tables.

All of this runs in `Config::resolve`, before any probe or socket: one `error:` line, a non-zero
exit, and the process never starts.

```toml
# every target and every global event goes to telegram
notify = ["telegram"]

[[notification]]
name = "telegram"
type = "telegram"
bot_token = "123456789:replace-me"
chat_id = "-1001234567890"

[[notification]]
name = "audit"
type = "logfile"
filename = "/var/log/uptime-monitor/audit.log"
format = "jsonl"
level = "info"

[[targets]]
ip = "1.1.1.1"
# inherits notify = ["telegram"]

[[targets]]
ip = "192.168.1.1"
notify = ["telegram", "audit"]   # this target adds the audit file

[[targets]]
ip = "10.0.0.1"
notify = []                      # this target notifies nobody
```

`to_toml` prints the declarations in order with `bot_token` and `token` as `<redacted>`, the
root `notify` list, and a target's `notify` list only when it differs from the root; the output
parses back into the same routing.

**Verify:** `cargo test --lib config` — one test per rule; the routing matrix (inherit, add a
channel, override, silence, a repeated name); an unknown name, a duplicate name, a duplicate
destination (`./a.log` versus `a.log`, host case, default port), an unused channel and a
`[telegram]` clash each refused; the legacy alias; and the `to_toml` round trip;
`cargo test --test config_precedence`.

### Step 10 — Wire the process

`src/monitor/mod.rs`: `RunError::Notification(#[from] notify::StartError)`; `run` starts the
channels before the workers and returns the error when a channel cannot open; the startup is a
`Option<Message>` passed through `run_with_notifier`; after the loop
`handle.shutdown(Some(Message::shutdown(now_ms())))`; `Supervisor::apply` compares `notify`
instead of `telegram` (warning `notification settings changed; restart the monitor to use
them`) and emits `Message::config_reload` for an applied and for a rejected reload.
`src/main.rs`: `log_notifications` prints one `INFO` line per declared channel as
`notification channel <name> (<kind>) configured for <destination>` — `chat <id>` for Telegram,
`scheme://host[:port]` only for a webhook, the path with logger, level and format for a log
file — then `notification defaults: <names>` for the root list, or
`notification defaults: none` when it is empty.

**Verify:** `cargo test --test monitor_e2e`, `cargo test --test reload`, `cargo test --test cli`.

### Step 11 — Adapt and extend the tests

`src/notify/fake.rs` gains `FakeChannel` (records payloads, replays send outcomes, records
`open`/`close`). `tests/notify.rs` uses it; `tests/monitor_e2e.rs` and `tests/reload.rs` use
`send`; `tests/cli.rs` asserts the new line prefix, a `--show-config` with one channel of each
type and `<redacted>`, that an unknown name in a `notify` list, an orphan channel and two
channels on one destination exit non-zero before the first probe, and that `--show-config`
round-trips; `tests/notify_channels.rs` posts to a local listener, writes a text file and a
JSONL file in a temporary directory, and checks that a target-scoped channel receives that
target's events only.

**Verify:** `cargo test`.

### Step 12 — Update the documentation

`docs/PRODUCT.md` (`FR-2` exception, `FR-15` format, `FR-25` rewritten around the declared-once
channel and its enablement by name, new `FR-28` webhook and `FR-29` log file with its `format`,
`BR-8`, scope in/out, assumptions, the alerting question);
`docs/ARCHITECTURE.md` (constraints, context diagram and table, solution strategy, component
table and contracts, Logging, Configuration and its routing table, Security, Reliability,
`INV-14`);
`docs/ADR.md` (`ADR-0033`, `ADR-0027` superseded); `docs/GLOSSARY.md`;
`docs/USER-GUIDE.md` (`Configuration → Notifications` with a declaration key table per channel
type including `format`, the root and per-target `notify` lists with inheritance, override and
the empty list, a worked example, the note that `--target` replaces the file target list and
with it its per-target lists, the log line format, troubleshooting rows for an unknown name, an
orphan channel, two channels on one destination and a level that hides events, limits; the
`What the tool does` summary stays at three lines); `docs/NOTIFICATIONS.md` renamed
from `docs/TELEGRAM-NOTIFICATIONS.md` and extended to the three channels;
`docs/DEVELOPER-GUIDE.md` (exception text, "Adding a notification channel"); `README.md`,
`AGENTS.md`, `NOTES.md`, `uptime-monitor.sample.toml`, `site/index.html`.

**Verify:** every field of `FileNotification`, the root `notify` key and the target `notify` key
appear in the user guide; `grep -rn "TELEGRAM-NOTIFICATIONS" --include="*.md" --include="*.toml"
--include="*.html" .` returns nothing outside `tech-plans/archive`; `grep -rn "\[telegram\]"
docs README.md site` mentions only the legacy alias; every key in
`uptime-monitor.sample.toml` appears in the user-guide tables.

### Step 13 — Run the quality gate

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.

### Step 14 — Align the simulated client's read timeout with the budget it asserts

*(Added by the user after review round 2: the close gate could not be satisfied.)*

`tests/simulate.rs`: `Sim::response` waits 20 seconds for an answer, while
`every_answer_costs_what_it_returns_at_the_widest_settings` allows `/api/v1/faults` 60
seconds and states that the count must still be fast. At `--history 3660d --fault-rate 1000`
the count of ten simulated years takes about 21 seconds, measured against the built binary,
so the client timed out mid-body and the failure surfaced as a missing `"total"` member
instead of as the timing the test is about. One documented `READ_TIMEOUT` constant, above the
60 second budget the test states, replaces the literal; a read that fails names the route that
stalled, so a future stall says what stalled.

**Verify:** `cargo test --test simulate` three times in a row, then
`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.

## Security considerations

- New egress: a webhook URL is supplied by the operator, so the "one fixed host" constraint
  ends. The mitigations are that TLS verification stays on and cannot be disabled, a redirect
  is a failure rather than something to follow (so a compromised endpoint cannot bounce the
  token to a third host), there is no proxy, and the request has a 5 second budget and an 8 KiB
  answer bound.
- A webhook token is a credential: file-only (no flag, no environment variable), sent as
  `Authorization: Bearer`, refused over `http`, redacted in `Debug`, in `--show-config` and in
  every error, and never part of a printed URL — the startup line and a configuration error
  state `scheme://host[:port]` only, because a webhook path may itself be a secret.
- A log file holds target addresses and causes: it is created in append mode with mode `0600`
  on Unix, its directory is never created, and its own transport errors go to stderr only, so
  a failing sink cannot feed itself. In `jsonl` mode every value passes through `json::string`,
  so a cause message cannot forge a line or escape the document, and a duplicate destination is
  refused while the configuration is resolved, before any file is opened.
- Untrusted input stays bounded and pattern-matched; the JSON is written by this process.
- Residual risk: a webhook endpoint sees every reported incident by design, and an operator who
  points a channel at a third-party service accepts that. Recorded in ADR-0033.

## Verification plan

- **Automated:** the unit suites named in the steps, plus `tests/notify.rs`,
  `tests/notify_channels.rs`, `tests/monitor_e2e.rs`, `tests/reload.rs`, `tests/cli.rs` and
  `tests/config_precedence.rs`.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo
  test`.
- **Manual:** `cargo run -- monitor --target 127.0.0.1 --show-config` prints each channel once
  with `<redacted>` secrets plus the root and per-target `notify` lists; a local listener (the
  command is in `docs/NOTIFICATIONS.md`) shows the JSON POST with the bearer header; a target
  that states `notify = ["audit"]` writes only its own events to that file while the root list
  still sends `READY` to Telegram; `tail -f` on the file shows one line per event with the
  chosen logger name in text mode, and one JSON document per line in `jsonl` mode; `simulate`
  is unchanged because it starts no channel.
- **Rollback:** a revert of the change; the legacy `[telegram]` alias means the old binary still
  reads a file that kept that section, but a file written only in the new form needs the same
  revert. No schema change and no data migration.

## Open questions

- [x] The defaults the interview could not confirm (the operator was unavailable) — **Decided:**
  unified `[[notification]]` declarations with a legacy `[telegram]` alias; enablement by name
  through the root and per-target `notify` lists; one queue, thread and policy per channel; a
  channel that cannot open is fatal at startup; the farewell is attempted once and pending
  incidents are dropped (no bounded drain); logger names `monitor` and `notification`; a token
  requires HTTPS. The maintainer may flip any of these while the plan is `draft`.
- [x] Does the log file channel use the stabilisation window? — **Decided:** yes, uniformly,
  with `stabilize_seconds` defaulting to `60`; `0` logs every recorded transition. The request
  named only filename, logger, level and format; the window is added so that one message model
  serves every channel.
- [x] Which output formats does the log file channel offer? — **Decided:** `format = "text" |
  "jsonl"`, `text` by default. `text` is the stderr line; `jsonl` is the webhook document plus
  a top-level `logger` member, one compact document per line, so the same event reads the same
  way wherever it is consumed.
- [x] How is a channel scoped? — **Decided:** it is not scoped; it is declared once and enabled
  by name. The root `notify` list applies to global events and to every target, a target's list
  replaces it for that target, an absent key inherits it, and `notify = []` silences the target.
  A channel is never configured twice, so there is no scope, level or format precedence to
  explain.
- [x] What identifies a channel, and when may one event reach it twice? — **Decided:** `name`
  is how it is referenced and the output destination is what makes it unique: the normalised
  filename, the canonical URL, the `chat_id`. Two channels on one destination are refused, so a
  destination receives one copy of an event.
- [x] Is a declared channel that nothing enables allowed? — **Decided:** no, it is a
  `ConfigError`; a forgotten declaration or a typo in a name must not pass as silence.
- [x] Does `--show-config` keep printing `[telegram]`? — **Decided:** no; it prints the resolved
  `[[notification]]` declarations and the routing lists, which are also the documented form.
- [x] One runbook or one per channel? — **Decided:** one `docs/NOTIFICATIONS.md`, because the
  three channels share the enable, verify and troubleshoot skeleton.

## Implementation checklist

One item per step, in order. Tick items as they are completed — this is the live progress log
used to resume a session.

- [x] Step 1 — Name the loggers and carry the name in every line
- [x] Step 2 — Define the message
- [x] Step 3 — Move the policy onto messages
- [x] Step 4 — Publish the channel contract
- [x] Step 5 — Run one queue and one thread per channel
- [x] Step 6 — Make Telegram a channel
- [x] Step 7 — Add the webhook channel
- [x] Step 8 — Add the log file channel
- [x] Step 9 — Declare the channels and route the events
- [x] Step 10 — Wire the process
- [x] Step 11 — Adapt and extend the tests
- [x] Step 12 — Update the documentation
- [x] Step 13 — Run the quality gate
- [x] Step 14 — Align the simulated client's read timeout with the budget it asserts
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration
      option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### 2026-11-16 — Run 2 (implementation complete)

**Outcome.** All thirteen steps are implemented and the full quality gate passes:
`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` → green
(385 unit tests, 26 CLI, 8 precedence, 6 end-to-end, 7 notify, 3 channel, 7 reload, 18 import,
60 web, 7 simulation, 4 store; the privileged suites are ignored as before).

#### Decisions

- **`start_with` takes a channel builder.** `start_with(&config, F, &shutdown)` where
  `F: Fn(&ChannelSettings) -> Box<dyn Channel>` builds one channel per declared entry. That is
  the seam `tests/notify.rs` and the runner unit tests use to hand in one `FakeChannel` each.
- **`NotificationConfig::channel(name)`** is a small lookup used by the tests and by the
  startup lines; `NotificationConfig::is_empty` was not needed and was not kept.
- **Legacy alias.** `[telegram]` declares a channel named `telegram` and appends it to the root
  list (deduplicated). It is an error when the name or the `chat_id` is already declared.
- **Log-file identity.** `normalise_path` makes the filename absolute and folds `.`/`..`
  lexically; symlinks stay distinct, as the plan states.
- **Webhook display.** `Destination` renders `scheme://host[:port]`; `--show-config` prints the
  full URL without the default port (`url_text`), which is what the operator wrote.
- **`Message`/`Event` re-exports.** Both are public at `notify` so the monitor and the tests
  name the model without reaching into `notify::message`.
- **`config::Telegram` gained `name`**, so the transport carries the configured channel name.

#### Deviations

- **Steps 3–5 ran as one compiling unit.** Replacing `policy::Notification` with `Message`
  breaks the old runner, queue, fakes and monitor at once, so `notify` was rewritten and then
  the three step verifies were run (`cargo test --lib notify::policy`, `notify::channel`,
  `notify`). No step was skipped; only the compile checkpoint moved.
- **The config types of step 9 were defined before step 5's runner**, because `notify::start`
  takes `&NotificationConfig`. Resolution, validation and `to_toml` were completed with step 9.
- **The runbook was moved with `mv`**, not `git mv`: the profile keeps Git writes off.
- **`uptime-monitor.toml` (the untracked local copy) still names the old runbook.** It is not
  a tracked repository file and was deliberately left untouched; `git grep` over tracked files
  finds no stale reference.

#### Discoveries

- The CLI tests asserted the old log prefix; they now assert `INFO  monitor …` and the new
  startup lines. No behavioural defect was found in the probe, store, web or simulation paths.
- The `ChannelStarted` message is delivered on the channel itself even for a target-only
  channel, which is what lets a file that follows one target state that it began.

#### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo test` — all suites green as listed above.
- Documentation: `docs/USER-GUIDE.md`, `docs/PRODUCT.md`, `docs/ARCHITECTURE.md`,
  `docs/ADR.md` (ADR-0033, ADR-0027 superseded), `docs/GLOSSARY.md`,
  `docs/NOTIFICATIONS.md` (renamed), `docs/DEVELOPER-GUIDE.md`, `README.md`, `AGENTS.md`,
  `NOTES.md`, `uptime-monitor.sample.toml`, `site/index.html`.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-16
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(notify): deliver notifications through declared channels`
- **Quality gate**: tests ✓ (532 passed, 10 ignored, 0 failed) | lint ✓ | types ✓
  (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/config.rs` (tests) | The `webhook` channel has no configuration test. The successful branch of `resolve_channel` for `webhook`, `canonical_url` and `url_text` are never exercised, two channels on one URL (host case, resolved default port) are never refused, and no `--show-config` prints a webhook. Step 9 names "a duplicate destination (`./a.log` versus `a.log`, host case, default port)" and step 11 names "a `--show-config` with one channel of each type" in their `Verify:` lines; neither exists. | open |
| 2 | major | `src/config.rs` (tests) | The channel validation rules of step 9 run unasserted: a key of another type, an invalid `name`, an invalid `logger`, an unknown `type`, an invalid `level`, an invalid `format`, and the `[telegram]` clash (the name and the `chat_id` already declared) have no test. Each refuses the file correctly when probed by hand (see evidence below), so this is missing coverage, not a defect. | open |
| 3 | major | `docs/USER-GUIDE.md` | The notification tables gave no accepted values for `name` or `logger`; the section did not state that `[[notification]]` is a top-level table; the JSON paragraph named the members `startup`, `reload` and `channel` without stating the `event` values (`config_reload`, `channel_started`); the `--target` paragraph omitted that a channel enabled only by a file target then receives no target event; `notify` was absent from the global and the common-target key tables. | fixed in review |
| 4 | major | `docs/NOTIFICATIONS.md` | The "Reading the log" example put a label in the target column of a `sent` line, while the line carries the target key; the silencing recipe said to remove the name from the root list, which now ends in the orphan error when nothing else enables the channel. | fixed in review |
| 5 | major | `docs/DEVELOPER-GUIDE.md` | "Adding a test" still described `notify::start_with` taking a `notify::fake::FakeSender` and the policy reading the `Instant` of a `Notification`; both were replaced by `FakeChannel` and `Message`. | fixed in review |

Evidence that the untested rules behave as documented (each run exits `1` before the first
probe, and none prints a secret):

```text
notification[0].filename: does not apply to this channel type
notification[0].type: must be telegram, webhook or logfile, not 'email'
notification[0].name: may contain only ASCII letters, digits, '.', '_' and '-'
notification[0].logger: may contain only ASCII letters, digits, '.', '_' and '-'
telegram: the channel name 'telegram' is already declared in [[notification]]
telegram.chat_id: channels 'dup' and 'telegram' both write to chat -100
notification[0].format: must be text or jsonl
notification[0].token: requires an https URL, so the token is not sent in clear
channels 'a' and 'b' both write to /tmp/review-smoke/a.log      # ./a.log versus a.log
```

The documentation fixes were re-checked after the edit: `git grep "TELEGRAM-NOTIFICATIONS"`
finds nothing outside `tech-plans`, `[telegram]` appears only as the legacy alias, every field
of `FileNotification` and both `notify` lists appear in the guide, the guide still has no link
outside itself, and `What the tool does` is three lines.

### Verification matrix

| # | Check | Result and evidence |
|---|---|---|
| 1 | Goals achieved and observable | ✓ 9/9. Live run of `monitor` with a log file channel: `<RFC 3339> <LEVEL> <logger> [<target>] <message>` lines, `notification channel audit (logfile) configured for …`, and a file holding `channel_started`, `startup`, `target_down`, `shutdown` once each, mode `0600`. `--show-config` prints the declarations in declaration order with `bot_token` and `token` as `<redacted>` and the root `notify` list before the tables. `--target` against a target-only channel starts it, writes its `CHANNEL` line and no target event. |
| 2 | Scope | ✓ 27 modified + 6 untracked source/test/doc files, all inside `## Affected modules`; `src/cli.rs` and `Cargo.toml` are listed but needed no change (the version bump belongs to the close phase). |
| 3 | Quality suite | ✓ `cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings` clean, `cargo test` 532 passed / 0 failed / 10 ignored. |
| 4 | Tests on real code paths | ✗ finding 1 and 2. The transports are tested (`webhook.rs`, `logfile.rs`, `tests/notify_channels.rs` against a real listener and files), the delivery rules and routing are tested (`channel.rs`, `mod.rs`, `tests/notify.rs`), but the webhook configuration path and seven validation rules are not. |
| 5 | No test weakened | ✓ the removed assertions are the migrated ones (`INFO  ` → `INFO  monitor `, `announce`/`notifications()` → `send`/`messages()`, `an_announcement_reaches_the_channel_as_it_is` → `the_startup_message_reaches_the_root_list`, `one announcement per start` → `one startup message per start`); no `#[ignore]` was added. |
| 6 | Checklist, questions, notes | ✓ 18/18, every open question `[x]` with a decision, Implementation Notes present with decisions, deviations, discoveries and verification. |
| 7 | Documentation contracts | ✓ with findings 3–5 fixed here: PRODUCT (`FR-2`, `FR-15`, `FR-25`, `FR-28`, `FR-29`, `BR-8`), ARCHITECTURE (component table, `notify` contract, Logging, Configuration, Security, Reliability, `INV-14`), ADR-0033 with ADR-0027 superseded, glossary terms, runbook, developer-guide recipe, README, AGENTS.md, NOTES.md, sample config, landing page. |
| 8 | No contradiction | ✓ the `notify` contract in the architecture matches the trait, the queue capacity, the retry schedule and the routing actually implemented; `INV-14` matches `resolve_notifications`. |
| 9 | No scope creep | ✓ no refactor outside the plan; `tls::connect` and `logging::format_line_at` are the two extractions the plan describes. |
| 10 | Edge cases and error paths | ✓ queue drop-oldest, five attempts with the payload rendered once, non-retryable answers, a `None` payload, the level filter, the duplicate destination normalisation (`./a.log` versus `a.log`), the empty list silences, the `--target` override, and every configuration refusal verified live. |
| 11 | Security | ✓ secrets are file-only, `<redacted>` in `Debug`, `to_toml` and `--show-config` (verified live), scrubbed from `SendError`, refused over `http`; `Destination` renders `scheme://host[:port]` only, so the startup line and the duplicate error carry no URL path; the log file is `0600` and its transport errors go to stderr; a `3xx` is not followed. |
| 12 | Security plan | not applicable (type `feature`). |
| 13 | User guide | ✗ → fixed in review (finding 3). After the fix, every option the change adds is documented with default and accepted values, the guide links to nothing outside itself, and the summary is three lines. |

### Gap analysis

- Goals: 9/9 achieved.
- Steps: 13/13 implemented. Step 9 and step 11 are `partial` as *tests* — findings 1 and 2.
- Scope: in scope. No out-of-scope file.
- Deviations: steps 3–5 compiled as one unit, the step-9 configuration types defined before the
  step-5 runner, `mv` instead of `git mv` (Git writes are off), and the untracked local
  `uptime-monitor.toml` left untouched. All recorded in Implementation Notes and acceptable.
- Additions: none beyond the plan.
- Deferred: `Cargo.toml` `0.8.0` (`version_bump: minor`) belongs to `spec-close`; nothing else.

### Known limitations

- The `[telegram]` alias stays for one release, so two ways to configure Telegram exist until
  it is removed.
- `Destination` renders the canonical URL, whose resolved port is always present, so a startup
  line reads `https://host:443` where the documents write `scheme://host[:port]` (nit).
- No rotation, compression or retention for a log file channel: the operator owns the file.

### Recommendations for future work

- Close findings 1 and 2 in `src/config.rs` (and one `tests/cli.rs` case for a webhook), then
  re-review: the behaviour is already correct, so the work is additive tests only.
- Consider asserting the absence of the URL path in the webhook startup line, next to the
  existing `Webhook` `Debug` assertion, so the host-only rendering cannot regress unnoticed.

### 2026-11-16 — Run 3 (review findings 1 and 2 closed)

**Outcome.** The two open findings of Review Notes round 1 are closed with tests only: no
production line changed. Quality gate re-run: `cargo fmt --check` clean,
`cargo clippy --all-targets -- -D warnings` clean, `cargo test` → 543 passed, 0 failed,
10 ignored (394 unit, 28 CLI, 8 precedence, 1 privileged, 18 import, 6 end-to-end, 7 notify,
3 channel, 7 reload, 7 simulation, 4 store, 60 web).

#### Finding 1 — the webhook configuration path

`src/config.rs`:
- `a_webhook_carries_its_url_token_and_window` — the successful branch of `resolve_channel`
  for `webhook`: name, token, parsed `host`/`port`/`target`, an explicit `stabilize_seconds`,
  and the defaults (`token` absent, `60 s`) when the optional keys are omitted.
- `a_webhook_destination_names_the_host_and_never_the_path` — `Destination` renders the host
  only, so a path that is itself a secret does not reach a log line.
- `two_channels_may_not_share_a_url` — `canonical_url` refuses two channels on one URL, for
  a host written in another case and for the resolved default port (`:443` on `https`, `:80`
  on `http`), and the error names the host without the path.
- `the_rendered_webhook_omits_the_default_port_and_round_trips` — `url_text` prints
  `https://host/path?query` and `https://host:8443/path`, the token is `<redacted>`, and the
  printed file parses back into the same destinations and routing.

`tests/cli.rs`:
- `show_config_prints_a_webhook_without_its_token` — a `--show-config` with a webhook: the URL
  as written, `token = "<redacted>"`, the secret absent.
- `the_startup_log_names_a_webhook_by_host_only` — the startup line reads
  `notification channel hook (webhook) configured for http://127.0.0.1:1`, the path absent.

#### Finding 2 — the validation rules

`src/config.rs`:
- `a_key_of_another_channel_type_is_refused` — a `telegram` declaration carrying `filename`.
- `an_invalid_channel_name_or_logger_is_refused` — a name with a space, a logger with `/`.
- `an_unknown_channel_type_is_refused` — `type = "email"`.
- `an_invalid_level_or_format_is_refused` — `level = "verbose"`, `format = "xml"`.
- `a_telegram_section_may_not_repeat_a_channel_name_or_a_chat` — the legacy section against a
  declared name `telegram`, and against a declared `chat_id`.

#### Discovery

- `to_toml` prints `token = "<redacted>"`, so a printed configuration can never parse back
  into the same `Webhook` value: the round trip is asserted on the destinations and the
  routing, not on the token. The first draft of the new test compared whole
  `NotificationConfig` values and failed on exactly that; the assertion now states the
  property that has to survive, and the existing `the_rendered_routing_round_trips` compares
  the same way.

#### Verification

- `cargo test --lib config` — 44 passed.
- `cargo test --test cli` — 28 passed.
- `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test` — green as
  listed above.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-16
- **Verdict**: PASS
- **Suggested commit**: `feat(notify): deliver notifications through declared channels`
- **Quality gate**: tests ✓ (543 passed, 10 ignored, 0 failed) | lint ✓ | types ✓
  (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`, re-run in
  this round on the final tree)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/config.rs` (tests) | The `webhook` configuration path had no test: `canonical_url`, `url_text`, the successful `resolve_channel` branch and the duplicate-URL refusal. | fixed in round 1: `a_webhook_carries_its_url_token_and_window`, `a_webhook_destination_names_the_host_and_never_the_path`, `two_channels_may_not_share_a_url` (host case, `:443`/`:80`), `the_rendered_webhook_omits_the_default_port_and_round_trips`, plus `tests/cli.rs` `show_config_prints_a_webhook_without_its_token` and `the_startup_log_names_a_webhook_by_host_only` |
| 2 | major | `src/config.rs` (tests) | Seven channel validation rules of step 9 ran unasserted. | fixed in round 1: `a_key_of_another_channel_type_is_refused`, `an_invalid_channel_name_or_logger_is_refused`, `an_unknown_channel_type_is_refused`, `an_invalid_level_or_format_is_refused`, `a_telegram_section_may_not_repeat_a_channel_name_or_a_chat` |
| 3 | major | `docs/USER-GUIDE.md` | Notification tables without accepted values, top-level position, wire `event` values, the `--target` consequence, and `notify` rows. | fixed in round 1 |
| 4 | major | `docs/NOTIFICATIONS.md` | A `sent` example with a label in the target column, and a silencing recipe that now trips the orphan error. | fixed in round 1 |
| 5 | major | `docs/DEVELOPER-GUIDE.md` | "Adding a test" described the removed `FakeSender` and `Notification`. | fixed in round 1 |

No open finding remains. The new tests are additive: `git diff tests/cli.rs` removes only the
round-1 log-prefix and `send` migrations, and no existing test was modified in round 2.

### Verification matrix

| # | Check | Result and evidence |
|---|---|---|
| 1 | Goals achieved and observable | ✓ 9/9 (unchanged by round 2; production code was not touched). |
| 2 | Scope | ✓ round-2 files are `src/config.rs` (tests module only) and `tests/cli.rs`, both in `## Affected modules`; the rest of the change set is the round-1 diff. |
| 3 | Quality suite | ✓ `cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings` clean, `cargo test` 543 passed / 0 failed / 10 ignored (394 unit, 28 CLI). |
| 4 | Tests on real code paths | ✓ findings 1 and 2 closed. The webhook identity is asserted through `resolve_channel` → `Destination` → `to_toml` → `parse` → `resolve`, and each new test cannot pass on the pre-change code: the webhook branch did not exist, `url_text` and the duplicate-URL refusal did not exist, and the seven rules refused nothing. |
| 5 | No test weakened | ✓ additions only in round 2. |
| 6 | Checklist, questions, notes | ✓ 18/18, open questions resolved, Implementation Notes carry Run 1, Run 2 and Run 3 with the findings they close. |
| 7 | Documentation contracts | ✓ round-1 corrections hold; a test-only round leaves no document stale (the developer guide lists test files, not counts). |
| 8 | No contradiction | ✓ unchanged. |
| 9 | No scope creep | ✓ tests only. |
| 10 | Edge cases and error paths | ✓ added: host case and the resolved default port in the destination identity, the path-free error and startup line, the redacted round trip, and the two `[telegram]` clashes. |
| 11 | Security | ✓ added: assertions that the URL path never reaches the startup line or the duplicate error, and that a webhook token prints `<redacted>` in `--show-config`. |
| 12 | Security plan | not applicable (type `feature`). |
| 13 | User guide | ✓ corrected in round 1 and unchanged since; every new option has its default and accepted values, no link leaves the file, the summary is three lines. |

### Gap analysis

- Goals: 9/9 achieved. Steps: 13/13 done; the test gap of steps 9 and 11 is closed.
- Scope: in scope.
- Deviations: as recorded in round 1 and Run 3 (the round-trip assertion compares
  destinations instead of the redacted token).
- Additions: none beyond the plan.
- Deferred: `Cargo.toml` `0.8.0` through `version_bump: minor` at close.

### Known limitations

- The `[telegram]` alias remains for one release.
- A startup line and a duplicate error name a webhook by the canonical host and its resolved
  port (`https://host:443`), where the documents write `scheme://host[:port]`: both identify
  the same endpoint.
- A printed `--show-config` cannot be written back as a working file for a channel that holds
  a secret, because the secret reads `<redacted>`; the destinations and the routing do survive
  the round trip.

### Recommendations for future work

- When the `[telegram]` alias is removed, drop `resolve_telegram` and the two clash cases with
  it, and record the removal in a new ADR.
- A future channel type should add its `Destination` variant and its `url_text`-like rendering
  in the same change, and test the duplicate-destination refusal at configuration level, which
  is where INV-14 is enforced.

### 2026-11-16 — Run 4 (step 14: the close-blocking timing issue, user-approved extension)

**Why.** The close phase could not re-run the quality suite green: `tests/simulate.rs`
`every_answer_costs_what_it_returns_at_the_widest_settings` failed intermittently, in code
this plan never touched (`tests/simulate.rs` and `src/web` are byte-identical to `main`). The
user directed that the timing be fixed in this plan rather than in a separate one, so step 14
was added with its checklist item and `tests/simulate.rs` was added to Affected modules.

**Cause.** `Sim::response` set a 20 second socket read timeout, while the test allows
`/api/v1/faults` 60 seconds and only requires the answer to be fast. At
`--history 3660d --fault-rate 1000` the fault count of ten simulated years (five targets,
3 660 days, a thousand faults a day) takes about 21 seconds: measured at 21.31 s against the
built binary, with the answer stating `"total": 7430422` when the client waits. The client
therefore timed out mid-body, and the failure surfaced as a missing `"total"` member — the
body truncated before the trailing `meta` chunk — instead of as the timing the test is about.

**Change.** `tests/simulate.rs` (tests only, no production line):
- One documented `READ_TIMEOUT: Duration = Duration::from_secs(70)` — above the 60 second
  budget the test states, which is the contract under test — replaces the literal 20 s.
- A read that fails now names the route and the timeout
  (`panic!("{path}: the answer did not arrive within {READ_TIMEOUT:?}: {error}")`), so a
  future stall says what stalled instead of surfacing as a wrong-body assertion.

**Verification.**
- `cargo test --test simulate` — three consecutive runs, `7 passed` each, 22.51 s / 23.00 s /
  22.42 s.
- `cargo fmt --check` clean, `cargo clippy --all-targets -- -D warnings` clean,
  `cargo test` → 543 passed, 0 failed, 10 ignored, every suite green.

**Known limitation, recorded rather than changed.** The widest-settings count is arithmetic
over generated draws — no event, no fault, no string is kept — but it is still O(days x rate)
and costs about 21 seconds for ten simulated years at a thousand faults a day. It stays within
the 60 second budget the test states; making it cheaper is a simulation performance change and
does not belong to this plan.

## Review Notes

- **Round**: 3
- **Review date**: 2026-11-16
- **Verdict**: PASS
- **Suggested commit**: `feat(notify): deliver notifications through declared channels`
- **Quality gate**: tests ✓ (543 passed, 10 ignored, 0 failed) | lint ✓ | types ✓
  (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`, executed on
  the final tree in this round)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `tests/simulate.rs` | The simulated-mode client waited 20 s for an answer while its own widest-settings test allows 60 s, so the fault count of ten simulated years (measured 21.31 s) truncated the body and failed the suite intermittently. Found by the close phase, fixed in step 14 at the user's direction. | fixed in round 3 |

No open finding. Round-1 and round-2 findings remain closed; the documentation corrections of
round 1 and the tests of round 2 are unchanged.

### What changed, and why it is not a weakened test

`tests/simulate.rs` (+12/−2): one `READ_TIMEOUT = 70 s` constant replacing the literal 20 s, and
a read failure that names the route and the timeout. No assertion was touched: the elapsed
budgets (10 s for the page routes, 60 s for the count), the status check and the `"total"`
check are exactly as they were, so a route that genuinely exceeds its budget still fails.

### Verification matrix

| # | Check | Result and evidence |
|---|---|---|
| 1 | Goals achieved and observable | ✓ 9/9 (production code unchanged in rounds 2 and 3; the round-1 live evidence stands). |
| 2 | Scope | ✓ `tests/simulate.rs` and the plan file only; `tests/simulate.rs` was added to `## Affected modules` when the user directed the step. |
| 3 | Quality suite | ✓ fmt clean, clippy clean, 543 passed / 0 failed / 10 ignored, 15/15 suites ok. |
| 4 | Tests on real code paths | ✓ and stable: `cargo test --test simulate` three times, 7 passed each (22.51 s / 23.00 s / 22.42 s), where the same test failed twice before the change. |
| 5 | No test weakened | ✓ no assertion changed; only the client timeout and the failure message. |
| 6 | Checklist, questions, notes | ✓ 19/19 (step 14 added and ticked), `validate --strict` clean, notes record cause, change and verification. |
| 7 | Documentation contracts | ✓ no document is stale: the developer guide lists test files, not timeouts; the runbook and guide describe operator behaviour, which this change does not touch. |
| 8 | No contradiction | ✓ `FR-26`'s claim is about how an answer is read (one page of memory, keyset pages), which holds; the 21 s count stays inside the 60 s budget the test states, and is recorded as a known limitation rather than presented as fast. |
| 9 | No scope creep | ✓ tests only; the simulation performance is reported, not rewritten. |
| 10 | Edge cases | ✓ a stalled route now fails with the route and the timeout in the message instead of a wrong-body assertion. |
| 11 | Security | ✓ no production line, no new input path. |
| 12 | Security plan | not applicable (type `feature`). |
| 13 | User guide | ✓ unchanged by this round and still accurate; every option documented with default and accepted values, no external link, summary three lines. |

### Gap analysis

- Goals: 9/9. Steps: 14/14 (step 14 added by the user during round 3).
- Scope: in scope.
- Deviations: none new.
- Additions: step 14, user-approved and recorded in the notes with its own checklist item.
- Deferred: `Cargo.toml` `0.8.0` through `version_bump: minor` at close.

### Known limitations

- The widest-settings fault count costs about 21 s for ten simulated years at a thousand faults
  a day: arithmetic over the generated draws, but O(days x rate). Inside the 60 s budget; a
  cheaper count is a simulation change for its own plan.
- Carried from round 2: the `[telegram]` alias for one release; the canonical URL shows its
  resolved port where the documents write `scheme://host[:port]`; a printed configuration
  cannot be written back as a working file when a channel holds a secret.

### Recommendations for future work

- If the simulated count has to be instant at the widest settings, make it a closed form over
  the per-day draw instead of generating every draw, and lower the 60 s budget only once it is
  genuinely fast.

