---
title: Give every failure a code and a standard message, record faults, and rename the row vocabulary
date: 2026-11-14
status: done
type: feature
impact: high
area: probe, store, web, query, vocabulary
version_bump: minor
tags: [cause, code, template, verdict, schema, csv, table, ui, rename, terminology]
depends_on: []
branch_recommended: true
---

## Summary

A failure is recorded today as a bare fact: the target stopped answering, with no statement of
*why*, and a local probe failure is a free-form `String` built with `format!` at thirty-one call
sites. This plan gives every failure two things: a **short code** that names it (`TCP.REFUSED`,
`DNS.NXDOMAIN`, `HTTP.UNEXPECTED_STATUS`) and **one standard message** declared as a template in
the code, with the details of that occurrence inserted when it is rendered. It also retires the
word *disconnection*: the rows become **faults**, because a row is no longer only a lost
connection but any recorded failure, and the standard message of a fault is revealed **under its
own row on click** instead of hiding in a hover tooltip. The cause of a fault is stored with its
event and shown in the table of both pages, in `query events`, and in the exported CSV. Rows
written before this change keep working with no cause.

## Why

- The information exists and is thrown away: `tcp::outcome` distinguishes a refusal from a reset
  and from a timeout ([src/probe/tcp.rs](../../src/probe/tcp.rs)), `dns::parse_response` reads the
  RCODE ([src/probe/dns.rs](../../src/probe/dns.rs)), `http::exchange` returns the status
  ([src/probe/http.rs](../../src/probe/http.rs)). All of it collapses into `Verdict::Down`, a unit
  variant, and into `ProbeError::Local { reason: String }`, whose wording is decided ad hoc at
  each call site.
- A user reading "down for 12 minutes" cannot tell a filtered packet from a closed port, and those
  two lead to opposite remediations.
- A free-form message cannot be grepped, counted, or filtered: two sites describe the same failure
  with different words, and a log line has no stable token to search for.
- The vocabulary is now wrong, not merely narrow. "Disconnection" names one cause out of the
  eight this plan introduces, it reads badly next to a `DNS.NXDOMAIN`, and it will read worse as
  soon as the table carries a row that is not a lost connection. The glossary's `Down episode` is
  precise but equally tied to one shape of failure.
- The message is worth more than a tooltip: it is the sentence that explains the row, and a hover
  is invisible on a trackpad, absent on touch, and lost the moment the pointer moves.

## Goals

- Every failure is a value `Failure { code, message }`: the **code** is a `&'static str` from a
  closed set, and the **message** is rendered from a `&'static str` template and a `Details`
  record of that occurrence. No call site formats a message by hand.
- The set of codes is exactly the table in step 1, and a unit test fails when a code is added
  without a template, or a template needs a detail its code does not supply.
- `Verdict::Down` carries the `Failure` of the check, and `ProbeError` carries a `Failure` too, so
  the two families speak the same language.
- A recorded occurrence is a **`Fault`**: the store type, the queries, the table heading ("Last 10
  faults"), the `query` output, the export route and the exported file name all use that word, and
  the word *disconnection* no longer appears in the current documentation.
- The standard message of a fault is revealed **in a row of its own under the fault**, opened by a
  click on the fault row; a browser without JavaScript still shows the message through the
  `title` attribute, so no reader loses the information.
- The cause of a fault survives in the database, so it is still there after a restart and after an
  import.
- The CSV gains `cause` and `cause_message` as its **last** two fields, so an archive written
  before this change still imports and an archive written after it re-imports unchanged.
- A database created by version 0.2 opens in 0.3 and keeps every event, with an empty cause on the
  rows that predate the column.
- Tables are visibly separated from what sits above them: a rule where the table begins, more
  space above it, and a header row that stays in place while a long table scrolls.

## Non-goals

- No cause for a local failure is persisted: `ProbeError` still writes nothing (INV-5). It gains a
  code and a template so the log is parseable, not so the history grows.
- No per-event probe history: only transitions are stored, as before.
- No change to how a state is decided: the cause describes an existing verdict, it never turns a
  `down` into an `up` or the reverse.
- No backfill and no re-rendering: a message is stored as it was rendered, so a row written under a
  2 s timeout keeps saying 2 s after the configuration changes. History states what was true then.
- No rewrite of the decision records written before this plan: ADR-0025 records the vocabulary
  change, and the older records keep the words they were written with. Only the index row and the
  current documents adopt `Fault`.
- No codes for `ConfigError`, `StoreError`, `ImportError` or `ServeError`: this plan covers the
  failures of a probe cycle.
- No translation layer: codes and messages are English, like every artefact in the repository.

## Affected modules

**Source**
- `src/probe/failure.rs` — *(new)* `Code`, `Code::code()`, `Code::template()`, `Details`,
  `render`, `Failure`; the only place a message is worded.
- `src/probe/mod.rs` — re-export the new module; `Verdict::Down(Failure)`; `ProbeError` carries a
  `Failure` instead of a bare `reason`.
- `src/probe/icmp.rs` — `ICMP.NO_REPLY` and the socket/io codes of that transport.
- `src/probe/tcp.rs` — `outcome` states `TCP.REFUSED`, `TCP.RESET`, `TCP.TIMEOUT`; `TCP.SOCKET`,
  `TCP.RESOLVE`, `TCP.INTERFACE` for the local failures.
- `src/probe/dns.rs` — `DNS.NXDOMAIN`, `DNS.NO_RECORD`, `DNS.TIMEOUT`, `DNS.UNRESOLVED`, and
  `DNS.BUILD`, `DNS.SOCKET`, `DNS.UNUSABLE`.
- `src/probe/http.rs` — `HTTP.UNEXPECTED_STATUS`, `HTTP.TLS`, and `HTTP.RESOLVE`, `HTTP.TLS_INIT`,
  `HTTP.MALFORMED`, `HTTP.HEAD_LIMIT`; the handshake errors are classified where the handshake
  runs, so a certificate problem is `HTTP.TLS` and not an anonymous `down`.
- `src/probe/fake.rs` — `down()`, `down_with(code)` helpers, so a test scripts a cause.
- `src/monitor/state.rs` — `Transition` gains `cause: Option<Failure>`; `transition` copies it.
- `src/monitor/mod.rs` — the recorded `Event` carries the cause; the log line of a local failure
  states its code.
- `src/store/schema.rs` — `SCHEMA_VERSION = 2`; `events.cause TEXT`, `events.cause_message TEXT`.
- `src/store/mod.rs` — `DownEpisode` renamed to `Fault`, `recent_downs`/`recent_downs_for` to
  `recent_faults`/`recent_faults_for`, `DOWNS_SELECT` to `FAULTS_SELECT`; `Fault` gains `cause`
  and `cause_message`; the 1 → 2 migration; `Event` gains both fields, in both insert paths.
- `src/import.rs` — `Row::parse` reads the two optional trailing fields.
- `src/web/export.rs` — the header and the row gain `cause` and `cause_message`.
- `src/web/mod.rs` — `RECENT_DOWNS` renamed to `RECENT_FAULTS`; the export route becomes
  `/export/faults.csv.gz` and `export_name` writes `faults-YYYYMMDD.csv.gz`; the previous route
  keeps answering with a permanent redirect.
- `src/web/page.rs` — `downs_table` renamed to `faults_table`, gaining the cause column, the fault
  row as a control, the message row beneath it, and the small script that toggles it; the table and
  header rules in `STYLE`.
- `src/web/detail.rs` — uses `faults_table` and includes the same script.
- `src/query/render.rs` — the cause in the `events` table and in the JSON.
- `Cargo.toml` — version `0.3.0`.

**Tests**
- `src/probe/failure.rs` (unit) — every code has a non-empty code and template; rendering leaves no
  `{placeholder}` behind; the code set is the one the table names.
- `src/probe/{icmp,tcp,dns,http}.rs` (unit) — each classification states its code, and each local
  failure its own.
- `src/monitor/state.rs`, `tests/monitor_e2e.rs` — the cause reaches the recorded event.
- `src/store/mod.rs` (unit) — a version 1 database with events migrates to 2, keeps them, and its
  rows read no cause.
- `tests/store.rs` — a fault round-trips its code and message; an old row reads `None`.
- `src/web/export.rs`, `tests/import.rs` — the two new columns; a six-column archive still imports;
  the file name and the redirect of the old route.
- `src/web/page.rs`, `src/web/detail.rs`, `tests/web.rs` — the column, the message row, the control
  attributes, the empty cell, the heading, the table separation.
- `src/query/render.rs` — the CLI column and the JSON keys.

**Documentation / configuration**
- `docs/PRODUCT.md` — `FR-17` (the heading, the cause column, the click that reveals the message,
  and the end of "no interaction beyond following a link"), `FR-18` (the CSV columns and the new
  export path), `FR-19` (an archive without them still imports), `FR-20`–`FR-23` (the code each
  method states).
- `docs/ARCHITECTURE.md` — the `probe` failure contract, the `web` output and export route, the
  `store` data model (`Fault`), the schema version.
- `docs/GLOSSARY.md` — `Fault` replaces `Down episode`; new `Cause`, `Cause code`, `Cause message`;
  `Verdict` reworded to refer to them.
- `docs/ADR.md` — ADR-0025 (a code and a template per failure, `Fault` as the row term, two new CSV
  columns, a redirect for the old export path) plus its index row.
- `docs/USER-GUIDE.md` — "The web page": the faults table, the click that reveals the message, the
  codes an operator will meet, the CSV columns, the new export path; "Operations": the 0.2 → 0.3
  upgrade note.
- `docs/DEVELOPER-GUIDE.md` — "Adding a probe type" gains "declare the codes in `probe::failure`
  and word the message once"; the import examples use `faults-*.csv.gz` and the new route.
- `README.md` — the quick start's import example uses the new file name.
- `network-monitor.toml` — unchanged (no new configuration key).

## Architecture impact

`probe` keeps owning the transports and now also owns the vocabulary of their failures: the code is
chosen where the answer is classified, so no other module learns a protocol detail. `store` owns
the two columns, the migration, and the `Fault` type; the web and query layers only render what the
store returns. `INV-3` is unaffected — the failure contract is part of `Probe`, and the fake
implements it. `INV-5` is unaffected — a `ProbeError` carries a cause and is still never persisted.

Two ripples to plan for rather than discover:

- `Verdict` is `Copy` today only because `Down` has no payload. With a formatted message inside,
  `Verdict`, `Transition` and the fake's script stop being `Copy` and are cloned or borrowed; the
  thirty-three `Verdict::Down` construction and assertion sites are updated in the same step.
- The exported route and file name are a public contract. The vocabulary change renames both, so
  the old path answers `308 Permanent Redirect` to the new one and the guide's upgrade section
  states it; nothing that worked before stops working.

The page gains its third script and its first control: a fault row becomes a button-like element
with `aria-expanded` and `aria-controls`, and the message lives in a real row beneath it, so the
table stays a table for a screen reader. Without JavaScript the control is inert and the `title`
carries the message, which is what the page does today.

## Decision-record impact

ADR-0025, because the change touches four contracts at once: the language failures are reported in,
the row vocabulary of the product, the schema version (a database written by 0.3 is refused by 0.2
rather than misread), and the shape and address of the exported CSV.

## Proposed solution

1. **The failure vocabulary.** Create `src/probe/failure.rs`:

   - `pub struct Details<'a>` with the optional values a template may insert: `host`, `port`,
     `name`, `server`, `address`, `url`, `interface`, `status`, `expect`, `timeout`, `method`,
     `other`, `reason`.
   - `pub enum Code` with one variant per row of the table below, plus `code(self) -> &'static str`
     and `template(self) -> &'static str`.
   - `fn render(template: &str, details: &Details) -> String`, replacing `{name}` with the value and
     rendering a placeholder the code did not supply as `unknown`, so a message never shows a raw
     brace.
   - `pub struct Failure { code: &'static str, message: String }` with
     `Failure::new(code, &Details)` and `Display` as `CODE — message`, so a log line carries both.

   The table is the contract; every code appears exactly once:

   | Code | Template | Details supplied |
   |---|---|---|
   | `ICMP.NO_REPLY` | `no echo reply from {address} within {timeout}` | address, timeout |
   | `ICMP.SOCKET` | `cannot open a raw ICMP socket: {reason}` | reason |
   | `ICMP.IO` | `cannot send or receive on the ICMP socket: {reason}` | reason |
   | `TCP.REFUSED` | `the connection to {host}:{port} was refused` | host, port |
   | `TCP.RESET` | `the connection to {host}:{port} was reset` | host, port |
   | `TCP.TIMEOUT` | `no answer from {host}:{port} within {timeout}` | host, port, timeout |
   | `TCP.SOCKET` | `cannot create the socket for {host}:{port}: {reason}` | host, port, reason |
   | `TCP.RESOLVE` | `cannot resolve {host}: {reason}` | host, reason |
   | `TCP.INTERFACE` | `cannot bind the probe to {interface}: {reason}` | interface, reason |
   | `DNS.NXDOMAIN` | `{name} does not exist, as answered by {server}` | name, server |
   | `DNS.NO_RECORD` | `{name} has no record of the requested type, as answered by {server}` | name, server |
   | `DNS.TIMEOUT` | `{server} did not answer the question for {name} within {timeout}` | server, name, timeout |
   | `DNS.UNRESOLVED` | `{name} does not resolve through the system resolver` | name |
   | `DNS.BUILD` | `{name} cannot be written as a DNS question: {reason}` | name, reason |
   | `DNS.SOCKET` | `cannot open a UDP socket to {server}: {reason}` | server, reason |
   | `DNS.UNUSABLE` | `{server} answered something that is not a usable answer: {reason}` | server, reason |
   | `HTTP.UNEXPECTED_STATUS` | `{url} answered {status}, outside {expect}` | url, status, expect |
   | `HTTP.TLS` | `the TLS handshake with {host}:{port} failed: {reason}` | host, port, reason |
   | `HTTP.RESOLVE` | `cannot resolve {host}: {reason}` | host, reason |
   | `HTTP.TLS_INIT` | `cannot build the TLS client: {reason}` | reason |
   | `HTTP.MALFORMED` | `{url} answered something that is not an HTTP answer: {reason}` | url, reason |
   | `HTTP.HEAD_LIMIT` | `{url} sent a head larger than the {expect} limit` | url, expect |
   | `PROBE.WRONG_METHOD` | `the {method} prober cannot check a {other} target` | method, other |

   The prefix names the **layer that reported the failure**, not the target's method: a `http`
   target whose port is closed states `TCP.REFUSED`, which is what happened.

   *Verify:* `cargo test --lib probe::failure` — one test per code for a non-empty code and
   template, one that renders every code with its required details and asserts the result holds no
   `{`, and one that fails when a variant is added without a row.

2. **Every prober states its cause.** `icmp::await_reply` → `ICMP.NO_REPLY`; `tcp::outcome` → the
   three connection codes; `dns` → the four from the RCODE and the system resolver; `http` →
   `HTTP.UNEXPECTED_STATUS` with the status and the expectation, `HTTP.TLS` from the handshake.
   `Verdict::Down(Failure)`, `fake::down()`/`down_with(code)`, and the `Copy` ripple handled.
   *Verify:* `cargo test --lib probe::` and `cargo test --test probe_network -- --ignored` — one
   assertion per classification, against public endpoints where a local server cannot produce the
   failure.

3. **Every local failure states its code.** The thirty-one `ProbeError::local(format!(…))` sites
   become `ProbeError::of(Code::…, &Details { … })`, and `ProbeError: Display` renders
   `CODE — message`, so `probe failed: TCP.RESOLVE — cannot resolve example.invalid: …` appears in
   the log with a token that can be grepped.
   *Verify:* `cargo test --lib probe::` — one test per module asserting the code of its local
   failures; `grep -rc "ProbeError::local" src/ | grep -v ":0"` returns nothing.

4. **Carry the cause to the event.** `monitor::state::Transition` gains `cause: Option<Failure>`,
   `transition` copies it from `Ok(Verdict::Down(failure))`, and `monitor::run` puts it in the
   `Event`.
   *Verify:* `cargo test --lib monitor::` and `cargo test --test monitor_e2e` — a scripted
   `down_with(Code::TcpRefused)` reaches the recorded event with its message.

5. **Store the cause, and rename the row.** `SCHEMA_VERSION = 2`; `events.cause TEXT` and
   `events.cause_message TEXT` in `schema::TABLES`; a migration arm for `Some(1)` running both
   `ALTER TABLE` statements and writing version 2. `Event` gains both fields in both insert paths.
   In the same step rename `DownEpisode` → `Fault` (with `cause` and `cause_message`),
   `recent_downs` → `recent_faults`, `recent_downs_for` → `recent_faults_for`, `DOWNS_SELECT` →
   `FAULTS_SELECT` and add the two columns to it.
   *Verify:* `cargo test --lib store::` and `cargo test --test store` — a version 1 database built
   by hand with an event opens, gains both columns, keeps the row with `NULL` cause and `NULL`
   message, is stamped 2; and a fault round-trips code and message.

6. **Export and import.** The header becomes
   `target,label,interface,state,occurred_at,rtt_us,cause,cause_message`; `row` appends both;
   `import::Row::parse` reads `fields.get(6)` and `fields.get(7)` and leaves them `None` when the
   archive predates them. The route becomes `/export/faults.csv.gz`, `export_name` writes
   `faults-YYYYMMDD.csv.gz`, and `/export/disconnections.csv.gz` answers `308` with
   `Location: /export/faults.csv.gz`.
   *Verify:* `cargo test --lib web::export` and `cargo test --test import` — an eight-column archive
   round-trips code and message, a six-column archive imports with no cause and no skipped row, the
   name carries the new word, and the old path redirects.

7. **Show the cause, and reveal the message on click.** `faults_table` in `src/web/page.rs` gains
   the column between the target key and `started`. The heading becomes "Last 10 faults". The code
   cell holds a `<button type="button" class="cause" aria-expanded="false" aria-controls="...">`
   whose `title` carries the message for a browser without JavaScript; directly under the row sits a
   hidden `<tr class="cause-row" id="..."><td colspan="5">` with the standard message, muted and set
   at the width of the row. A script (`CAUSE_SCRIPT`, included by both pages next to
   `LOCALIZE_SCRIPT`) toggles `hidden` and `aria-expanded` when the row or the button is clicked, and
   opens one fault at a time so the table does not jump. A fault without a cause leaves the cell
   empty and has no message row. `src/query/render.rs` adds the code to the `events` table and both
   fields to the JSON.
   *Verify:* `cargo test --lib web::page`, `cargo test --test web`, `cargo test --lib query::` — a
   fault with a cause, one without, the control attributes, the collapsed and the expanded markup,
   and the rendered CLI line; then the page in a browser: one click opens the message under its own
   row, a second closes it, and with JavaScript disabled the `title` still shows the message.

8. **Separate the tables.** In `STYLE`: a top rule on the header row
   (`thead th { border-top: 1px solid var(--rule) }`), room above the table, a header that stays in
   place while the table scrolls, painted with the page background so rows never show through it,
   and a message row that reads as part of its fault rather than as a new one. Keep `--rule`, the
   type scale and the muted header of the current theme.
   *Verify:* `cargo test --lib web::page` — the style assertions; then the page in a browser at
   320 px and at full width, with a table longer than the viewport and one fault expanded.

9. **Document.** PRODUCT (`FR-17` with the new heading, the column and the click, `FR-18` with the
   columns and the new path, `FR-19`, `FR-20`–`FR-23`), ARCHITECTURE (`probe` failure contract,
   `web` output and route, `store` `Fault` and schema version), GLOSSARY (`Fault`, `Cause`,
   `Cause code`, `Cause message`), ADR-0025 with its index row, USER-GUIDE (the faults table, the
   click, the codes an operator meets, the CSV columns, the new path, the 0.2 → 0.3 upgrade note),
   DEVELOPER-GUIDE ("Adding a probe type", the import examples), README (the import example),
   `Cargo.toml` version `0.3.0`.
   *Verify:* `grep -rn "disconnection" docs/ README.md` returns only the historical decision
   records; the guide lists the codes exactly as `probe::failure` declares them;
   `python3 ~/.copilot/skills/spec-workflow/scripts/spec_plan.py validate 002 --strict` exits 0.

## Security considerations

A message is assembled from two sources only: a template written in this repository, and values
this repository produced — a parsed `u16` status, a `Duration`, a target field, or the text the
platform returned for an `io::Error`. No byte from a remote peer reaches a template as free text:
the DNS `reason` strings come from the local codec's own vocabulary, and the HTTP `reason` from
`io::Error`/`rustls` messages. Every rendered cell still goes through `escape`, and every CSV field
through `field()`, which quotes commas and quotes, so an imported archive cannot inject a column.
The new script manipulates only `hidden` and an attribute of elements it found by `id`; it never
builds HTML from data, so a hostile cause cannot script the page. No new dependency, no new
`unsafe`, no new I/O.

## Verification plan

`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` (the profile's
`quality.all`), plus the manual run below:

```bash
./target/debug/network-monitor monitor --config ./network-monitor.toml --listen 127.0.0.1:8642
# in another shell, with a target whose port is closed and one whose host black-holes:
./target/debug/network-monitor query events --limit 5
curl -s localhost:8642/ | grep -o '<button[^>]*class="cause"[^>]*>[^<]*</button>' | head
curl -s localhost:8642/export/faults.csv.gz | gunzip | head -3
curl -s -o /dev/null -w '%{http_code} %{redirect_url}\n' localhost:8642/export/disconnections.csv.gz
```

Expected: the query line and the page state `TCP.REFUSED` with the message
`the connection to …:… was refused` for the closed port and `TCP.TIMEOUT` for the black-holed host;
a click on the row reveals that message underneath it; the CSV carries both in its last two
columns; the old export path answers `308` towards the new one; an archive downloaded before the
change imports with an empty cause.

## Open questions

- [x] The prefix names the failing layer (`TCP.REFUSED` for an `http` target whose port is closed)
      rather than the target's method. Best guess: keep — it states what happened, and it avoids a
      second copy of every socket and timeout code under `HTTP.`.
- [x] The old export path answers `308` rather than being removed with a breaking change. Best
      guess: keep the redirect for one release, and drop it in 0.4 with a `BREAKING CHANGE` note.
- [x] Clicking anywhere on the fault row toggles it, and the code cell is the keyboard-reachable
      control. Best guess: both, through one delegated handler, with `aria-expanded` on the button.
- [x] Only one fault is open at a time. Best guess: yes — a table where several messages are open
      loses the alignment that makes the columns readable.
- [x] A placeholder whose value the code did not supply renders as `unknown`. Best guess: yes, with
      a test per code, so a message can never show `{status}` to a reader.
- [x] The message is stored as it was rendered, not re-rendered from the current configuration. Best
      guess: yes — history states what the probe reported with the timeout in force then.
- [x] A row without a cause leaves the cell empty and opens nothing. Best guess: yes, like the key
      column, rather than a dash that reads as data.
- [x] `query events` shows the code in the table and both fields in the JSON. Best guess: yes.
- [x] A local failure stays unpersisted, so its code lives in the log only. Best guess: keep —
      persisting it would break INV-5.

## Implementation checklist

- [x] Step 1 — The failure vocabulary: `Code`, `Details`, `render`, `Failure`
- [x] Step 2 — Every prober states its cause
- [x] Step 3 — Every local failure states its code
- [x] Step 4 — Carry the cause to the event
- [x] Step 5 — Store the cause, and rename `DownEpisode` to `Fault`
- [x] Step 6 — Export and import the two columns, rename the route with a redirect
- [x] Step 7 — Show the cause and reveal the message on click
- [x] Step 8 — Separate the tables visually
- [x] Step 9 — Document, ADR-0025, version 0.3.0
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### Step 1 — the failure vocabulary (2026-11-14)

- **Created** `src/probe/failure.rs`: `Details` (13 optional values), `Code` with 26 variants,
  `Code::ALL` for the tests, `render`/`value`, `Failure { code, message }` whose `Display` is
  `CODE — message`. The module doc states the layer-prefix rule.
- **Six unit tests**: every code has an uppercase prefixed token and a template; no two codes
  collide; every template renders with a fully populated `Details` leaving no `{` and no
  `unknown`; a missing detail reads `unknown`; a message states the occurrence; a log line
  carries both halves.

### Step 2 — each prober states its cause (2026-11-14)

- `Verdict` became `Down(Failure)` and therefore lost `Copy`: it is `Clone` only, and the fake's
  script and fallback clone instead of copying.
- `icmp`: `await_reply` now returns `Ok(Option<Duration>)` — the round trip, or `None` when the
  budget expires — so the address and the timeout, which only `exchange` knows, build the
  `ICMP.NO_REPLY` failure in a new `conclusion()` helper. The three platform arms map through it.
- `dns`: `Response::Unresolved` split into `NoSuchName` (RCODE 3) and `NoRecord` (RCODE 0 with no
  answer of the type), which is what makes `DNS.NXDOMAIN` and `DNS.NO_RECORD` distinguishable.
  The system-resolver path states `DNS.UNRESOLVED` for a name it cannot resolve and
  `DNS.NO_RECORD`, with `the system resolver` as the server, for a name of the wrong family.
- `tcp`: `outcome` now takes the address and the timeout and maps refused / reset / timeout;
  `WouldBlock` joins `TimedOut` as `TCP.TIMEOUT`.
- `http`: the private error enum was renamed `Unreachable` (it collided with the new `Failure`),
  carries `Down(Failure)`/`Local(ProbeError)`, and a `Venue { host, port }` travels with it so
  every socket failure can name the endpoint. A status outside the expectation states
  `HTTP.UNEXPECTED_STATUS` with the URL, the status and the expectation
  (`StatusMatch::specs()` joined, e.g. `2xx, 3xx`), and a handshake that failed states `HTTP.TLS`
  through a dedicated `handshake()` mapper.
- **Test adaptation, no weakening**: `Verdict::Down` assertions became assertions on the *code*
  (`code_of` helpers in `tcp`, `dns`, `http` tests), which is strictly stronger; `is_up` in
  `tests/probe_network.rs` takes a reference. No test was deleted, skipped, or relaxed; the
  ignored public-endpoint suite still passes (`5 passed`).

### Step 3 — every local failure states its code (2026-11-14)

- `ProbeError` is now a struct over a `Failure`, built with `ProbeError::of(Code, &Details)`, and
  displays `CODE — message`. Its `code()` and `message()` are read by the tests; the monitor keeps
  logging `probe failed: {error}`, which now carries the token.
- All 31 `ProbeError::local(format!(…))` call sites are gone (`grep -rc "ProbeError::local" src/`
  is empty); `Unreachable::Local` in `tcp` and `http` carries the `ProbeError` itself, so the site
  that knows the cause builds it and the outer `probe()` only forwards.
- **Deviation — three codes added to the table** (26 instead of 23), because the table did not
  cover three real local failures and forcing them into an existing code would have produced a
  false message:
  - `ICMP.INTERFACE` — a raw socket that cannot be bound to the configured device (the plan's
    `ICMP.SOCKET` says "root or CAP_NET_RAW is required", which is wrong for a bind failure).
  - `TCP.CONNECT` — a socket error that is neither a refusal, a reset, nor a timeout (e.g. `EPERM`
    from `connect`); `TCP.SOCKET` says "cannot create", which it is not.
  - `DNS.SERVER_FAILURE` — `SERVFAIL`, `REFUSED`, `NOTIMP`: a definitive answer the product
    classifies as a local failure (FR-21), so it needs a code that is neither a verdict nor an
    unusable answer.
- **Wording refinement**: `DNS.SOCKET` reads `cannot ask {server}: {reason}`, because the same code
  covers the system resolver, where there is no UDP socket to open.
- `icmp` gained three small constructors (`socket_failure`, `io_failure`, `interface_failure`) at
  the parent level, shared by the unix, windows and unsupported-OS arms; the Windows arm was
  updated but remains unexercised on this host, as recorded in the earlier plan.

### Step 4 — the cause reaches the event (2026-11-14)

- `monitor::state::Transition` gained `cause: Option<Failure>` and clones it from
  `Ok(Verdict::Down(failure))`; `Transition` is no longer `Copy` either, and the monitor puts
  `cause`/`cause_message` on the `Event`, taking both from the same `Failure` so they can never
  disagree. Test: `the_cause_of_a_disconnection_reaches_the_transition` asserts the code, the
  message, and that a return to `up` carries none.

### Step 5 — the store, schema 2 and the rename (2026-11-14)

- `SCHEMA_VERSION = 2`; `events.cause` and `events.cause_message` in `schema::TABLES`, both inserts
  widened, `Event`, `EventRow`, `ExportedEvent` and `Fault` carry them, and `FAULTS_SELECT` reads
  them.
- **Migration**: `migrate` keeps the idempotent table creation and then, for a version below 2,
  calls `add_cause_columns`, which checks `pragma_table_info('events')` before each
  `ALTER TABLE … ADD COLUMN` and stamps the new version. Test
  `a_version_one_database_gains_the_cause_columns_and_keeps_its_events` builds a real version 1
  database by hand, opens it, and asserts the version, the surviving row, `NULL` cause, and that a
  second open does not add the columns again.
- **Rename**: `DownEpisode` → `Fault`, `recent_downs`/`recent_downs_for` → `recent_faults`/
  `recent_faults_for`, `DOWNS_SELECT` → `FAULTS_SELECT`, `RECENT_DOWNS` → `RECENT_FAULTS`,
  `downs_table` → `faults_table`, and the user-visible strings: "Last 10 faults", "No fault recorded
  yet.", "faults in the window", "N faults" in the detail header, "faults" as the day-by-day
  column. The `drops` internals (`daily_drops`, `window_drops`) keep their name: they count drops
  in a window, which the glossary still calls a drop.

### Step 6 — the exported format and the route (2026-11-14)

- Header `target,label,interface,state,occurred_at,rtt_us,cause,cause_message`; `row` appends both
  through `field()`, so a message with a comma or a quote is escaped like every other column
  (`a_hostile_cause_message_is_quoted`).
- `import::Row::parse` reads `fields.get(6)` and `fields.get(7)` and leaves them `None` when absent:
  `an_archive_written_before_causes_still_imports` feeds a six-column file and asserts no row is
  skipped; `the_cause_of_a_fault_survives_the_round_trip` exports, imports and reads the codes back.
- The route became `/export/faults.csv.gz` with the file named `faults-YYYYMMDD.csv.gz`;
  `/export/disconnections.csv.gz` answers `308` with `Location` through `respond_redirect`, tested by
  `the_old_export_path_still_answers_and_points_at_the_new_one`.

### Step 7 — the cause and the click (2026-11-14)

- `faults_table`: heading "Last 10 faults", five columns (`target`, the untitled key, `cause`,
  `started`, `lasted`), the code in a `<button class="cause" aria-expanded aria-controls
  title="…">`, and, under a row that has a cause, a `<tr class="cause-row" id="cause-N" hidden>`
  with the message. A row without a cause has an empty cell, no `title`, no message row.
- `CAUSE_SCRIPT` (both pages) toggles `hidden` and `aria-expanded`, closes any other fault first,
  and ignores the click that bubbles from the button so one click is one toggle. Without JavaScript
  the button is inert and the `title` still states the message.
- `query events` gained a `CAUSE` column and the JSON gained `cause` and `causeMessage`; for that,
  `EventRow` carries both.
- **Decision — the open questions, as implemented**: the layer prefix is kept (the real run below
  shows an `http` target stating `TCP.REFUSED`); the old export path redirects; the whole row
  toggles and the button is the keyboard control; one fault is open at a time; a missing detail
  reads `unknown`; the message is stored as it was rendered; a row without a cause opens nothing.

### Step 7, refined on request — the target on two lines, rows of one height (2026-11-14)

- The user asked for taller rows, all the same height, and the technical key **under** its label
  instead of in a column of its own, so the table reads as a grid rather than as a list with an
  empty gap. The table is now four columns: `target`, `cause`, `started`, `lasted`.
- The first cell holds `<span class="fault-name">` with `<span class="fault-key">` beneath it in
  11 px, muted; a target without a label states the key alone and nothing under it. `tr.fault`
  carries `height: 52px`, measured at 52 px for all three rows whether the target has one line or
  two. The message row keeps `colspan="4"`.
- Tests, `PRODUCT.md` FR-17, `ARCHITECTURE.md`, `USER-GUIDE.md` and the counts in the empty state
  follow the new shape; the wording about the "untitled column" is gone from every document.
- **Second refinement, on request**: the cause is the **last** column, so the order is `target`,
  `started`, `lasted`, `cause`. The column is right-aligned with no right padding, so its text ends
  on the same edge the `lasted` figures already end on and the two read as one rail; a test asserts
  the order by the position of the two headers. The `colspan` of the message row stays 4.
- **Third refinement, on request**: the cause column **takes all the room left**, the other three
  taking the width of their content (`table.faults th, table.faults td { white-space: nowrap }`
  plus `width: 100%` on the cause). Measured at a 1400 px viewport: the table is 1040 px and the
  columns are 129 / 178 / 148 / **586**, so the cause holds exactly the 586 px the others leave;
  the page itself never scrolls horizontally.
- **Fourth refinement, on request**: the durations are too close to the code, so the columns take
  stated shares of the width — `table-layout: fixed` with 15 %, 30 %, 45 % and 10 % — and `lasted`
  returns to the **last** column, after the cause. The duration cell was too narrow for "2h28 and
  counting" on one line, so "and counting" is now a block span in 11 px under the figure, like the
  key under a label. Measured at 1400 px: 156 / 312 / 468 / 104 px of 1040, exactly 15 / 30 / 45 /
  10 per cent, no cell overflows, all three rows 52 px.

### Step 8 — the tables announce themselves (2026-11-14)

- `table { margin: 22px 0 0 }`, `th { position: sticky; top: 0; background: Canvas; padding-top:
  11px; border-top: 1px solid var(--rule) }`, the fault row as a pointer, the cause button muted
  with a dotted underline, and the message row inset so it reads as part of its fault. `.table-scroll`
  wraps the five-column table so a narrow screen scrolls the table, never the page.
- **Correction during the step**: a first attempt set `table { display: block; overflow-x: auto }`
  at the narrow breakpoint, which broke the sticky header and the row layout; it was replaced by the
  `.table-scroll` wrapper. Measured at 320 px: the document does not scroll horizontally
  (`scrollWidth == clientWidth == 320`) while the table scrolls inside its 288 px box.

### Step 9 — documentation, decision, version (2026-11-14)

- `PRODUCT.md`: FR-17 (the faults table, the rule that marks its start, the cause column, the click
  and the end of "no client-side interaction beyond following a link"), FR-18 (two new columns, the
  new path, the redirect), FR-19 (a six-column archive still imports), FR-20/21/22 (the codes each
  method states), and a new FR-24 for the code-plus-template rule.
- `ARCHITECTURE.md`: the `probe` output and guarantees, the new `probe::failure` component row, the
  `web` output and route, the `store` migration and the stored message.
- `GLOSSARY.md`: `Down episode` replaced by `Fault`; new `Cause`, `Cause code`, `Cause message`;
  `Verdict` reworded.
- `ADR.md`: ADR-0025 with its index row, placed after ADR-0024.
- `USER-GUIDE.md`: the faults table and how a message is opened, the table of codes an operator
  meets, the CSV columns, the new export path, and the 0.2 → 0.3 upgrade note (schema 2, the two
  columns added, an older binary refusing the database, the export moving).
- `DEVELOPER-GUIDE.md`: "Adding a probe type" gained the obligation to declare codes and templates;
  the import examples use `faults-*.csv.gz` and the new route. `README.md` follows.
- `Cargo.toml`: `0.3.0`.

### Verification of the whole plan (2026-11-14)

- `cargo fmt --check` ✓, `cargo clippy --all-targets -- -D warnings` ✓, `cargo test` ✓ 210 + 14 + 18
  + 4 + 5 + 4 + 20 + 7 + 1 tests, 0 failed, 10 ignored.
- `cargo test --test probe_network -- --ignored` ✓ 5 passed, so the causes hold against public
  endpoints.
- Real run of `monitor` with three failing targets: `tcp://127.0.0.1:9` → `TCP.REFUSED`,
  `http://127.0.0.1:9/health` → `TCP.REFUSED` (the layer, not the target method), the DNS target
  up; `query events` printed the `CAUSE` column, and the JSON carried `cause` and `causeMessage`.
- Real page: the table under its rule, a click opening "the connection to 9.9.9.9:443 was refused"
  under its row with `aria-expanded` set, the detail page showing the same list, the export
  decompressing to the eight columns, and the old path answering `308`.

## Implementation Notes

### Notebook of the fixes requested by review round 1 (2026-11-15)

Finding 1 (major, `serve` and `query events` fail with `no such column: cause` on a 0.2 database).
`store::open_ro` opened read-only and never migrated, so a version 1 file stayed version 1 while the
queries already selected `events.cause`; `docs/ARCHITECTURE.md` promised a database "migrated on
open", so the code was wrong, not the document. Two helpers now carry the shared part:

- `read_only_connection(path)` builds the read-only pool with the writable pragmas the migration
  needs.
- `checked_version(&connection)` reads `PRAGMA user_version` and applies `migrate` when the file is
  below the current version, refusing a file from the future as before.

`open_ro` therefore opens, migrates through a short writable connection, then reopens read-only.
The upgrade note in `docs/USER-GUIDE.md` changed to match: any open — `serve`, `query`, `import` —
adds the cause columns, not only "the first writable open".

Two tests in `src/store/mod.rs` pin the new behaviour: `a_read_only_open_migrates_a_version_one_database`
(the `version_one_database` fixture, opened read-only, then read back at `version 2` with the cause
column present) and `a_read_only_open_still_refuses_a_database_from_the_future` (a version above the
current one is still an error). End to end, `serve` alone against a freshly built version 1 file
answers `HTTP 200` and the file reports `version after serve: 2`.

Finding 2 (major, only the generic template test covered the 14 remaining codes). Every code that a
test can reach is now asserted, usually through the real path rather than through `render`:

- `DNS.NO_RECORD`, `DNS.SERVER_FAILURE`, `DNS.UNUSABLE`, `DNS.SOCKET`, `DNS.BUILD` in `src/probe/dns.rs`
  (wire-level responses over UDP and a socket that refuses to be created).
- `HTTP.TLS` (a silent TLS server that drops the handshake), `HTTP.RESOLVE` (a host that does not
  resolve) in `src/probe/http.rs`.
- `TCP.INTERFACE`, `TCP.CONNECT` in `src/probe/tcp.rs`, and the ICMP helpers
  (`socket_failure`, `io_failure`, `interface_failure`) in `src/probe/icmp.rs`.

Two codes stay unasserted, on purpose: `TCP.SOCKET` and `HTTP.TLS_INIT` need the operating system to
refuse a socket or to fail a context initialisation that already worked once, which no deterministic
test can force. They are covered by the template test only, and by the entry in
`## Known limitations`.

Finding 3 (minor, `Details.url` received only the host). `Venue` gained `scheme` and `target` and a
`url()` method, so `HTTP.MALFORMED` and `HTTP.HEAD_LIMIT` now name `http://host:port/path` instead of
the bare host and port. `parse_status` takes the `Venue` rather than the port, and the tests assert
that the message starts with `http://127.0.0.1:{port}/`.

Found while fixing the above: `socket_failure` appended "(root or CAP_NET_RAW is required)" twice,
because the reason it received already carried it. Fixed in `src/probe/icmp.rs`; the helper is now
covered by `the_socket_helpers_state_their_code`.

Finding 4 (nit, `308` answered with `no-store`) is left as the reviewer wrote it: a nit is report-only
and the redirect carries no body, so there is nothing else to fix here.

Verification of the fix round: `cargo fmt --check` ✓, `cargo clippy --all-targets -- -D warnings` ✓,
`cargo test` ✓ 294 passed, 0 failed, 10 ignored.

### Notebook of the fixes requested by review round 2 (2026-11-14)

Finding 1 (major, the read-only open wrote to the history). The previous fix reached the migration
through `Store::open_rw`, which also runs `rewrite_legacy_target_keys`: a single page view on a
pre-methods database rewrote `events.target` from `10.9.9.9` to `icmp://10.9.9.9`, against `INV-10`
and `BR-8`. Two behaviours were possible — keep the history untouched, or widen the permission and
record it — and the first was chosen: it keeps the invariant and the business rule, and it needs no
new decision record to justify a page that edits history.

- `add_missing_columns(path)` is the new private helper: a short writable connection that applies
  `PRAGMAS` and `migrate`, and nothing else. `open_ro` calls it instead of `Store::open_rw`, so a
  read-only entry point adds the columns of a schema one version behind and rewrites no row. The
  first writable open (`monitor`, `import`) still converts the bare addresses, as `FR-23` says.
- Test `a_read_only_open_leaves_the_history_as_it_is` builds a version 1 database holding a bare
  key, opens it read-only, asserts the keys are `["icmp://1.1.1.1", "9.9.9.9"]` and the version 2,
  then opens it writable and asserts they became `["icmp://1.1.1.1", "icmp://9.9.9.9"]`.
- Documents restated where the single write of a read-only open matters: `INV-10` (now "never
  alters the history … the only write is adding the columns of a schema one version behind, never an
  `UPDATE` or a `DELETE` of a row"), ARCHITECTURE 64, 151, 169, 184, 292 and the `web` security
  paragraph, PRODUCT `FR-17` and the web-page row of the requirements table, and the user guide's
  upgrade section, which now states that `serve` and `query` change no row while the first
  `monitor` or `import` rewrites the keys of a 0.1 history. ADR-0025 gains the consequence: the
  migration runs on every open, that is the only write a read-only open performs, and a stale
  database on a non-writable file now stops `serve` and `query` at open instead of answering a
  failed query per request.
- Measured after the fix: the same pre-methods database served one request (`home=200`), read back
  `events.target` unchanged at `10.9.9.9` with `schema_meta.version` at 2, and `query events`
  printed the row.

Finding 2 (minor, stale type name). `docs/ARCHITECTURE.md` listed `DownEpisode` in the `store`
contract row; it now lists `Fault`.

Finding 3 (nit, `308` answered `no-store`). `respond_redirect` no longer sends `Cache-Control`, so a
cache may remember a permanent redirect; it keeps `X-Content-Type-Options: nosniff` like every other
answer. `the_old_export_path_still_answers_and_points_at_the_new_one` asserts the header is absent;
the live check reports `308` with `Location: /export/faults.csv.gz`.

Verification of this round: `cargo fmt --check` ✓, `cargo clippy --all-targets -- -D warnings` ✓,
`cargo test` ✓ 295 passed, 0 failed, 10 ignored.

### Notebook of the change requested after review round 3: a read-only verb that never writes

The reviewer of round 3 accepted the fix of round 2 but the user asked the question behind it: was
migrating the schema on the read-only verb really necessary, or could a 0.2 database be read
without altering it? It could. The columns are additive and nullable, and every field of `Fault`,
`EventRow` and `ExportedEvent` is already `Option<String>`, so a reader can select `NULL` in their
place and behave identically.

- `Store` gains `has_cause`, set at open from the version `checked_version` reports;
  `cause_columns()` returns `cause, cause_message` or `NULL, NULL`, and `faults_select()` fills the
  `{cause}` placeholder of `FAULTS_SELECT`. The three read texts — `FAULTS_SELECT` (four call
  sites), `events` and `events_after` — are the only places that name the columns.
- `open_ro` no longer writes at all: `add_missing_columns` is deleted, so `serve` and `query` open
  a database one version behind as it is. A read-only verb therefore works against a snapshot, a
  backup, or a volume mounted read-only, which is what `INV-10`, `BR-8` and the guide promise.
- Tests: `a_read_only_open_migrates_a_version_one_database` becomes
  `a_read_only_open_reads_a_database_without_the_cause_columns` (faults, events and the streamed
  export all answer with `cause: None`, and the file stays at version 1), a new
  `a_read_only_open_of_a_migrated_database_reads_the_cause` covers the other shape, and
  `a_read_only_open_leaves_the_history_as_it_is` now also asserts the version it finds.
- Documents: `INV-10` is back to "the web page never writes"; ARCHITECTURE 64, 145, 151, 169, 184,
  223, 265, 292 and the `web` security paragraph say the migration belongs to a writable open;
  PRODUCT `FR-17` and the requirements row say the page writes nothing; the guide's upgrade section
  states that `serve` and `query` never write and that a 0.2 database states no cause until a
  `monitor` or an `import` has upgraded it; ADR-0025 replaces the previous consequence with the
  read-only one, including the cost (a read path that knows two shapes of the table). The
  developer guide's "Changing the schema" gained the rule that a new column must be selectable as
  `NULL` by a read-only open.

Measured after the change: a version 1 database on a file with mode `444` served `200`, exported
`target,label,interface,state,occurred_at,rtt_us,cause,cause_message` with both cause fields empty,
answered `query events`, and kept `schema_meta.version` at 1 and `events.target` at `10.9.9.9`; an
`import` afterwards stamped version 2 and converted both keys to `icmp://10.9.9.9`.

Verification of this round: `cargo fmt --check` ✓, `cargo clippy --all-targets -- -D warnings` ✓,
`cargo test` ✓ 296 passed, 0 failed, 10 ignored.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(probe): give every failure a code and a standard message`
- **Quality gate**: tests ✓ (283 passed, 10 ignored) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check --all-targets && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/store/mod.rs` (`open_ro`) | A database written by 0.2 breaks every read path that selects the new columns until something opens it writable, and `serve` never does. Reproduced with a hand-built version 1 database: `serve` answers `500 query failed` and logs `database error: no such column: e.cause`; `query events` exits 1 with the same error, while `query state` works because it selects no cause. A serve-only deployment — a documented mode — loses its page after the upgrade, and what the migration adds. ARCHITECTURE says "the schema is created and migrated on open". Fix: migrate in `open_ro` too (open read-write, run the version step, reopen read-only), not only in `open_rw`. | open |
| 2 | major | `src/probe/{dns,http}.rs` tests | The code each failure states is asserted for `TCP.REFUSED/RESET/TIMEOUT/RESOLVE`, `DNS.NXDOMAIN/TIMEOUT/UNRESOLVED`, `HTTP.UNEXPECTED_STATUS/MALFORMED/HEAD_LIMIT`, `ICMP.IO` and `PROBE.WRONG_METHOD`; fourteen codes are only covered by the generic "every template renders" test, so the mapping from a site to its code is unverified. Several are reachable with the fixtures already in the file: `DNS.SERVER_FAILURE` and `DNS.UNUSABLE` (the local UDP server can answer rcode 2 or a truncated head — `a_server_failure_is_not_a_verdict` stops at `parse_response`), `DNS.NO_RECORD` over the wire, `HTTP.TLS` (serve plain bytes to an `https://` target). `ICMP.SOCKET`, `ICMP.INTERFACE`, `TCP.SOCKET`, `TCP.INTERFACE` need privileges or a broken host and are acceptable as they are. | open |
| 3 | minor | `src/probe/http.rs:466`, `:314` | `Details.url` receives only `venue.host`, so an `HTTP.MALFORMED` or `HTTP.HEAD_LIMIT` message reads `127.0.0.1 answered something that is not an HTTP answer` instead of naming the URL the template promises. The `Venue` would need the target path, which `exchange` does not have. | open |
| 4 | nit | `src/web/mod.rs` (`respond_redirect`) | A `308` answers `Cache-Control: no-store`, which tells a cache not to remember a redirect that is meant to be permanent. | open |

### Gap analysis

- Goals: 8/9 achieved. Goal 8 — "a database created by 0.2 opens in 0.3 and keeps every event" — holds
  for `monitor` and `import` and fails for `serve` and `query events` (finding 1); the rows are kept,
  the reads are not.
- Scope: in scope. Every changed file is listed under Affected modules; the two beyond it are
  `Cargo.lock` (produced by the version bump) and the plan itself.
- Deviations: documented — three codes added beyond the table (`ICMP.INTERFACE`, `TCP.CONNECT`,
  `DNS.SERVER_FAILURE`) because forcing those failures into a neighbour code would have produced a
  message that lied; `DNS.SOCKET` widened to `cannot ask {server}: {reason}` so it also covers the
  system resolver; `icmp::await_reply` returns `Option<Duration>` so the cause is built where the
  address and the timeout are known; the `display: block` attempt for narrow screens replaced by the
  `.table-scroll` wrapper.
- Additions: five UI refinements the user asked for during the phase — key under the label, rows of one
  height, cause last, cause taking the remaining width, and the narrow-screen scroller. All recorded
  in Implementation Notes and mirrored in the documents.
- Deferred: none.
- Row-by-row: 1 ✗ (findings 1 and 2), 2 ✓, 3 ✓, 4 ✗ (finding 2), 5 ✓ (no test deleted: `#[test]`
  count 277 → 293; the removed assertions are the shape migrations, each replaced by a stronger one —
  `Verdict::Down` became an assertion on the code, `contains("resolve")` became the code plus the
  message), 6 ✓ (13/13, every open question ticked with the decision), 7 ✓ (PRODUCT FR-17/18/19/20-22
  and the new FR-24, ARCHITECTURE component row and guarantees, GLOSSARY `Fault`/`Cause`/`Cause
  code`/`Cause message`, ADR-0025 in order after 0024, USER-GUIDE with the codes table and the 0.2 →
  0.3 note, DEVELOPER-GUIDE, README, `Cargo.toml` 0.3.0), 8 ✗ (finding 1 contradicts "migrated on
  open"), 9 ✓, 10 ✓ (the store test builds a version 1 database by hand and opens it twice; hostile
  messages go through `escape` and `field`), 11 ✓ (no new `unsafe`, no dependency added, TLS
  verification still not optional, the script only toggles `hidden` and an attribute), 12 n/a, 13 ✓
  (no configuration option changes, so nothing to add to `Configuration`; no link to an internal
  document; the summary is three lines; the codes table matches `probe::failure`).

### Known limitations

- The message of a fault is stored as it was rendered, so changing a template does not rewrite history.
- A connect interrupted by a signal stays a local failure and is never retried.
- The codes of the Windows ICMP arm and of the privileged raw socket are exercised only by the
  `#[ignore]` acceptance test.

### Recommendations for future work

- Give `Venue` the target path, so `HTTP.MALFORMED` and `HTTP.HEAD_LIMIT` can name the URL (finding 3).
- Drop the `/export/disconnections.csv.gz` redirect in 0.4 with a `BREAKING CHANGE` note, as ADR-0025
  says.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(probe): give every failure a code and a standard message`
- **Quality gate**: tests ✓ (294 passed, 10 ignored) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check --all-targets && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/store/mod.rs:333` (`open_ro`) | The round-1 fix made the read-only open write, and nothing says so. `open_ro` calls `Store::open_rw`, which runs `migrate` and then `rewrite_legacy_target_keys`, whose second statement is `UPDATE events SET target = 'icmp://' \|\| target …`. On a pre-methods database a single page view rewrites the `target` of every event: built `events.target = '10.9.9.9'`, served one request (`200`), read back `icmp://10.9.9.9`. That contradicts `INV-10` ("The web page never writes: every request opens the database read-only and issues only `SELECT` statements", ARCHITECTURE:345), ARCHITECTURE:281 ("`web` … and never writes"), ARCHITECTURE:184-185 ("A writable open rewrites the bare addresses … the read-only open the page uses never writes (INV-10)"), ARCHITECTURE:64 and :291 ("the `query` component … cannot repair, migrate, or compact a database"), `BR-8` (PRODUCT:98, "a page must never be able to alter the history"), `FR-17` (PRODUCT:76, "The read-only web page …", "Every request opens the database read-only"), `FR-23` (PRODUCT:84, "the first writable open rewrites each bare address"), the guide's `serve` row (USER-GUIDE:253, "read-only") and its upgrade item 3 (USER-GUIDE:343, "the first writable open rewrites bare addresses"). ADR-0008 keeps its "can never write" rationale and no decision record covers the change. Side effect measured: on a stale database on a non-writable file, `serve` and `query events` now exit with `database error: attempt to write a readonly database` instead of starting. The new test `a_read_only_open_migrates_a_version_one_database` asserts the migration but not immutability, and its comment ("never write otherwise") is the claim that does not hold. | open |
| 2 | minor | `docs/ARCHITECTURE.md:123` | The `store` component row still names `DownEpisode` among its public types; the type is `Fault`. | open |
| 3 | nit | `src/web/mod.rs` (`respond_redirect`) | Carried from round 1: a `308` answers `Cache-Control: no-store`. | open |

### Gap analysis

- Goals: 9/9 achieved. Goal 8 now holds and was verified this round: a hand-built version 1 database opens through `serve` alone, answers `200`, reports `version 2`, keeps its event, and `query events` prints it with an empty cause. Goals 1-7 and 9 were re-verified live: the page renders the four columns (target 15 %, started 30 %, cause 45 %, lasted 10 %), the button carries `aria-expanded`/`aria-controls` and the message in `title`, the hidden `tr.cause-row` sits under its row, the CSV is `target,label,interface,state,occurred_at,rtt_us,cause,cause_message`, and `/export/disconnections.csv.gz` answers `308` to `/export/faults.csv.gz`.
- Scope: in scope. Every changed file is listed under Affected modules; the two beyond it remain `Cargo.lock` (version bump) and this plan.
- Deviations: the two the fix round introduced are documented in Implementation Notes (the read-only open now migrates; the two codes left unasserted). The first is documented as a decision, which is what makes finding 1 a documentation gap rather than an undocumented deviation.
- Additions: the fix round added assertions for `DNS.NO_RECORD`, `DNS.SERVER_FAILURE`, `DNS.UNUSABLE`, `DNS.SOCKET`, `DNS.BUILD`, `HTTP.TLS`, `HTTP.RESOLVE`, `TCP.INTERFACE`, `TCP.CONNECT` and the ICMP helpers, plus `Venue::url()`. Verified: 24 of the 26 codes now have an assertion outside `probe/failure.rs`; `TCP.SOCKET` and `HTTP.TLS_INIT` have none, as the notes state, and that limit is recorded.
- Deferred: none.
- Row-by-row: 1 ✓ (9/9, verified live), 2 ✓, 3 ✓, 4 ✗ (finding 1: no test pins what a read-only open is allowed to write), 5 ✓ (`#[test]` count 277 → 304; the four removed test names are renames — `recent_faults_are_newest_first_and_keep_the_open_one`, `the_last_faults_sit_above_the_day_by_day_table`, `the_last_faults_are_listed_with_their_duration`, `statistics_count_only_faults_that_start_inside_the_window` — and the `parse_status` assertions survive with the new `Venue` argument), 6 ✓ (13/13, all open questions ticked, Implementation Notes present with a block for each round), 7 ✓ for the feature and ✗ for the read-only guarantee (finding 1), 8 ✗ (finding 1 contradicts `INV-10` and `ADR-0008`), 9 ✓, 10 ✓ (`grep -rn "ProbeError::local" src` returns nothing; every template renders with no `{` and no `unknown` for the details its code declares), 11 ✓ (no new `unsafe`, no dependency, `escape` on the code and the `title`, the message row goes through `escape`, the script only toggles `hidden` and an attribute), 12 n/a, 13 ✗ (finding 1: the guide's `serve` row and its upgrade item 3 are inaccurate; otherwise accurate, self-contained, three-line summary, no configuration option added).

### Known limitations

- The message of a fault is stored as it was rendered, so changing a template does not rewrite history.
- `TCP.SOCKET` and `HTTP.TLS_INIT` are covered only by the template test: no deterministic fixture makes the platform refuse a socket or fail a TLS context initialisation that already succeeded.
- A local failure stays in the log and is never persisted (INV-5).

### Recommendations for future work

- Resolve finding 1 by choosing one behaviour and stating it: either keep `rewrite_legacy_target_keys` out of the read-only path so a page view only adds the two columns and never alters the history (which keeps `BR-8` intact and needs `INV-10`, ARCHITECTURE:184-185/:281/:291, PRODUCT `FR-17`/`FR-23`, the guide's `serve` row and upgrade item 3 reworded to "the read-only open migrates a database one version behind and writes nothing else"), or record the wider permission in ADR-0025 or a new record and update the same documents. Either way add the test that pins it, and mention the non-writable-file behaviour of a stale database.
- Drop the `/export/disconnections.csv.gz` redirect in 0.4 with a `BREAKING CHANGE` note, as ADR-0025 says.

## Review Notes

- **Round**: 3
- **Review date**: 2026-11-14
- **Verdict**: PASS
- **Suggested commit**: `feat(probe): give every failure a code and a standard message`
- **Quality gate**: tests ✓ (295 passed, 10 ignored) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check --all-targets && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | nit | `docs/ADR.md:179` (ADR-0006) | The record still says "The query path cannot repair, migrate, or compact a database"; a `query` open now brings an older schema forward. Left as written: a decision record is historical, and ADR-0025 records the qualification ("the migration runs on every open … the only write a read-only open performs"). | open |
| 2 | nit | `tests/web.rs:675` | `serving_never_writes_to_the_database` compares the number of events before and after three page views, not their content, so it would not have caught the round-2 rewrite of a target key. The behaviour is now pinned by the store test, so nothing is unverified; the web-level assertion could compare the rows themselves. | open |

### Gap analysis

- Goals: 9/9 achieved, each re-verified in this round. Goal 1: `grep -rn "ProbeError::local" src` returns nothing and `Code::ALL` holds 26 codes. Goal 2: `every_code_states_a_token_and_a_template`, `no_two_codes_share_a_token`, `every_template_renders_with_the_details_its_code_supplies`. Goal 3: `Verdict::Down(Failure)` and `ProbeError::of(Code, &Details)`. Goal 4: the heading "Last 10 faults", `FAULTS_SELECT`, `/export/faults.csv.gz`, the 308 redirect, `query` columns. Goal 5: served page shows `tr.fault` with `button.cause[aria-expanded][aria-controls][title="no echo reply …"]` and the hidden `tr.cause-row#cause-0` under it, and a row without a cause leaves the cell empty and opens nothing. Goal 6: the cause is read back from the same database by the CSV and by `query`. Goal 7: the CSV is `target,label,interface,state,occurred_at,rtt_us,cause,cause_message` with `ICMP.NO_REPLY,"no echo reply from 9.9.9.9 within 2s"` on the fault row and empty on the `up` row; a six-column archive still imports (`imported 1 events for 1 target(s) … 0 rows skipped`). Goal 8: a hand-built version 1 database with a bare key, opened by `serve` alone, answers `200`, reports `schema_meta.version` 2, and keeps its event. Goal 9: the stylesheet keeps the top rule on `thead th`, the sticky header painted with `Canvas`, the 15/30/45/10 per cent widths and the 52 px rows.
- Round-2 findings: 1 fixed and verified live — the same pre-methods database, after one page view, still reads `events.target = '10.9.9.9'` at version 2, while a writable open (`import`) converts both rows to `icmp://10.9.9.9`, so `FR-23` holds and `INV-10`/`BR-8` are true again. 2 fixed — `docs/ARCHITECTURE.md:123` lists `Fault`; no current document names `DownEpisode`, `recent_downs` or `downs_table`. 3 fixed — the 308 answers `Location`, `Content-Length: 0`, `X-Content-Type-Options: nosniff` and no `Cache-Control`.
- Scope: in scope. Every file in the diff is listed under Affected modules, plus `Cargo.lock` (version bump) and this plan.
- Deviations: documented in Implementation Notes — a read-only open migrates the schema while rewriting no row (the round-2 fix), the three codes beyond the plan's table, `DNS.SOCKET` widened, `icmp::await_reply` returning `Option<Duration>`, and the `.table-scroll` wrapper.
- Additions: the UI refinements the user asked for during the phase (key under the label, equal row heights, cause last and then `lasted` last, the 15/30/45/10 per cent widths, the horizontal scroller) and the round-1/round-2 coverage work; all recorded.
- Deferred: none.

### Known limitations

- A message is stored as rendered, so changing a template does not rewrite history.
- `TCP.SOCKET` and `HTTP.TLS_INIT` have no deterministic fixture; they are covered by the template test only.
- A local failure states its code in the log and is never persisted (INV-5).
- A database one version behind on a file that cannot be written stops `serve` and `query` at open; the upgrade needs one writable open, which the user guide and ADR-0025 both state.

### Recommendations for future work

- Drop the `/export/disconnections.csv.gz` redirect in 0.4 with a `BREAKING CHANGE` note, as ADR-0025 says.
- Compare the stored rows, not only their count, in `serving_never_writes_to_the_database`.

## Review Notes

- **Round**: 4
- **Review date**: 2026-11-14
- **Verdict**: PASS
- **Suggested commit**: `feat(probe): give every failure a code and a standard message`
- **Quality gate**: tests ✓ (296 passed, 10 ignored) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo check --all-targets && cargo test`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | nit | `docs/ADR.md:179` (ADR-0006) | Carried from round 3: the record still says "The query path cannot repair, migrate, or compact a database", which is true again after this round, so nothing to change — the sentence is historical and correct. Recorded only to close it. | closed |
| 2 | nit | `docs/DEVELOPER-GUIDE.md:604` | The new rule points at "`Store::cause_columns`", which is a private method (`fn cause_columns(&self)`), not a qualified public item; naming it as a pattern rather than a path would be more accurate. | open |
| 3 | nit | `docs/ARCHITECTURE.md:145` | The guarantee now reads "… and keeps every row —, a read-only open reading an older database as it is; …", an awkward clause that a reader has to parse twice. | open |

### Gap analysis

- Goals: 9/9 achieved. Goal 8 ("a database created by 0.2 opens in 0.3 and keeps every event") now holds through two paths, both verified live this round: a version 1 database on a file with mode `444` served `200` and exported `target,label,interface,state,occurred_at,rtt_us,cause,cause_message` with empty causes, `query events` answered, and the file was **byte-identical** afterwards (`md5` 41d2683e…, `schema_meta.version` 1, `events.target` still `10.9.9.9`, `pragma table_info('events')` with no cause columns); an `import` then stamped version 2, added both columns, converted both keys to `icmp://10.9.9.9`, and a cause written afterwards is read back by `query events` as `TCP.REFUSED`. Goals 1-7 and 9 are unchanged since round 3 and still covered by the same evidence (`ProbeError::local` absent, 26 codes, the fault table with its control and message row, the eight CSV columns, the 308).
- Scope: in scope. This round touched `src/store/mod.rs`, the five documents the change obliges, and this plan; every one is under Affected modules.
- Deviations: the read path now knows two shapes of `events` (`cause, cause_message` or `NULL, NULL`, chosen from the version the open found) instead of migrating on a read-only open. Documented in Implementation Notes and recorded in ADR-0025 as the cost of the decision.
- Additions: none beyond the change the user asked for; the tests that pinned the previous behaviour were replaced by ones that pin the new behaviour, with one more assertion each (three read paths plus the version, where the old test asserted only the version).
- Deferred: none.

### Known limitations

- A 0.2 database read only by `serve` or `query` states no cause on its rows; the columns arrive with the first `monitor` or `import`, which the user guide and ADR-0025 both state.
- A database left at version 1 while already carrying the cause columns by hand would be read with `NULL` causes; the schema is written by the binary only, so this needs deliberate interference.
- `TCP.SOCKET` and `HTTP.TLS_INIT` remain covered by the template test only.
- A local failure states its code in the log and is never persisted (INV-5).

### Recommendations for future work

- `a_read_only_open_of_a_migrated_database_reads_the_cause` asserts the version and that a legacy row reads no cause; the cause round trip itself is asserted by `recent_faults_are_newest_first_and_keep_the_open_one`. A name closer to what it proves would read better.
- Compare the stored rows, not only their count, in `serving_never_writes_to_the_database` (`tests/web.rs:675`), carried from round 3.
- Drop the `/export/disconnections.csv.gz` redirect in 0.4 with a `BREAKING CHANGE` note, as ADR-0025 says.
