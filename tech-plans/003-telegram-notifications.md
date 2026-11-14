---
title: Notify target state changes over Telegram with a stabilised up window
date: 2026-11-14
status: reviewed
type: feature
impact: medium
area: config, monitor, notify, documentation
version_bump: minor
tags: [telegram, notifications, config]
depends_on: []
branch_recommended: false
---

## Summary

Add an optional, opt-in Telegram channel that tells the operator when a target changes
state. A `down` notification is sent as soon as the transition is recorded; an `up`
notification is sent only after the target has stayed `up` continuously for
`stabilize_seconds` (default `60`), and a `down` is repeated only after an `up` has been
sent. Configuration lives in one global `[telegram]` section of the TOML file; without it
the tool behaves exactly as today, with no network access beyond the probes. Recording,
the database schema, the query command, and the web page do not change.

## Why

The owner runs the monitor on a NAS and learns about an outage only by opening the page or
querying the database. The tool deliberately records only transitions, so the signal is
already precise; what is missing is a push. Today the only options are an external script
polling the SQLite file or a full uptime platform, both of which the project exists to
avoid.

A naive push is worse than none: with `interval_ms` of a few seconds, an unstable link
produces a message per probe and the operator mutes the channel. The requirement is
therefore not "send a message" but "send one message per incident, with a stable recovery
statement".

## Goals

- With `[telegram]` absent or removed, the process makes no outbound connection other than
  the configured probes, and every existing test passes unchanged.
- A recorded `up` -> `down` transition produces exactly one Telegram message, stating the
  target, the word `DOWN`, and the cause (code and message) of the failure.
- A `down` -> `up` transition produces a Telegram message only after `up` has held
  continuously for `stabilize_seconds`; a transition back to `down` inside that window
  produces no message at all.
- After an `up` message has been sent, the next `down` transition produces a new `down`
  message; a `down` that follows an already-notified `down` produces nothing.
- The first verdict of a target after a start or a restart notifies its observed state
  (`unknown` -> `up` is held by the same stabilisation rule as any other `up`).
- A failed delivery is retried five times with waits of 1, 2, 4, and 8 seconds; a message
  that still fails is logged as one `WARN` line and never blocks a probe cycle.
- The bot token never appears in `--show-config`, in a log line, or in an error message.
- `--show-config` prints the effective `[telegram]` section with the token redacted, and
  the file-only configuration of the section is documented as a deliberate exception to
  `FR-2`.

## Non-goals

- Notification transports other than Telegram: no webhook, mail, MQTT, Slack, or generic
  command hook. The database stays the integration point for everything else.
- Per-target notification settings, quiet hours, message templates, or a per-target
  opt-out: the section is global and the message format is fixed.
- Persisting the notification state: a restart re-notifies the first observed state, which
  the operator asked for.
- Retrying a lost notification after a restart, or storing an outbox in the database.
- Changing how transitions are detected or persisted; notification is a side effect of an
  already-committed event.
- `serve` (the process without probing) gains no notification behaviour.

## Affected modules

**Source**
- `src/config.rs` — `Telegram` and `FileTelegram`, `DEFAULT_STABILIZE_SECONDS`, the
  `telegram` field of `Config` and `FileConfig`, validation in `Config::resolve`, the
  redacted rendering in `Config::to_toml`, and the unit tests of each.
- `src/lib.rs` — publish `notify` and `tls`.
- `src/tls.rs` — *(new)* one `ClientConfig` built from the embedded roots, shared by the
  HTTP probe and the Telegram client.
- `src/probe/http.rs` — drop the local `tls_config` and call `crate::tls::config`; no
  behavioural change.
- `src/shutdown.rs` — `pub fn pause(duration, shutdown)`, moved here from `monitor`.
- `src/notify/mod.rs` — *(new)* `Notification`, `Notifier`, `NoopNotifier`,
  `ChannelNotifier`, `Engine`, `deliver`, `start`, `start_with`.
- `src/notify/policy.rs` — *(new)* the per-target `Policy` that decides when a state is
  reported.
- `src/notify/queue.rs` — *(new)* the bounded queue that drops the oldest message when
  full.
- `src/notify/telegram.rs` — *(new)* the HTTPS `sendMessage` client, the form encoding,
  and the classification of a response.
- `src/notify/fake.rs` — *(new)* `FakeSender` and `FakeNotifier` for the tests.
- `src/monitor/mod.rs` — hand the notifier to every worker, notify after the event is
  written, `run_with_notifier`, and refuse a live change of the `[telegram]` section.

**Tests**
- `src/config.rs` — *(extend unit tests)* parsing, defaults, validation, redaction.
- `src/notify/policy.rs` — *(unit tests)* the suppression rules of the Goals.
- `src/notify/queue.rs` — *(unit tests)* drop-oldest at capacity.
- `src/notify/telegram.rs` — *(unit tests)* encoding, request text, response
  classification, retry count, and the absence of the token from an error.
- `tests/notify.rs` — *(new)* the engine with a fake sender: immediate `down`, deferred
  `up`, a flap inside the window, a retry sequence, and a full queue.
- `tests/monitor_e2e.rs` — *(extend)* one notification per recorded transition, through
  `run_with_notifier` and a `FakeNotifier`.
- `tests/reload.rs` — *(extend)* a changed `[telegram]` section is reported and not applied
  while the monitor runs.

**Documentation / configuration**
- `docs/PRODUCT.md` — move alerting out of "Out of scope" into "In scope" for Telegram
  only, add `FR-25`, update the business rule that declares alerting out of scope, and
  resolve the "Alerting hooks" open question.
- `docs/ARCHITECTURE.md` — context diagram and table (Telegram Bot API), building block
  and contract for `notify`, a solution-strategy bullet, the egress constraint, `INV-11`.
- `docs/ADR.md` — `ADR-0026`, the decision and its rejected alternatives.
- `docs/GLOSSARY.md` — **Notification**, **Stable `up` window**, **Chat ID**, **Bot
  token**.
- `docs/USER-GUIDE.md` — a new `### Telegram notifications` subsection of `Configuration`
  with every key and its default, the setup steps (self-contained), the message format, and
  a `Troubleshooting` entry for a silent channel.
- `docs/TELEGRAM-NOTIFICATIONS.md` — *(new)* the runbook: bot, channel, permissions,
  `chat_id`, a `curl` test, and failure modes.
- `docs/DEVELOPER-GUIDE.md` — a note in `### Adding a configuration option` about the
  file-only secret, and how to test the notifier without the network.
- `AGENTS.md` — one row for the runbook in the project-documents table.
- `README.md` — one line in `## Quick start` and one row in `## Documentation`.
- `network-monitor.toml` — a commented `[telegram]` example with no real token.

## Architecture impact

A new component, `notify`, owns the notification decision and delivery. It depends on
`config` (the settings), `logging`, `shutdown`, and `tls`; `monitor` depends only on its
published contract (`Notifier`, `Notification`) and never on its internals, so the debounce
logic and the delivery are testable without a network and without raw sockets. The process
gains one optional outbound dependency on `api.telegram.org` over TLS, which must be
recorded in the context view and in the constraints; recording remains independent of it
(`INV-11`). No schema change, no new crate, no inbound socket, no change to the query or
web components.

## Decision-record impact

`ADR-0026` — "Notify state changes over Telegram, with a stabilisation window and a
dedicated delivery thread". It records: the reversal of the "alerting is out of scope"
statement, the choice of Telegram over a webhook or a shell hook, the file-only token as an
explicit exception to `FR-2`/`ADR-0005`, the stabilisation rule, the dedicated thread with a
bounded queue, and the rejected alternative of blocking the probe loop.

## Proposed solution

### Step 1 — Add the `[telegram]` configuration section

In `src/config.rs`:

- Add `pub const DEFAULT_STABILIZE_SECONDS: u64 = 60;` and
  `pub const REDACTED_TOKEN: &str = "<redacted>";`.
- Add `#[derive(Clone, PartialEq, Eq)] pub struct Telegram { pub bot_token: String,
  pub chat_id: String, pub stabilize: Duration }` with a hand-written `Debug` that prints
  `bot_token: "<redacted>"`. `Config` derives `Debug`, so the derived form would leak the
  token into any debug log.
- Add `#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)] #[serde(deny_unknown_fields)]
  pub struct FileTelegram { pub bot_token: Option<String>, pub chat_id: Option<String>,
  pub stabilize_seconds: Option<u64> }`, and `pub telegram: Option<FileTelegram>` on
  `FileConfig`; `deny_unknown_fields` makes the key mandatory here before it is accepted.
- Add `pub telegram: Option<Telegram>` to `Config` and resolve it in `Config::resolve`:
  absence means `None`; a present section requires a non-empty `bot_token` that contains no
  whitespace, `/`, `?`, or `#` (it travels in the request path) and a non-empty `chat_id`;
  `stabilize_seconds` defaults to `DEFAULT_STABILIZE_SECONDS`, `0` is allowed and means
  "report `up` at once", there is no upper bound. Every rejection is `ConfigError::Invalid`
  naming `telegram.bot_token`, `telegram.chat_id`, or `telegram.stabilize_seconds`.
- Extend `Config::to_toml` with a `SerializableTelegram { bot_token, chat_id,
  stabilize_seconds }` emitted only when the section exists, the token always
  `REDACTED_TOKEN`.
- Extend the existing round-trip test: the parsed file compares equal except for the token,
  which reads `REDACTED_TOKEN`.

There is no `Overrides` field and no `clap` argument for the section: the operator chose the
file as the only source. Document that exception in the ADR and in the user guide.

**Verify:** `cargo test --lib config` — the new tests cover a full section, defaults, a
missing `bot_token`, a missing `chat_id`, a token with a `/`, and `to_toml` not containing
the real token.

### Step 2 — Extract the shared TLS configuration

Create `src/tls.rs` with `pub fn config() -> &'static Arc<rustls::ClientConfig>`: the body
of the current `probe::http::tls_config`, including the `OnceLock`, the `webpki_roots`
extension, and the `http/1.1` ALPN list. In `src/probe/http.rs`, replace `tls_config()` with
a call to `crate::tls::config()` and keep every failure path exactly as it is. Publish both
`tls` and `notify` from `src/lib.rs`.

**Verify:** `cargo test --lib probe::http` — the transport tests, TLS and local-server ones
included, pass unchanged.

### Step 3 — Add the notification vocabulary and the stabilisation policy

Create `src/notify/policy.rs` with no I/O:

- `pub struct Notification { pub target: String, pub key: String, pub state: TargetState,
  pub cause: Option<String>, pub occurred_at_ms: i64, pub at: std::time::Instant }` and
  `impl Notification { pub fn text(&self) -> String }` rendering plain text:

  ```text
  DOWN cloudflare-dns (icmp://1.1.1.1)
  Cause: ICMP.NO_REPLY — no reply within 2000 ms
  2026-11-14T12:34:56Z
  ```

  and for `up` the first and third lines only. The second line is present only when a cause
  exists. When `target == key` the parenthesised key is omitted. The instant is
  `crate::time::format_rfc3339(self.occurred_at_ms)`.
- `pub struct Policy { stabilize: Duration, notified: Option<TargetState>,
  pending: Option<(Instant, Notification)> }` with:
  - `observe(&mut self, notification: Notification) -> Option<Notification>` — `down`
    clears `pending` and returns the notification only when `notified` is not already
    `down`, setting `notified = down`; `up` returns the notification at once when
    `stabilize` is zero, otherwise stores `(at + stabilize, notification)` when no earlier
    pending `up` exists, and returns `None`. An `up` that is already notified returns `None`.
  - `next_deadline(&self) -> Option<Instant>`.
  - `due(&mut self, now: Instant) -> Option<Notification>` — returns and clears the pending
    notification whose deadline has passed, setting `notified = up`.

Then `pub struct Engine { stabilize: Duration, policies: HashMap<String, Policy> }` in the
same module, keyed by `Notification::key`, with `observe`, `next_deadline`, and
`due_all(now) -> Vec<Notification>`.

**Verify:** `cargo test --lib notify::policy` covers: immediate `down`; `up` held for the
window; `up` cancelled by a `down` inside it; no second `down` after a flap; `down` after a
notified `up`; zero window; and the rendered text with and without a cause.

### Step 4 — Add the bounded queue and the notifier thread

Create `src/notify/queue.rs`: `Queue` holding `Mutex<VecDeque<Notification>>` and a
`Condvar`, with `CAPACITY = 32`, `push(&self, notification) -> bool` (true when the oldest
was dropped to make room), `pop_timeout(&self, Duration) -> Option<Notification>`, and
`close(&self)`.

Create `src/notify/mod.rs` with:

- `pub trait Notifier: Send + Sync { fn notify(&self, notification: Notification); }` and
  `pub struct NoopNotifier` whose `notify` does nothing.
- `pub trait Sender: Send { fn send(&mut self, text: &str) -> Result<(), SendError>; }` and
  `pub struct SendError { pub retryable: bool, pub reason: String }`.
- `pub fn deliver(sender: &mut dyn Sender, text: &str, shutdown: &Shutdown) -> Result<(), SendError>`
  — at most five attempts, waiting 1, 2, 4, and 8 seconds between them with
  `shutdown::pause`; stops at once when `SendError::retryable` is false or the shutdown is
  requested; logs one `WARN` per failed attempt and one `WARN` with `notification lost` at
  the end.
- `pub struct ChannelNotifier { queue: Arc<Queue> }` implementing `Notifier`: `push` and,
  when the push dropped the oldest, one `WARN`. It never blocks.
- `pub struct Handle { pub notifier: Arc<dyn Notifier>, stop: Shutdown, thread: Option<JoinHandle<()>> }`
  with `shutdown(self)` joining the thread, used when the monitor returns.
- `pub fn start(settings: &Telegram, shutdown: &Shutdown) -> Handle` builds a
  `Telegram` sender; `pub fn start_with(settings: &Telegram, sender: Box<dyn Sender>, shutdown: &Shutdown) -> Handle`
  is the seam the tests use.
- The thread loop: on every pass, compute `engine.next_deadline()`, take the smallest of that
  and one second as the wait, then `queue.pop_timeout(wait)`; on a notification pass it to
  `engine.observe`, on a timeout call `engine.due_all(Instant::now())`, and send each
  returned notification with `deliver`, logging `INFO` on success. The loop leaves when the
  queue is closed or the shutdown is requested; pending messages are dropped.

**Verify:** `cargo test --lib notify` — the queue tests prove drop-oldest at 32, and the
engine tests prove that one due notification is returned per deadline.

### Step 5 — Implement the Telegram client

Create `src/notify/telegram.rs`:

- `pub struct Telegram { token: String, chat_id: String }` with `pub fn new(settings: &Telegram) -> Self`
  and a hand-written `Debug` that redacts the token.
- `fn form_encode(value: &str) -> String` — unreserved ASCII characters unchanged, space as
  `%20`, everything else as uppercase `%XX` over UTF-8 bytes.
- `fn request(&self, text: &str) -> String` — `POST /bot<token>/sendMessage HTTP/1.1`,
  `Host: api.telegram.org`, `Content-Type: application/x-www-form-urlencoded`,
  `Content-Length`, `Connection: close`, and the body
  `chat_id=<encoded>&text=<encoded>&disable_web_page_preview=true`.
- `impl Sender for Telegram`: resolve `api.telegram.org:443`, `TcpStream::connect_timeout`
  with a five-second budget, set the read and write timeouts to the same budget, wrap in
  `crate::tls::config()` with `ServerName`, write the request, flush, then read the response
  head and body into one buffer capped at 8 KiB. `classify(status, body)` returns `Ok(())`
  for `200` with `"ok":true`, a retryable error for `429` and `5xx`, and a non-retryable
  error for any other status or for `"ok":false`. No error text may contain the request
  path or the token: the token is in the path, so the errors name the host and the status
  only, plus the `description` field of the body when one is present.

**Verify:** `cargo test --lib notify::telegram` — encoding of `down`, newline, `&`, and a
non-ASCII byte; the request line and body; `classify` for 200/`ok`, 400, 401, 429, 500;
`deliver` with a fake sender that fails twice then succeeds, and one that is never retried.

### Step 6 — Wire notifications into the monitor

In `src/monitor/mod.rs`:

- `Supervisor` gains `notifier: Arc<dyn Notifier>`; `start_workers` clones it into each
  `monitor_target` call.
- In `monitor_target`, after `store.record_transition(&event)?` and before the `INFO` line,
  build a `Notification` from `target.display()`, `target.key()`, `transition.to`,
  `transition.cause` rendered as `format!("{failure}")`, `event.occurred_at_ms`, and
  `Instant::now()`, then call `notifier.notify(...)` and a `DEBUG` line stating that it was
  queued. A store failure still returns before any notification, so an alert never precedes
  the event it describes.
- Add `pub fn run_with_notifier(config, prober, store, shutdown, reload, notifier)` holding
  the current body of `run`; `run` resolves `config.telegram` through `notify::start` or
  uses an `Arc<NoopNotifier>`, calls `run_with_notifier`, then shuts the notifier handle
  down. No existing call site or test changes.
- In `Supervisor::apply`, after the database check, compare the resolved `telegram` with the
  running one: when it differs, log `telegram settings changed; restart the monitor to use
  them` and keep the running value, exactly as the database path is handled. The notifier
  thread is therefore never restarted by a reload.
- Replace the local `pause` with `shutdown::pause`.

**Verify:** `cargo test --test monitor_e2e --test reload` — one `FakeNotifier` notification
per recorded transition, none for a local probe error, and the reload warning.

### Step 7 — Update the documentation and the example configuration

- `docs/PRODUCT.md`: remove Telegram from "Out of scope", state the Telegram channel in
  "In scope", add `FR-25` (the section, the stabilisation rule, the repeat rule, the retry
  policy, the redaction), replace the business rule that declares alerting out of scope with
  the narrower "no transport other than Telegram; the database stays the integration point",
  and resolve the "Alerting hooks" open question.
- `docs/ARCHITECTURE.md`: add Telegram Bot API to the context diagram and the external
  element table, `notify` to the building block table with its contract, a solution-strategy
  bullet, the outbound-egress constraint, and `INV-11` ("a notification is never a condition
  of recording, and a delivery failure never changes a stored event").
- `docs/ADR.md`: append `ADR-0026` and its index row.
- `docs/GLOSSARY.md`: the four terms.
- `docs/USER-GUIDE.md`: `### Telegram notifications` under `Configuration` with the three
  keys, their defaults, and their effect; the bot, channel, and `chat_id` steps;
  the exact message layout; the stabilisation example; a note that the token is the one
  exception to the command-line mirror; and a `Troubleshooting` entry.
- `docs/TELEGRAM-NOTIFICATIONS.md`: the same procedure as a runbook, plus `curl` helpers,
  the `getUpdates` `channel_post.chat.id` recipe, and the delivery failure modes.
- `docs/DEVELOPER-GUIDE.md`: in `### Adding a configuration option`, one sentence on a
  secret that is file-only and redacted; in `### Adding a test`, how `notify::fake` drives the
  delivery without the network.
- `AGENTS.md`: the runbook row. `README.md`: the quick-start line and the documentation row.
- `network-monitor.toml`: a commented `[telegram]` block whose token reads
  `123456789:replace-me`.

**Verify:** `grep -n "telegram" docs/USER-GUIDE.md docs/TELEGRAM-NOTIFICATIONS.md
network-monitor.toml` shows every key of `FileTelegram`, and `cargo run -- --show-config`
prints the section with `<redacted>` and no real token.

## Security considerations

- **Secret at rest.** The bot token lives in the configuration file, which is already the
  file that holds the database path; the runbook states the file permission
  (`chmod 600`) and that the token must never be committed. The example file carries a
  placeholder only.
- **Secret in output.** `Config` and `Telegram` redact the token in `Debug`, `to_toml`
  prints `<redacted>`, and the client never puts the token into an error string, so a
  `WARN` line and `--show-config` are safe to paste into an issue. A unit test asserts it.
- **Transport.** HTTPS to one fixed host with the certificate roots compiled into the
  binary (`ADR-0024`); no proxy support, no way to disable verification, no inbound port.
- **Untrusted input.** The response is bounded at 8 KiB and is only pattern-matched for
  `"ok":true`; it is never executed, stored, or rendered as HTML. The message body is
  percent-encoded, so a label containing `&` or a newline cannot inject a parameter.
- **Content.** Messages carry the target label, its key, the cause and the instant — no
  credentials and no payload from the probed endpoints.
- **Residual risk.** The token is stored in clear text in a file the operator controls, and
  a compromised token lets a third party post to that channel; revoking it in BotFather is
  the documented recovery.

## Verification plan

- **Automated:** the unit and integration tests named in the steps, notably
  `tests/notify.rs` (immediate `down`, deferred `up`, flap suppression, retry, queue
  overflow), `tests/monitor_e2e.rs` (one notification per transition), `tests/reload.rs` (a
  live `[telegram]` change is refused), and the configuration tests (parsing, defaults,
  validation, redaction).
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  (the `all` command of `.spec-workflow.yml`).
- **Manual:** with a real bot and a private channel, stop a target (`tcp` to a closed port)
  and observe one `DOWN` message with its cause; restore it and observe no message for the
  first 60 s, then one `UP`; drop it again inside the window and observe no new message.
  Finally set an invalid token and confirm five `WARN` lines, one per attempt, with no token
  in them.
- **Rollback:** remove the `[telegram]` section. Nothing else changed — no schema, no CLI,
  no file format — so an older binary reads the same database.

## Open questions

- [x] Who reports the state, the notifier or the probe loop? The notifier owns the decision:
      the answer for `up` arrives on a deadline, not on a probe, so a 300 s interval still
      reports a stable recovery within a second of the window closing.
- [x] Where does the token live? Only in the TOML file, redacted everywhere it is printed;
      no flag, no environment variable. The operator chose it and accepted the exception to
      `FR-2`.
- [x] What happens to the notification state across a reload? It survives, because the
      notifier thread is created once and a changed section is applied from the next start;
      the workers restart without losing the pending window.
- [x] Is an initial `unknown` -> `up` reported? Yes, after the same stabilisation window; the
      operator asked for both initial states.
- [x] Does a delivery failure change what is reported next? No: the state machine records an
      attempted notification, so the "one `down` per incident" rule holds even when a message
      is lost, and a lost message is never retried after the fifth attempt.

## Implementation checklist

- [x] Step 1 — `[telegram]` configuration section with redaction
- [x] Step 2 — shared TLS configuration extracted
- [x] Step 3 — notification text and stabilisation policy
- [x] Step 4 — bounded queue and notifier thread
- [x] Step 5 — Telegram HTTPS client and retry policy
- [x] Step 6 — notifier wired into the monitor
- [x] Step 7 — documentation and example configuration
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects the new behaviour and every configuration
      option (`bot_token`, `chat_id`, `stabilize_seconds`), or the plan records why it needs
      no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### Decisions

- **`deliver` counts and logs.** Five attempts, waits of 1, 2, 4, and 8 seconds, one `WARN`
  per failed attempt, and one `WARN notification lost: <reason>` when it gives up — for a
  non-retryable answer as well, so a `401` produces two lines and five retryable failures
  produce six. A `WARN` is emitted at every level, so the operator always sees a lost
  message.
- **`Policy` measures the window from the first `up` of the episode.** A second `up`
  observed while a window is already pending neither restarts nor shortens it; this is what
  makes a 300 s interval still close the window on time.
- **`Notification::text`** uses `crate::time::format_rfc3339`, i.e. millisecond precision
  (`2025-11-24T16:00:00.000Z`), and omits the parenthesised key when the target has no
  label, because then `target == key`.
- **`Queue` holds its items and its closed flag in one mutex** behind one condition
  variable; `pop_timeout` returns `None` on a closed queue, and the delivery loop uses
  `is_closed` to tell a timeout from a close.
- **`Handle` also owns the queue** (a private field) so `shutdown` can close it before
  joining the thread; only `notifier` is public, as the plan's sketch showed.
- **The Telegram client scrubs its own token from any error it returns**, on top of never
  putting the request path into a message, so a future error string cannot leak it.
- **Token validation** refuses whitespace, `/`, `?`, and `#`; `:` is allowed because every
  real BotFather token contains one.

### Deviations

- **`ADR-0026` was already taken.** The repository carries an accepted `ADR-0026` for the
  sandbox skill bind, dated the same day. The notification decision is recorded as
  **`ADR-0027`** in `docs/ADR.md` (index row and record), and every reference points there.
- **`INV-11` was already taken.** `docs/ARCHITECTURE.md` already defines `INV-11` (one
  listener). The new invariant — "a notification is never a condition of recording, and a
  delivery failure never changes a stored event" — is **`INV-12`**.
- **`notify::start` was added in Step 5, not Step 4.** It builds the Telegram client, so it
  cannot exist before `src/notify/telegram.rs`; Step 4 delivered `start_with`, the queue,
  the retries, the fake and the thread loop, and Step 5 added `start` beside the client.
- **`tests/notify.rs` has no "full queue" case.** Drop-oldest at capacity is proven
  deterministically by `notify::queue::tests::a_full_queue_drops_the_oldest`; the public
  `start_with` seam cannot stall the drain without a blocking fake, and a timing-based
  integration test would be flaky. The queue test is the contract.
- **`deliver` retry tests live in `src/notify/telegram.rs`** (the plan listed them there) and
  the engine behaviour is covered by `tests/notify.rs`, so the module keeps one test
  location per concern.
- **Two existing test fixtures gained `..FileConfig::default()`**: `tests/config_precedence.rs`
  and `tests/monitor_e2e.rs` built `FileConfig` exhaustively and had to name the new field.
  No assertion or behaviour of those tests changed.

### Discoveries

- The monitor's `pause` had to move to `shutdown` before the delivery thread could reuse
  it; it is now `shutdown::pause`, and the three call sites in `monitor` use it. No
  behaviour changed.
- `Config::telegram` is compared with `PartialEq` by the supervisor; `Telegram` therefore
  derives `PartialEq`/`Eq` while keeping a hand-written `Debug` that redacts the token.
- The manual verification of the plan (a real bot, a real channel, an invalid token) needs
  network access and a credential, so it was not run in the sandbox. The offline tests
  cover every branch the manual step would exercise: immediate `down`, deferred `up`, a
  flap inside the window, retries, a non-retryable answer, and the absence of the token
  from the rendered output.

### Extensions

- None beyond the plan.

### Review round 1 fixes (2026-11-14)

- **Findings 1 and 2** — the two references still on `ADR-0026` now read `ADR-0027`:
  `docs/PRODUCT.md` (the resolved "Alerting hooks" open question) and `docs/ARCHITECTURE.md`
  (`### Configuration`). A grep of the working tree leaves `ADR-0026` only in the record itself
  and in the unrelated sandbox documents.
- **Finding 3** — `docs/USER-GUIDE.md` no longer links `TELEGRAM-NOTIFICATIONS.md`; it points to
  its own `Troubleshooting` section, so the guide stays distributable on its own. The runbook is
  still referenced from `README.md` and `AGENTS.md`.
- **Finding 5** — `deliver` now delegates to `notify::deliver_with(sender, text, shutdown,
  waits)`, which takes the backoff as a parameter; the production `BACKOFF` constant keeps the
  1, 2, 4 and 8 seconds. This allowed the new
  `notify::telegram::tests::five_failed_attempts_lose_the_message` without a fifteen-second
  wait, and it removed the three seconds the two existing retry tests spent sleeping.
  `tests/notify.rs` still drives the real `deliver` path end to end, so the constant is covered.
- **Finding 6** — the `.gitignore` entry is outside `## Affected modules` and the user approved
  it explicitly in review round 1: it keeps the local `network-monitor-telegram.toml`, which
  holds a real token, out of Git. Recorded here so the next round sees the decision.
- **Finding 4** — no code change: the one-line fixture update in `tests/config_precedence.rs` is
  a consequence of the new `FileConfig` field, already recorded under *Discoveries*.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(notify): add telegram notifications behind a stable up window`
- **Quality gate**: tests ✓ (336 passed, 0 failed) | lint ✓ (0 warnings) | types ✓ (0 errors) —
  `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` (exit 0)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `docs/PRODUCT.md:151` | The resolved "Alerting hooks" open question cites `[ADR-0026](./ADR.md)`, which is the sandbox skills bind. The Telegram decision is `ADR-0027`. | open |
| 2 | major | `docs/ARCHITECTURE.md:298` | The `### Configuration` concept states `(ADR-0026)` for the file-only secret; it is `ADR-0027`. Same renumbering miss as finding 1. | open |
| 3 | major | `docs/USER-GUIDE.md:293` | The new subsection closes with a relative link to `TELEGRAM-NOTIFICATIONS.md`. The user guide declares itself distributable on its own and is otherwise link-free, so an internal link breaks its self-containment. | open |
| 4 | minor | `tests/config_precedence.rs` | Outside `## Affected modules`: one line (`..FileConfig::default()`) forced by the new `FileConfig` field. Recorded in Implementation Notes; no assertion changed. | open |
| 5 | minor | `src/notify/mod.rs` | No test exhausts the retry loop (five attempts, then `notification lost`). `a_retryable_failure_is_attempted_again` stops at three, `a_permanent_failure_is_not_retried` at one, so deleting the cap would keep the suite green. The plan's verification list did not ask for this case. | open |
| 6 | minor | `.gitignore` | Outside `## Affected modules`: one ignored path, `network-monitor-telegram.toml`, so the local file with a real token cannot be committed. Approved by the user in this review round. | approved 2026-11-14 |

### Gap analysis

- **Goals: 5/5 achieved.**
  - Opt-out: `monitor::run` builds a `NoopNotifier` when `config.telegram` is `None`; `config::tests::an_absent_telegram_section_configures_nothing`. Existing suites pass; two fixtures needed `..FileConfig::default()` (findings 4 and the Implementation Notes).
  - `down` at once with target, cause and instant: `tests/notify.rs::a_disconnection_is_delivered_at_once`, `notify::policy::tests::a_disconnection_is_reported_at_once`, `tests/monitor_e2e.rs::each_recorded_transition_is_notified_once`.
  - Deferred `up`, silent flap: `a_recovery_is_delivered_when_the_window_closes`, `a_flap_inside_the_window_sends_nothing`, `a_disconnection_inside_the_window_cancels_the_recovery`.
  - New `down` after a notified `up`, silence on a repeated `down`: `a_disconnection_after_a_notified_recovery_is_reported_again`, `a_flap_reports_one_disconnection_per_incident`.
  - First verdict after a restart: `the_first_verdict_reports_the_observed_state`.
  - Retries: `a_retryable_failure_is_attempted_again`, `a_permanent_failure_is_not_retried`, `a_failed_delivery_is_retried_before_it_gives_up` (finding 5 is the uncovered exhaustion branch).
  - Redaction: `the_rendered_configuration_redacts_the_token`, `the_debug_form_of_the_section_hides_the_token`, `a_reported_failure_never_carries_the_token`, `the_client_debug_form_hides_the_token`, plus `--show-config` inspected by hand (prints `bot_token = "<redacted>"`).
- **Scope:** 27 staged files. 25 are named in `## Affected modules`; `tests/config_precedence.rs` is a documented deviation (finding 4) and `.gitignore` is approved by the user (finding 6). No unrelated work in the staged set; the worktree has no unstaged changes.
- **Deviations:** `ADR-0026` → `ADR-0027` and `INV-11` → `INV-12` (both numbers were already taken by the sandbox work), `notify::start` delivered in Step 5, no full-queue integration test — all four are recorded in Implementation Notes and are legitimate. Findings 1 and 2 are the residue of the ADR renumbering: the record itself is right, two references still say `ADR-0026`.
- **Additions:** none beyond the plan.
- **Deferred:** the manual acceptance of the plan (a real bot, a real channel, a revoked token) needs network and a credential and is left to the operator; the offline suite covers the same branches.

### Known limitations

- The Telegram answer is read to end of stream and matched for `"ok":true`; a chunked body is not de-chunked and a `description` containing an escaped quote is truncated at the escape. Both only affect the text of a `WARN` line, never the recorded history.
- A lost message is never re-sent, and the notification state does not survive a restart, by design (`FR-25`, ADR-0027).
- `stabilize_seconds` has no upper bound.

### Recommendations for future work

- Add the exhaustion test of finding 5: five retryable failures, assert five attempts and one `notification lost` line.
- A cheap documentation check would have caught findings 1 and 2: grep every `ADR-`/`INV-` reference against the index of `docs/ADR.md` and the invariant list in `docs/ARCHITECTURE.md`.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-14
- **Verdict**: PASS
- **Suggested commit**: `feat(notify): add telegram notifications behind a stable up window`
- **Quality gate**: tests ✓ (337 passed, 0 failed) | lint ✓ (0 warnings) | types ✓ (0 errors) —
  `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` (exit 0)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `docs/PRODUCT.md:151` | Cited `ADR-0026` (sandbox skills) instead of `ADR-0027` | fixed in round 2 — line now reads `[ADR-0027](./ADR.md)` |
| 2 | major | `docs/ARCHITECTURE.md:298` | Same wrong record number in `### Configuration` | fixed in round 2 — `(ADR-0027)` |
| 3 | major | `docs/USER-GUIDE.md:293` | Relative link to `TELEGRAM-NOTIFICATIONS.md` broke the guide's self-containment | fixed in round 2 — now `[Troubleshooting](#troubleshooting)`, an intra-document anchor; `grep '](\./'` finds no link in the guide |
| 4 | minor | `tests/config_precedence.rs` | Outside `## Affected modules` | approved by the user in round 2 (one line, `..FileConfig::default()`, no assertion touched) |
| 5 | minor | `src/notify/mod.rs` | The retry loop was never exhausted by a test | fixed in round 2 — `deliver` delegates to `deliver_with(…, waits)`, and `notify::telegram::tests::five_failed_attempts_lose_the_message` drives five retryable failures with an empty backoff |
| 6 | minor | `.gitignore` | Outside `## Affected modules` | approved by the user in round 1 and recorded in Implementation Notes |

No new findings in round 2. Residual `ADR-0026` mentions exist only in `docs/ADR.md` (the record
itself) and `docs/AGENT-SANDBOX.md`, where that number is the correct one.

### Gap analysis

- **Goals: 5/5 achieved**, evidenced in round 1 and re-run green in round 2: opt-out through
  `NoopNotifier`, immediate `down` with cause and instant, `up` held by the window, silence on a
  flap and on a repeated `down`, first verdict after a restart, five attempts then `notification
  lost`, and token redaction in `Debug`, `to_toml`, `--show-config` and every error string.
- **Scope:** 27 files staged against `d5fe825`, 25 named in `## Affected modules`; the two
  exceptions are approved by the user (findings 4 and 6). The fix round touched six files, all
  listed or the plan itself; no untracked files, no unstaged work outside the change set.
- **Deviations:** `ADR-0026` → `ADR-0027`, `INV-11` → `INV-12`, `notify::start` delivered in
  Step 5, no full-queue integration test, and the `deliver_with` seam added in the fix round —
  all recorded in Implementation Notes and legitimate.
- **Additions:** none beyond the plan; the test seam is a refactor inside a module the plan owns,
  made to satisfy finding 5 without a fifteen-second test.
- **Deferred:** the manual acceptance against a real bot and a real channel needs a credential and
  network access and stays with the operator; the offline suite covers the same branches.

### Known limitations

- A chunked answer is not de-chunked, and a `description` containing an escaped quote is truncated
  at the escape. Both affect the text of a `WARN` line only.
- A lost message is never re-sent, and the notification state does not survive a restart, by
  design (`FR-25`).
- `stabilize_seconds` has no upper bound.

### Recommendations for future work

- The test of finding 5 pins the attempt count through the `ATTEMPTS` constant; if the literal
  five becomes a requirement of its own, assert `5` in one place and let the constant follow.
- Add the documentation check suggested in round 1 to whatever script the project runs before a
  release: every `ADR-` and `INV-` reference must resolve.



