---
title: Add an InfluxDB notification channel that writes one point per event
date: 2026-11-18
status: draft
type: feature
impact: medium
area: notify, config, docs
version_bump: minor
tags: [notifications, influxdb, channel, http]
depends_on: []
branch_recommended: false
---

## Summary

A fourth notification channel type, `influxdb`, will send every event the channel receives
to InfluxDB through its HTTP write API, one line-protocol point per event. The channel is
declared once in `[[notification]]` with `name`, `type = "influxdb"`, `url`, `org`, `bucket`
and `token`, and is enabled by name exactly like Telegram, a webhook and a log file; the
routing, the stabilisation window, the per-channel queue and the five-attempt retry are the
existing ones and do not change.

The write API is the InfluxDB 2.x/3.x `/api/v2/write` endpoint, `precision=ms`, with
`Authorization: Token <token>`. The point is one measurement `uptime_monitor` carrying the
event as a tag, so one query covers every target and every event type. No crate is added:
the transport reuses the HTTP client the webhook already wrote, and the line protocol is
rendered by a small function inside the new module. Telegram, the webhook, the log file, the
event database and every other behaviour are unchanged.

## Why

The tool records transitions in SQLite and pushes them to a chat, an automation endpoint and a
file. An operator who already runs InfluxDB and Grafana has to scrape the SQLite file or
receive the JSON document on a webhook and translate it themselves. The webhook is not a
substitute: InfluxDB does not accept the JSON document, and a Telegraf or a script between the
two is a moving part on the NAS that the single-binary deployment exists to avoid.

The cost of leaving it unfixed is a third-party bridge that must be deployed, secured and kept
in step with the JSON schema, or no time-series view of outages at all. The channel model of
[ADR-0033](../docs/ADR.md#adr-0033--deliver-notifications-through-declared-channels-with-a-typed-message)
was built for exactly this extension: one typed `Message`, one transport per channel, and the
monitor depending only on the `Notifier` contract.

Plan `015` is open and reserves `FR-36` and `ADR-0041`; this plan therefore uses `FR-37` and
`ADR-0042`. The two plans touch different parts of `src/notify/message.rs` and different
sections of the shared documents, so neither is a dependency of the other.

## Goals

- A `[[notification]]` entry with `type = "influxdb"` resolves, starts and writes one point per
  event it receives, and `notify` / `[[targets]].notify` enable it by name exactly as they
  enable the three existing types.
- The write is one `POST {prefix}/api/v2/write?org=…&bucket=…&precision=ms` per event, with
  `Content-Type: text/plain; charset=utf-8` and `Authorization: Token <token>`; a `2xx` answer
  is success, `429` and `5xx` are retried under the existing five-attempt rule, and every other
  answer is not retried.
- Each point is the measurement `uptime_monitor` with the tags `event`, `target`, `label`,
  `state` and `cause` as they apply, the field `count=1i`, and the event instant in milliseconds,
  so `sum("count")` grouped by `event` and `target` answers the usual questions; a tag value
  containing a space, a comma, an equals sign or a line break is escaped or flattened and can
  never split a point.
- A missing `org`, `bucket` or `token`, a `url` carrying a query or a fragment, a token carrying
  whitespace or a control character, and a key that belongs to another channel type are refused
  by `Config::resolve` with a message naming `notification[<i>].<key>`, before the first probe.
- `--show-config` prints the channel with `token = "<redacted>"` and re-parses with only the
  secret redacted and every other key identical; no secret and no query value reaches a log
  line, an error message or a `Debug` rendering.
- `check` verifies the channel with one read-only `GET …/health` and names it in a `fail` line
  when it cannot be reached.
- The three existing channel types, the routing, the policy, the retry, the database and the
  page behave exactly as they do today; a token on a `webhook` `http` URL stays refused.

## Non-goals

- InfluxDB 1.x (`/write`, `db`, `rp`, `u`, `p`) and an `api` key selecting a version. One write
  API per channel keeps the configuration and the tests small; 1.x reached end of life and
  1.8 emulation is not a requirement.
- Per-probe metrics, a batching worker, a flush interval, gzip, and a backfill of the recorded
  history. The channel sends the same events the other channels send, at the moment they are
  reported; the database remains the history.
- Configuration of the measurement name, the tag names, the field name, the timestamp precision
  or any other part of the point schema. A dashboard that needs another shape reads the SQLite
  file or uses a webhook.
- A new crate, a proxy setting, a way to disable certificate verification, or a redirect
  follower.
- InfluxDB 3 flavours that require a different write path or a SQL write endpoint.

## Affected modules

**Source**
- `src/notify/http.rs` *(new)* — the shared minimal HTTP client extracted from the webhook:
  `TIMEOUT`, `MAX_RESPONSE`, `connect`, `exchange`, `classify`, one `SendError` policy.
- `src/notify/influx.rs` *(new)* — `Influx` (the `Channel` implementation), `point`,
  the tag escaper, the request builder, the answer classification and their tests.
- `src/notify/webhook.rs` — delegate `exchange` and `open_connection` to `notify::http`; its
  request builder, `classify` wrapper, scrubbing and tests stay as they are.
- `src/notify/mod.rs` — `pub mod http;`, `pub mod influx;`, the `build` arm for
  `ChannelSettings::Influx`.
- `src/notify/message.rs` — `Kind::Influx` and its `as_str` (`"influxdb"`); no other change.
- `src/config.rs` — `FileNotification.org`/`bucket`, the `Influx` settings type with its
  `endpoint`/`health` helpers and a redacting `Debug`, the `ChannelSettings` variant and its
  `name`/`kind`/`stabilize`/`destination` arms, the `resolve_channel` arm, `reject_foreign`,
  `to_toml`, a query-value encoder, and the unit tests.
- `src/check.rs` — no production change; one test that a failing `influxdb` channel is named in
  a `fail` line.

**Tests**
- `src/notify/influx.rs` — point rendering for every event, escaping and line-break flattening,
  the request line and headers, the classification table, the `Debug` redaction, token scrubbing.
- `src/notify/http.rs` — the status-line parser and the shared classification policy.
- `src/config.rs` — resolution, every refusal, destination uniqueness (same bucket refused,
  different buckets accepted), `--show-config` redaction and round-trip, query encoding.
- `tests/notify_channels.rs` — a channel against a `127.0.0.1:0` listener: the point body, the
  `Authorization` header and the write path.
- `tests/cli.rs` — `--show-config` on a file with an `influxdb` channel prints `<redacted>` and
  exits zero.

**Documentation / configuration**
- `docs/PRODUCT.md` — the scope bullet, the `type` list of `FR-25`, the new **FR-37**, the
  business rule that names the channel types.
- `docs/GLOSSARY.md` — **Channel** and **Output destination** gain the new kind; new
  **InfluxDB channel**, **InfluxDB token**, **InfluxDB point**.
- `docs/ARCHITECTURE.md` — egress invariant, the C4 context diagram and the external-systems
  table, the `notify` component contract and the transport table, the secret handling, the
  `check` description, `INV-14`.
- `docs/ADR.md` — new **ADR-0042** and its index row.
- `docs/USER-GUIDE.md` — **Notifications** (common key table, new `#### InfluxDB` subsection),
  **What is sent, and when**, **Limits and deliberate choices**.
- `docs/NOTIFICATIONS.md` — the type table, a new runbook section with the `influx` CLI and
  `curl` checks, the failure-mode table.
- `docs/DEVELOPER-GUIDE.md` — one line in *Adding a notification channel*: an HTTP transport
  reuses `notify::http` and never opens its own socket.
- `README.md` — the one-sentence list of channel types.
- `uptime-monitor.sample.toml` — a commented `influxdb` example next to the other declarations.
- `site/index.html` — the feature sentence that lists the channel types.

## Architecture impact

`notify` gains a fifth module and a sibling for the shared HTTP client; the contract that the
monitor depends on — `Notifier`, `Message`, `Channel`, `Routing`, `start` — does not change, and
neither does the per-channel queue, policy or retry. The configuration grows one channel variant
and three keys, and the destination of a channel remains its real sink, so `INV-14` still holds:
the canonical write URL (scheme, host, resolved port, path, `org`, `bucket`) identifies the
channel, and two declarations cannot write to the same bucket.

The extraction of `notify::http` is a structural change to the webhook with no behaviour change:
the bytes on the wire, the timeouts, the 8 KiB answer bound and the classification stay
identical, which the existing webhook tests prove by not being modified.

Egress grows one destination per enabled channel, exactly as the webhook did. This changes the
`ARCHITECTURE.md` egress invariant, the external-systems table and the `notify` component row,
which the plan updates in the same change.

## Decision-record impact

A new decision record is required, **ADR-0042**, because the change adds a transport, a
configuration surface and one deliberate deviation: a token is accepted on a plain `http` URL,
where the webhook refuses it. The record states the context (a NAS and container deployment
where InfluxDB usually has no TLS, and a token scoped to one bucket), the decision, the
consequences, the residual risk and the alternatives (v1 API, both APIs behind an `api` key,
InfluxDB as a history exporter, a webhook plus Telegraf, a new crate, a batching worker).

`ADR-0033` is amended with one line: it lists three transports, and `FR-25`/`FR-37` now list
four.

## Proposed solution

### Step 1 — Extract the shared HTTP client into `src/notify/http.rs`

Move `TIMEOUT`, the `MAX_RESPONSE` bound, `connect_plain`, `exchange_over`, `split_response` and
the retryable/non-retryable classification out of `src/notify/webhook.rs` into the new
`src/notify/http.rs`, and publish:

- `pub fn connect(url: &UrlParts) -> Result<(), SendError>` — the TLS handshake included, then
  closed again;
- `pub fn exchange(url: &UrlParts, request: &str) -> Result<(u16, String), SendError>` — one
  request, the status and the bounded body;
- `pub fn classify(status: u16, channel: &str, host: &str) -> Result<(), SendError>` — `2xx` is
  success, `429` and `5xx` are retryable, everything else is not.

`webhook.rs` keeps `request()`, keeps a two-line `classify` wrapper that calls
`http::classify(status, "webhook", host)`, and delegates `exchange` and `open_connection`. Add
`pub mod http;` to `src/notify/mod.rs`. No behaviour, header, timeout or message changes.

**Verify:** `cargo test --lib notify::webhook` and `cargo test --test notify_channels` pass with
the existing webhook tests unmodified (`git diff src/notify/webhook.rs` shows only deletions of
the moved functions and the delegations).

### Step 2 — Add the configuration surface and the `Kind`

In `src/config.rs`: add `org` and `bucket` to `FileNotification`; add the `Influx` settings type;
add the `ChannelSettings::Influx` variant and its four arms; add the `"influxdb"` arm of
`resolve_channel`; add `org` and `bucket` to the list `reject_foreign` compares against; add the
two fields and the arm to `to_toml`. In `src/notify/message.rs`: add `Kind::Influx` with
`as_str() == "influxdb"`.

```rust
// src/config.rs — the settings the transport and --show-config share
pub struct Influx {
    pub name: String,
    /// Scheme, host, port and an optional path prefix; never a query.
    pub base: UrlParts,
    pub org: String,
    pub bucket: String,
    pub token: String,
    pub stabilize: Duration,
}
```

The destination is the canonical write URL, so `INV-14` needs no new variant:

```rust
ChannelSettings::Influx(settings) => Destination::Url(canonical_url(&settings.endpoint())),
```

Validation, each refused with `notification[<i>].<key>`:

- `url` — required; `parse_url` must accept it, and its path must contain no `?` (`a query is not
  part of the base URL`); a fragment and credentials are already refused by `parse_url`.
- `org`, `bucket` — required and non-empty after trimming; percent-encoded into the query, so any
  character is accepted.
- `token` — required and non-empty; `whitespace or control characters are not allowed in a header`
  is refused, because the token travels in `Authorization`.
- a key of another type (`bot_token`, `chat_id`, `filename`, `logger`, `level`, `format`,
  `max_bytes`, `max_files`, and `org`/`bucket` on the other types) is refused.

The declaration, added to the commented examples of the sample file:

```toml
# uptime-monitor.sample.toml — the block is commented out, like its siblings
# [[notification]]
# name = "influx"
# type = "influxdb"
# url = "https://influx.example.test:8086"  # base URL; a path prefix is allowed, a query is not
# org = "home"
# bucket = "uptime"
# token = "replace-me"                      # read from the file only; <redacted> in --show-config
# stabilize_seconds = 60
```

**Verify:** `cargo test --lib config` (the new tests and the updated
`must be telegram, webhook, logfile or influxdb` assertion), and `uptime-monitor --show-config`
on a temporary file holding the same block uncommented prints `token = "<redacted>"`.

### Step 3 — Render the point and build the two URLs

In `src/notify/influx.rs`, add the pure functions the transport uses, plus the `Influx` settings
helpers in `src/config.rs`:

- `Influx::endpoint()` → `base` with `target = "{prefix}/api/v2/write?org={enc}&bucket={enc}&precision=ms"`;
- `Influx::health()` → `base` with `target = "{prefix}/health"`;
- `fn point(message: &Message) -> String` → the line protocol for one event;
- `fn escape_tag(value: &str) -> String` → `,`, ` ` and `=` escaped with a backslash, `\n` and
  `\r` flattened to a space so one point is one line;
- `fn encode_query(value: &str) -> String` in `src/config.rs` → RFC 3986 unreserved characters
  kept, every other UTF-8 byte as `%XX`.

`{prefix}` is `base.target` with one trailing `/` removed, so `https://host:8086` gives `""` and
`https://host/influx/` gives `"/influx"`. Building the two URLs on the settings type keeps the
knowledge of the path in one place: the transport, the destination and `check` all call it, and
none of them formats a query.

The shape, fixed and documented:

```text
uptime_monitor,event=target_down,target=icmp://1.1.1.1,label=cloudflare-dns,state=down,cause=ICMP.NO_REPLY count=1i 1764000000000
uptime_monitor,event=startup count=1i 1764000000000
```

`event` is always present; `target` and `label` only on a target event, `label` only when the
target has one; `state` only on a target event; `cause` only on a `down`. The field is always
`count=1i`, and the timestamp is `occurred_at_ms` at `precision=ms`.

**Verify:** `cargo test --lib notify::influx` covers one point per variant of `Event`, a label
with `,`, `=` and a space, a label with a newline, a target-only and a global event, and
`encode_query("a b/c") == "a%20b%2Fc"`.

### Step 4 — Implement the transport

`src/notify/influx.rs` implements `Channel`:

- `name`, `kind` (`Kind::Influx`);
- `open` — `Ok(())`, like the webhook: a start never depends on the network;
- `verify` — one `GET {prefix}/health` through `notify::http::exchange`; `2xx` is success, any
  answer is `"influxdb {host} answered {status}"`;
- `payload` — `Some` for every message, body `point(&message)`, media type
  `text/plain; charset=utf-8`;
- `send` — the request below, `exchange`, then `classify(status, "influxdb", &host)`;
- `close` — nothing.

```text
POST /api/v2/write?org=home&bucket=uptime&precision=ms HTTP/1.1
Host: influx.example.test:8086
Authorization: Token <token>
Content-Type: text/plain; charset=utf-8
Content-Length: <len>
Connection: close

<one point>
```

The token is scrubbed from every `SendError` before it is returned, `Debug` renders
`<redacted>`, and every message names `scheme://host[:port]` only. Add the `build` arm to
`src/notify/mod.rs`.

**Verify:** `cargo test --lib notify::influx` asserts the exact request for a known settings
value, the `Authorization: Token` header, the classification table (`204`/`200` ok, `301` and
`400`/`401`/`403`/`404` not retryable, `429` and `500`/`503` retryable, a transport failure
retryable), and that neither `Debug` nor a scrubbed error contains the token.

### Step 5 — Prove the transport against a local listener

Add a test to `tests/notify_channels.rs` in the shape of
`a_webhook_receives_the_json_document_over_http`: bind `127.0.0.1:0`, declare
`ChannelSettings::Influx` over `http://<address>`, start the channel, send `down(KEY)`, read
the two requests (the channel's own `CHANNEL` message and the transition), and assert that the
transition request starts with `POST /api/v2/write?org=home&bucket=uptime&precision=ms HTTP/1.1`,
carries `Authorization: Token <token>` and `Content-Type: text/plain; charset=utf-8`, and whose
last line is
`uptime_monitor,event=target_down,target=icmp://1.1.1.1,label=cloudflare-dns,state=down,cause=ICMP.NO_REPLY count=1i 1764000000000`.

The test also pins the accepted plaintext behaviour: a token on `http://127.0.0.1:<port>` is
accepted for this type. A second assertion keeps the existing rule intact for the webhook — a
token on an `http` URL is still refused by `Config::resolve`.

**Verify:** `cargo test --test notify_channels`.

### Step 6 — Cover `check` and `--show-config`

Add to `src/check.rs`'s tests one case where the declared channel is an `influxdb` channel whose
`url` is `http://127.0.0.1:1` (a refused connection, deterministic, no external network) and
assert, through the existing `find(&report, "channel influx")` helper, that the line is a `fail`
whose reason names the host. Add to `tests/cli.rs` a case that writes a configuration with an
`influxdb` channel, runs `--show-config`, and asserts the output contains `type = "influxdb"`,
`bucket = "uptime"` and `token = "<redacted>"` and no occurrence of the secret.

**Verify:** `cargo test --lib check` and `cargo test --test cli`.

### Step 7 — Document the requirement and the terms

`docs/PRODUCT.md`: add the channel to the scope bullet (line 34), extend the `type` list and the
routing paragraph of `FR-25`, add **`FR-37`** with the keys, the endpoint, the point shape, the
retry classification, the refusals and the `check` behaviour, and change the business rule that
reads "the notification channels are the three of `FR-25`" to name the four.
`docs/GLOSSARY.md`: extend **Channel** and **Output destination**, and add **InfluxDB channel**,
**InfluxDB token** and **InfluxDB point**, one definition each, without repeating `FR-37`.

**Verify:** `rg -n "FR-37|InfluxDB (channel|token|point)" docs/PRODUCT.md docs/GLOSSARY.md`
finds each new item once, and every number, key and default matches the code of steps 2–4.

### Step 8 — Record the architecture and the decision

`docs/ARCHITECTURE.md`: the egress invariant (line 29) gains the InfluxDB write; the C4 context
gains one `System_Ext(influx, "InfluxDB", …)` and one `Rel`; the external-systems table gains a
row (request: one write per event; answer: the HTTP status); the `notify` component table gains
`notify::influx` and `notify::http`; the guarantees paragraph states the point schema, the
token rule and the `verify` behaviour; the secret paragraph names the InfluxDB token; `INV-14`
lists the canonical write URL as a destination.

`docs/ADR.md`: add **ADR-0042 — Send events to InfluxDB through a channel using the v2 write
API** (context, decision, consequences, alternatives: v1 API, an `api` key, InfluxDB as a
history exporter, webhook plus Telegraf, a new crate, a batching worker) and its index row; add
the one-line amendment to ADR-0033.

**Verify:** `rg -n "ADR-0042" docs/ADR.md` shows the index row and the section, and the diagram
still parses as a Mermaid `flowchart`/`C4Context` block (visual check on GitHub or with
`npx @mermaid-js/mermaid-cli` if available).

### Step 9 — Update the user guide

`docs/USER-GUIDE.md` is self-contained and must let an operator configure the channel without
the repository: the common table's `type` row gains `influxdb`; a `#### InfluxDB` subsection
gives the keys with type, default and effect, an example, the point schema, the retry table and
the plaintext warning; **What is sent, and when** gains one sentence that each event is one
point; **Limits and deliberate choices** states that only the v2 write API is supported, that
there is no batching and no per-probe metric, and that an `http` URL sends the token in clear.

| Key | Default | Meaning |
|---|---|---|
| `url` | — | Required. Base URL of the server, `http://` or `https://`; an optional path prefix; no query |
| `org` | — | Required. The organization of the write |
| `bucket` | — | Required. The bucket the points are written to |
| `token` | — | Required. The API token, with write permission on the bucket; `<redacted>` in `--show-config` |

**Verify:** every row of the new table matches the field, the default or the refusal implemented
in `src/config.rs`, and the example parses: `uptime-monitor check` on it prints one `ok` line for
the channels when the server is reachable, and a `fail` line whose subject is `channel influx`
otherwise.

### Step 10 — Update the runbook and the remaining documents

`docs/NOTIFICATIONS.md`: the "Three channel types exist" sentence becomes four, the key table
gains the row, a new section after the log file explains how to create the bucket and a
write-only token (`influx auth create --org home --write-bucket <id>`), declares the channel,
proves it with `curl {url}/health` and a manual `curl -XPOST '{url}/api/v2/write?org=home&bucket=uptime&precision=ms' --data-binary 'uptime_monitor,event=test count=1i'`,
and states what a `204` means; the failure-mode table gains the InfluxDB answers (`204`
delivered, `400` bad line or unknown bucket, `401`/`403` token, `404` wrong path, `413` body,
`429`/`5xx` retried, transport failure retried) and the note that `/health` is unauthenticated,
so a wrong token surfaces on the first event, not at `check`.

`docs/DEVELOPER-GUIDE.md`: one sentence in *Adding a notification channel* — an HTTP transport
uses `notify::http` and never opens its own socket. `README.md`: the sentence that lists the
channel types. `uptime-monitor.sample.toml`: the commented block of step 2.
`site/index.html`: the feature sentence that lists the types, and the FAQ answer that names
them if it does.

**Verify:** `rg -n "influxdb" README.md docs/NOTIFICATIONS.md site/index.html uptime-monitor.sample.toml`
and `rg -n "notify::http" docs/DEVELOPER-GUIDE.md` find each update, `rg -n "Three channel types" docs/`
finds nothing, and the `curl` examples use the same path and precision as `Influx::endpoint()`.

### Step 11 — Run the quality gate and review the diff

Run the whole gate, then read `git diff` once end to end: no secret, no new crate, no change to
`Cargo.toml`, no unrelated file.

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.

## Security considerations

This plan is not a `security` plan, but it handles a secret and a new egress path.

- **Exposure.** The token is a credential with write access to one bucket. It travels in the
  `Authorization` header of every write. The plan accepts an `http` URL, where the header — and
  therefore the token — is readable on the path, where the webhook refuses the combination.
  This is a deliberate, documented deviation: the primary deployment is a container on a NAS
  LAN where InfluxDB commonly has no TLS, and refusing `http` would make the channel unusable
  there. It is recorded in ADR-0042 and in `USER-GUIDE.md` and `NOTIFICATIONS.md`.
- **Mitigation.** The documentation recommends a token scoped to the bucket with write
  permission only, and `https` whenever the path is shared. The token is read from the file
  only, printed `<redacted>` by `--show-config`, replaced by `<redacted>` in every `Debug` and
  in every `SendError`, and a URL is printed as `scheme://host[:port]`, never with the path.
  `org` and `bucket` are percent-encoded, so a value can neither break the query nor reach the
  request line unescaped; the token is refused when it carries whitespace or a control
  character, so it can neither split nor inject a header.
- **Residual risk.** An operator who writes `http://` to a remote host exposes the token to the
  network. Accepted by the operator at declaration time; the runbook states it in one line.
- **No new dependency, no new parser of untrusted input.** The answer is bounded at 8 KiB and
  only its status is used, exactly as for the webhook.

## Verification plan

- **Automated.** The unit tests of steps 2–4 (resolution, refusals, destination uniqueness,
  `--show-config` redaction, point rendering and escaping, request bytes, classification,
  redaction), the `notify_channels` integration test, the `check` and `cli` cases, and the
  unchanged webhook and log-file tests.
- **Quality gate.** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  (from `.spec-workflow.yml`).
- **Manual.** With an InfluxDB 2.7 container
  (`docker run -d --name influx -p 8086:8086 influxdb:2.7`, then `influx setup`), declare the
  channel against `http://127.0.0.1:8086`, run `uptime-monitor check` (one `ok` line for the
  channels), start the monitor, stop a target, and confirm in the InfluxDB UI that
  `from(bucket: "uptime") |> range(start: -1h)` shows `uptime_monitor` with
  `event=target_down`.
- **Rollback.** Revert the commit. There is no schema change, no migration and no state on
  disk: the previous binary runs the previous configuration unchanged, provided the
  `influxdb` declaration is removed at the same time, because the old `resolve_channel` refuses
  the unknown type.

## Open questions

- [ ] Confirm the plaintext decision: a token is accepted on an `http` URL for `influxdb`, where
      `webhook` refuses it. Current best guess: accept, document, record in ADR-0042; the
      alternative is the webhook rule, which would require TLS inside the LAN. Owner: operator.
- [ ] Confirm `FR-37` and `ADR-0042` as the numbers, since plan `015` reserves `FR-36` and
      `ADR-0041`. Current best guess: `FR-37`/`ADR-0042`, no dependency between the plans.
- [ ] Confirm `count=1i` as the field name (a `sum("count")` reads naturally in Flux) rather than
      `value=1i`. Current best guess: `count`.
- [ ] Confirm `/health` as what `check` verifies, accepting that a wrong token is detected on the
      first event rather than at `check`, because `/health` is unauthenticated and an
      authenticated read would fail a write-only token. Current best guess: `/health`.

## Implementation checklist

- [ ] Step 1 — Extract the shared HTTP client into `src/notify/http.rs`
- [ ] Step 2 — Add the configuration surface and the `Kind`
- [ ] Step 3 — Render the point and build the two URLs
- [ ] Step 4 — Implement the transport
- [ ] Step 5 — Prove the transport against a local listener
- [ ] Step 6 — Cover `check` and `--show-config`
- [ ] Step 7 — Document the requirement and the terms
- [ ] Step 8 — Record the architecture and the decision
- [ ] Step 9 — Update the user guide
- [ ] Step 10 — Update the runbook and the remaining documents
- [ ] Step 11 — Run the quality gate and review the diff
- [ ] All open questions resolved
- [ ] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [ ] Documentation listed under Affected modules is updated
- [ ] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration option, or the plan records why it needs no change
- [ ] `## Implementation Notes` added or updated in this plan

