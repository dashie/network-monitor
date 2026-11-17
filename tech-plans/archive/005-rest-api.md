---
title: Add a read-only public REST API under /api/v1
date: 2026-11-14
status: done
type: feature
impact: medium
area: web/api
version_bump: minor
tags: [api, rest, http, json, openapi]
depends_on: []
branch_recommended: true
---

## Summary

The web server that already publishes the read-only page also exposes a read-only JSON API
under `/api/v1`: the targets, the faults, and the per-day totals of the history it serves,
each filterable by target and by a date range and each paginated with `limit`/`offset`. Every
answer is an object — `{"data": …, "meta": …}` — and never a bare array; every error is
`{"error": {"status", "code", "parameter", "message"}, "meta": …}`. The contract is described
by an OpenAPI 3.1 document served at `/api/v1/openapi.json`. Nothing is written, no new
listening socket appears, and the HTML pages are unchanged.

## Why

The history is already machine-readable — the SQLite file, `query --format json`, and the CSV
export — but none of those is reachable from a dashboard over the network that the page
already serves. `query` needs a shell on the host, the CSV is a bulk transfer, and reading the
SQLite file remotely means shipping a client and accommodating the writer. The page itself is
for people: it answers one window at a time and offers no filter beyond the target.

An operator who runs a homelab dashboard, a status page, or a Telegram bot of their own can
reach the same history today only by scraping HTML or mounting the volume. A small, stable,
paginated JSON API over the socket that is already open removes that barrier without adding a
database, a dependency, or an authentication surface.

## Goals

- `GET /api/v1/targets` returns one record per target — key, label, interface, state, when the
  state began, when the target was first seen, the last round-trip time, and how long `up` has
  held — ordered by key, paginated.
- `GET /api/v1/faults` returns one record per fault, newest first, filterable by `target`
  (canonical key or label) and by `from`/`to`, paginated, each record stating the start, the
  end, the duration, and the cause.
- `GET /api/v1/faults/daily` returns one record per UTC day, oldest first, filterable by
  `target` and `from`/`to`, paginated, each record stating its target — `null` when the answer
  aggregates every target — faults, down time, the average and the longest `up` run of the day,
  and whether the day is still running. A day the window clips at its edges counts only what
  falls inside it, and the UTC day still running states the last 24 hours. *(Amended 2026-11-14:
  the field was missing; the user chose one series with a nullable `target` and confirmed the
  30-day default window. Amended 2026-11-15: the clipping rule is stated here because three
  review rounds found it unstated — Step 15.)*
- `GET /api/v1/openapi.json` returns the OpenAPI 3.1 document of the four routes.
- Every success is a JSON object whose `data` holds an array and whose `meta` echoes `limit`,
  `offset`, `total`, the resolved filters, and the instant the answer was built; no endpoint
  ever returns an array as the response root.
- Every failure is a JSON object with a stable `code`, an HTTP status that matches it, and the
  offending `parameter` when there is one; `limit` outside `1..=1000`, a malformed `from`/`to`,
  a `from` not before `to`, an unknown parameter, an unknown `/api/v1` path, and a non-`GET`
  method are all rejected with `4xx`, never silently ignored.
- The API answers `HEAD`, is gzipped exactly like the page under `--compression auto`, states
  `Cache-Control: no-store`, and carries `Access-Control-Allow-Origin: *`.
- The date filters accept the RFC 3339 UTC form the tool writes (`2026-11-14T22:00:00.000Z`)
  and epoch milliseconds; `from` is inclusive and `to` exclusive. An accepted instant lies in
  the fixed calendar domain `1970-01-01T00:00:00Z … 2100-01-01T00:00:00Z`, and a value outside
  it is `400 INVALID_PARAMETER` naming the parameter. *(Amended 2026-11-15: review round 5
  showed that "any value that fits `i64`" cannot be kept by calendar arithmetic and that a
  ceiling anchored to `now` refuses the natural "this month"/"this year" queries — Step 20.)*
- The three data routes stream: each record is written as it is produced, gzip compresses on
  the fly, the answer is `Transfer-Encoding: chunked` with no `Content-Length`, and no route
  builds the whole JSON text in memory. `/faults` reads the history in keyset pages of 200, so
  the memory of one answer is one page whatever the total; `/targets` and `/faults/daily` are
  bounded by the number of targets and by the length of the window, and state that bound.
- Every route has its own module: `src/web/api/mod.rs` holds the shared contract, and
  `targets.rs`, `faults.rs`, `daily.rs`, and `openapi.rs` hold one route each.
- An error states a fixed message. The detail of a store failure goes to the log only, and no
  answer echoes the request path, so no database path, no SQLite text, and no client input
  reaches the reader.
- The existing behaviour — HTML pages, CSV export, `query`, schema, configuration — is
  unchanged, and no new crate is added to `Cargo.toml`.

## Non-goals

- No write method, no authentication, no token, no rate limit beyond the connection cap the
  server already has (32 concurrent connections). The API is public by design, exactly like
  the page (`BR-8`).
- No new endpoint beyond the four above: no single-target route, no event-level route, no
  CSV-through-the-API, no `/healthz` change.
- No API version negotiation beyond the fixed `/api/v1` prefix: a breaking change means a new
  prefix, not a header or a query flag.
- No change to the database schema, to the stored timestamps, or to the CSV format.
- No database query cache, no cursor pagination, no aggregate endpoint beyond the daily totals.

## Dependencies

None: no other plan is a prerequisite.

## Affected modules

**Source**
- `src/store/mod.rs` — `EventFilter` gains `until_ms` and `offset`; new `FaultFilter`,
  `Store::faults`, `Store::count_faults`; `Fault` gains `interface`; `FAULTS_SELECT` reads it;
  `Store::daily_drops` takes `Option<&str>` so it aggregates every target.
- `src/json.rs` — *(new)* the JSON primitives moved out of `query::render`: string escaping,
  option numbers, option strings, array assembly.
- `src/lib.rs` — register `pub mod json;`.
- `src/query/mod.rs` — name the two new `EventFilter` fields (`until_ms`, `offset`) as `None`.
- `src/query/render.rs` — call the moved primitives; the rendered output must not change.
- `src/web/api/mod.rs` — *(new)* the shared contract of the API: dispatch, `Reply`, `Rejection`,
  `Params`, the envelope and error writers, and the chunked streaming writer.
- `src/web/api/targets.rs`, `src/web/api/faults.rs`, `src/web/api/daily.rs`,
  `src/web/api/openapi.rs` — *(new)* one route per file, with the record builder it owns.
- `src/store/mod.rs` — *(added)* the keyset fields of `FaultFilter` and the cursor they build,
  so `/faults` reads one page at a time without repeating a row.
- `src/web/mod.rs` — hand `/api/v1/*` to `web::api` before the static routes; accept `OPTIONS`
  for those paths; factor the gzip response writer so pages and JSON share it.
- `src/web/stats.rs` — **in scope from Step 21** (review round 5): `up_runs_per_day` walks one
  day at a time and its `day + day_ms` overflows near the top of the range, so the walk must
  advance by construction. `up_runs_between` and `summarize` stay as they are.
- `src/store/mod.rs` — *(Step 21)* the private `day_slices` day walk: the same structural
  advance guard as `up_runs_per_day`.
- `src/web/page.rs` — reused as is (`up_for`); no edit expected.
- `src/time.rs` — reused as is (`parse_rfc3339`, `now_ms`, `format_rfc3339`); no edit expected.

**Assets**
- `assets/api/openapi.json` — *(new)* the OpenAPI 3.1 document, embedded with `include_str!`.

**Tests**
- `src/store/mod.rs` — unit tests for the extended filter, the fault queries, the count, and
  `daily_drops` across every target; *(Step 21)* the bounded walk of `day_slices` at the edges
  of `i64`.
- `src/web/stats.rs` — *(Step 21)* the bounded walk of `up_runs_per_day` when a run ends at the
  edge of the range.
- `src/web/api/mod.rs` — *(Step 20)* the domain constants and the unit test that pins them to
  the two calendar dates.
- `tests/web.rs` — the HTTP contract of the four routes, the envelope, pagination, filters,
  errors, CORS, gzip, `HEAD`, and the OpenAPI drift guard. The file already holds `Server` and
  `seeded_database`, so the API is exercised by the same harness as the page instead of a
  second copy of it. *(Step 20)* the domain refusals and the natural period queries replace
  `an_instant_at_the_end_of_the_range_answers_an_envelope`.

**Documentation / configuration**
- `docs/PRODUCT.md` — new `FR-26` for the API; `BR-8` states that the one socket serves the
  page and the API; *(Step 22)* `FR-26` also states the accepted instant domain.
- `docs/USER-GUIDE.md` — new `## REST API` section after `## The web page` (routes, envelope,
  parameters, pagination, errors, curl examples, OpenAPI address, the absence of
  authentication); the `--compression <mode>` row of `## Command line` now covers the JSON
  answers too; *(Step 22)* the parameter table and the paragraph under it state the accepted
  instant domain, and the `faults`/`daily` `from` row stops promising "no bound on faults".
- `docs/ARCHITECTURE.md` — a `web::api` component entry (input, output, guarantees) and the
  `json` module in the module list.
- `docs/ADR.md` — `ADR-0028`, the envelope, pagination, and no-new-dependency decision.
- `docs/GLOSSARY.md` — **Envelope** and **Day total**.
- `docs/DEVELOPER-GUIDE.md` — repository layout gains `src/json.rs`, `src/web/api.rs`, and
  `assets/api/openapi.json`.

No `Cargo.toml` edit in the implementation: the version bump to `0.5.0` belongs to the close
step, not to the steps below.

## Architecture impact

The `web` component grows one submodule that owns the JSON contract; it reads the store through
the same read-only connection the pages use and writes nothing. No new thread, socket,
dependency, table, or configuration key appears, so the C4 views and the invariants of
`docs/ARCHITECTURE.md` stand; the document gains the `web::api` component contract.

## Decision-record impact

A new decision record is required — `ADR-0028`, *Expose a read-only JSON API under `/api/v1`
with an envelope*: an object root instead of an array, `meta` echoing the request, `limit`/
`offset` pagination, a hand-written encoder instead of a new crate, an OpenAPI document served
by the binary, `Access-Control-Allow-Origin: *` on a deliberately unauthenticated surface, and
— from the round-5 revision (Step 22) — the accepted instant domain as a fixed calendar window,
with the mobile ceiling anchored to `now` recorded as the rejected alternative.

## Proposed solution

### Step 1 — Extend the store filters and add the fault queries

In `src/store/mod.rs`:

- `EventFilter` gains `until_ms: Option<i64>` (exclusive) and `offset: Option<u32>`;
  `Store::events` adds `AND (?5 IS NULL OR occurred_at_ms < ?5)` and
  `LIMIT COALESCE(?4, -1) OFFSET COALESCE(?5, 0)`, keeping the existing order
  (`occurred_at_ms DESC, id DESC`). Update the two struct literals that name every field
  (`src/query/mod.rs`, the unit tests in `src/store/mod.rs`) and `src/query/mod.rs` passes
  `until_ms: None, offset: None`.
- `Fault` gains `interface: Option<String>`; `FAULTS_SELECT` selects `e.interface` and the
  three mappers (`recent_faults`, `recent_faults_for`, `faults_after`) read it.
- New `FaultFilter { target: Option<String>, from_ms: Option<i64>, to_ms: Option<i64>,
  limit: Option<u32>, offset: Option<u32> }` and two methods built on `FAULTS_SELECT`:
  `faults(&self, &FaultFilter) -> Result<Vec<Fault>, StoreError>` with
  `AND (?1 IS NULL OR e.target = ?1) AND (?2 IS NULL OR e.occurred_at_ms >= ?2)
  AND (?3 IS NULL OR e.occurred_at_ms < ?3) ORDER BY e.occurred_at_ms DESC, e.id DESC
  LIMIT COALESCE(?4, -1) OFFSET COALESCE(?5, 0)`, and
  `count_faults(&self, &FaultFilter) -> Result<i64, StoreError>` wrapping the same WHERE in
  `SELECT COUNT(*) FROM ( … )`.
- `Store::daily_drops(&self, target: Option<&str>, from_ms, to_ms)` filters with
  `(?1 IS NULL OR e.target = ?1)`; the two call sites pass `Some(&target.target)`.

Unit tests: `from`/`to` bounds and `offset` on `events`; `faults` filtered by target and range
with the newest first; `count_faults` ignoring `limit`/`offset`; `daily_drops(None, …)` summing
two targets into one bucket.

**Verify:** `cargo test --lib store && cargo test --test store && cargo test --test cli`

### Step 2 — Move the JSON primitives into their own module

Create `src/json.rs` with `pub fn string(&str) -> String`, `pub fn option_string(&Option<String>)`,
`pub fn option_i64(Option<i64>)`, and `pub fn array(Vec<String>) -> String`, moved verbatim from
`src/query/render.rs` (escaping rules included: `"`, `\`, `\n`, `\r`, `\t`, and every control
character as `\u00XX`). `src/query/render.rs` calls them, and `src/lib.rs` registers the module.
One escaper, two writers: the `query` output and the API must never disagree on how a cause
message is quoted.

**Verify:** `cargo test --test cli` (the `query --format json` assertions are unchanged) and
`cargo fmt --check`

### Step 3 — Add `web::api` and dispatch it from the server

Create `src/web/api.rs` and wire it into `src/web/mod.rs`.

Dispatch: after the request line is parsed, a `path` that starts with `/api/v1/` goes to
`api::handle(method, path, query, stream, database, include_body, gzip)`; everything else keeps
today's contract. `OPTIONS` is answered only for `/api/v1/*` (204, CORS headers); a method other
than `GET`/`HEAD` there is `405` with `Allow: GET, HEAD, OPTIONS`; outside `/api/v1` the answer
stays the plain-text `405`.

Parameters (`api::Params`): `target`, `from`, `to`, `limit`, `offset`.

| Parameter | Accepted | Default | Rejected with |
|---|---|---|---|
| `target` | canonical key or label, resolved with `Store::resolve_target` | every target | `404 UNKNOWN_TARGET` |
| `from` | RFC 3339 UTC (`time::parse_rfc3339`) or epoch milliseconds, inclusive | none on `/faults`; `now - 30 days` on `/faults/daily` | `400 INVALID_PARAMETER` |
| `to` | same forms, exclusive | none on `/faults`; `now` on `/faults/daily` | `400 INVALID_PARAMETER` |
| `limit` | integer `1..=1000` | `100` | `400 INVALID_PARAMETER` |
| `offset` | integer `>= 0` | `0` | `400 INVALID_PARAMETER` |

A `from` that is not earlier than `to` is `400 INVALID_RANGE`. A query parameter that is not
one of the five is `400 INVALID_PARAMETER` naming it, so a typo cannot be read as a default.

Routes and records (camelCase keys, UTC instants as RFC 3339 **and** the same instant in epoch
milliseconds, one property per fact):

- `GET /api/v1/targets` — `Store::states()`, ordered by key, sliced by `offset`/`limit` in
  memory (`total` is the length of the list). Record:
  `target`, `label`, `interface`, `state`, `stateSince`, `stateSinceMs`, `firstSeenAt`,
  `firstSeenAtMs`, `lastRttUs`, `upForMs` (`page::up_for`, `null` while not `up`).
- `GET /api/v1/faults` — `Store::faults` + `Store::count_faults`. Record: `id`, `target`,
  `label`, `interface`, `occurredAt`, `occurredAtMs`, `cause`, `causeMessage`, `endedAt`,
  `endedAtMs`, `durationMs` (`null` while the fault is open), `ongoing` (boolean, the inverse
  of an end). `state` and `rttUs` are deliberately absent: a fault opens on a `down` event,
  whose state is `down` and whose round-trip time is always `NULL`, so both fields would be
  constants.
- `GET /api/v1/faults/daily` — `Store::daily_drops(target, from, to)` for `faults` and
  `downMs`, and `Store::timeline(from, to)` for the `up` runs: per target
  `stats::up_runs_per_day(segments, DAY_MS)` merged day by day, and for the current UTC day
  `stats::up_runs_between(segments, now - DAY_MS, now)` of every target concatenated, exactly
  the rule the detail page applies. Record: `day` (`YYYY-MM-DD`), `dayStart`, `dayStartMs`,
  `faults`, `downMs`, `avgUpRunMs`, `longestUpRunMs` (both `null` on a day with no `up` run),
  `partial` (`true` only for the UTC day still running), and `target` — the resolved key, or
  `null` when the request named none and the series aggregates every target. Days are ordered
  oldest first, and the window ends 30 days before `to` when `from` is absent.
- `GET /api/v1/openapi.json` — the asset of Step 5.

Envelope: `{"data": [...], "meta": {...}}` on success, with `meta` =
`{"limit", "offset", "total", "filters": {"target", "from", "to"}, "generatedAt"}`; the
filters carry the **resolved** instants and the resolved target key, or `null` where the
request stated none. `/targets` states `filters` with three `null`s.

Error: `{"error": {"status", "code", "parameter", "message"}, "meta": {"generatedAt"}}` with
the codes `INVALID_PARAMETER` (400), `INVALID_RANGE` (400), `UNKNOWN_TARGET` (404),
`NOT_FOUND` (404, an unknown `/api/v1` path), `METHOD_NOT_ALLOWED` (405),
`DATABASE_UNAVAILABLE` (503, the database of `open_store`), `QUERY_FAILED` (500).

Headers on every `/api/v1` answer: `Content-Type: application/json; charset=utf-8`,
`Cache-Control: no-store`, `X-Content-Type-Options: nosniff`, `Access-Control-Allow-Origin: *`,
and `Vary: Accept-Encoding` when the body may be gzipped; `OPTIONS` adds
`Access-Control-Allow-Methods: GET, HEAD, OPTIONS` and `Access-Control-Max-Age: 600`. The gzip
writer of `respond_page` becomes a shared `respond_compressed(stream, status, content_type,
body, include_body, gzip)` used by both the page and the API, so `HEAD` states the compressed
`Content-Length` and sends no body in both cases.

**Verify:** `cargo build` plus the tests of Step 4

### Step 4 — Pin the HTTP contract with tests

Extend `tests/web.rs` (same `Server` and `seeded_database`):

- the body of each route starts with `{"data"` — an array is never the root — and `meta`
  carries `limit`, `offset`, `total`, `filters`, and `generatedAt`;
- `/targets` lists every seeded target with the ten properties, `limit=1&offset=1` returns the
  second target and `total` stays the full count;
- `/faults` returns faults newest first, `from` includes a fault at the exact instant, `to`
  excludes one at its exact instant, a `target` given as a label resolves to the same records as
  the canonical key, a page past the end returns `"data": []` with the same `total`;
- `/faults/daily` returns one record per UTC day oldest first, sums two targets into one day,
  marks the current day `partial`, and echoes the default 30-day window in `meta.filters`;
- the rejections: `limit=0`, `limit=1001`, `offset=-1`, `limit=abc`, `from=not-a-date`,
  `from` after `to`, `bogus=1`, `target=does-not-exist`, `/api/v1/nope`, and a `POST`;
- `OPTIONS /api/v1/targets` is 204 with the CORS headers, every API answer states
  `Access-Control-Allow-Origin: *`, and the gzip form is sent when the client advertises it;
- `HEAD /api/v1/targets` states the same `Content-Length` and sends no body.

**Verify:** `cargo test --test web`

### Step 5 — Publish the OpenAPI document

Write `assets/api/openapi.json` (OpenAPI 3.1): the server, the four paths with their
parameters and responses, the `Envelope`, `Meta`, `Error`, `Target`, `Fault`, and `DayTotal`
schemas, and one example per route. Embed it in `src/web/api.rs` with
`include_str!("../../assets/api/openapi.json")` and serve it at `/api/v1/openapi.json` as
`application/json; charset=utf-8` (same headers, no envelope, gzipped like the rest).

The document is hand-written, so the test is the guard that it does not drift: it must name the
three data paths, each must answer 200, and every property documented for a record must appear
as a key in the live answer of its route.

**Verify:** `cargo test --test web` and
`curl -s localhost:8642/api/v1/openapi.json | head -20`

### Step 6 — Document the contract

- `docs/PRODUCT.md`: `FR-26` states the four routes, the envelope, the filters, the pagination,
  the errors, and that the API is read-only, unauthenticated, and served on the page's socket;
  `BR-8` says the one socket serves the page **and** the API and still writes nothing.
- `docs/USER-GUIDE.md`: `## REST API` — one paragraph of purpose, the parameter table, an
  envelope example per route, the error shape, the pagination rule and its caveat (an
  `offset` page over a list that grows at the top can repeat a fault; use `from`/`to` for a
  stable window), the OpenAPI address, and the note that there is no authentication. The
  `--compression <mode>` row of `## Command line` now also names the JSON answers.
- `docs/ARCHITECTURE.md`: the `web::api` component (input: method, path, query, the read-only
  store; output: one JSON object per request plus the log lines of a failure; guarantees: no
  write, a bounded answer, no array root, one encoder).
- `docs/ADR.md`: `ADR-0028`.
- `docs/GLOSSARY.md`: **Envelope** and **Day total**.
- `docs/DEVELOPER-GUIDE.md`: the three new paths in the repository layout.

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` and an
inspection that every documented parameter and field exists in `src/web/api.rs`

### Step 7 — Read the faults in keyset pages

`FaultFilter` gains `after_ms: Option<i64>` and `after_id: Option<i64>`. `Store::faults` adds

```sql
AND (?6 IS NULL OR e.occurred_at_ms < ?6
     OR (e.occurred_at_ms = ?6 AND e.id < ?7))
```

after the existing bounds, so a caller walks the history newest first with the last row of the
previous page as its cursor: no `offset`, no row repeated when a new fault lands between two
reads. `count_faults` keeps ignoring the cursor.

Unit tests: a walk of three pages of two returns every fault exactly once, newest first; a row
inserted between two reads is not seen twice; the cursor stops the walk when the page is short.

**Verify:** `cargo test --lib store`

### Step 8 — Split `web/api` into one module per route

`src/web/api.rs` becomes `src/web/api/`. `mod.rs` keeps the contract — `handle`, `Reply`,
`Rejection`, `Params`, `Meta`, `respond_envelope`, `fail`, `reject`, `open`, `resolve`, the
streaming writer — and each route moves to its own file with its record builder:
`targets.rs` (`targets`, `target_record`), `faults.rs` (`faults`, `fault_record`), `daily.rs`
(`daily`, `daily_record`), `openapi.rs` (`OPENAPI`, the route). No behaviour changes.

**Verify:** `cargo build && cargo test --test web` (the same 35 tests, unchanged)

### Step 9 — Stream the answers

Each data route writes its answer as it reads, through one writer in `mod.rs`:

- `begin_stream` starts a `200` with the JSON content type, `Cache-Control: no-store`,
  `X-Content-Type-Options: nosniff`, `Connection: close`, the CORS header, `Vary:
  Accept-Encoding`, and `Transfer-Encoding: chunked` — the `Chunked` writer of `web::mod` is
  reused, so the mechanism is the one the CSV export already uses.
- The body is written in order: `{"data": [` then one rendered record per chunk, separated by
  commas, then `],"meta": {…}}`.
- When the client advertises gzip and `--compression auto` is in force, the chunks pass through
  a `flate2::write::GzEncoder` writing into the chunked stream, and the head states
  `Content-Encoding: gzip`; otherwise the bytes go out as they are. Nothing is buffered whole:
  the encoder holds its own window, the answer holds one page and one record.
- `/faults` loops the keyset pages of Step 7 (200 rows) until the requested `limit` is reached or
  a page comes back short, writing every record as it arrives, so a `limit=1000` answer holds at
  most 200 `Fault` values at a time.
- `/targets` and `/faults/daily` write from the slice they already have — the number of targets,
  and the days of the page, which `limit` bounds at 1 000. *(Corrected 2026-11-14: the plan said
  `limit` bounded the window; it does not, so the daily endpoint now reads only the days it
  returns — Step 14.)*
- `HEAD` on a data route states the same headers as `GET` and no length, because a streamed body
  has none: the existing test that compares `Content-Length` with the body length is replaced by
  one that asserts `Transfer-Encoding: chunked` and no `Content-Length`.
- A write that fails mid-answer is logged and stops the answer: the status is already sent, so
  the client sees a truncated chunked body rather than a second response.

**Verify:** `cargo test --test web`

### Step 10 — Harden the error bodies

`Rejection::failed` and the `503` of `open` state a fixed message — `query failed` and
`cannot read the event database` — and log the detail of the store error, which may name the
database path, nowhere else. The `404` of an unknown route states `no such route` and no longer
echoes the requested path. `parameter` still names the offending query parameter, which is a
closed set of five names.

**Verify:** `cargo test --test web` and an inspection that no `error.to_string()` reachable from a
response remains in `src/web/api/`

### Step 11 — Pin streaming and the hardened errors with tests

In `tests/web.rs`:

- a data route answers `Transfer-Encoding: chunked`, no `Content-Length`, and the de-chunked body
  is the document the envelope states (`data`, `meta`, `total`);
- a seeded history of 250 faults (`limit=250`) comes back whole across the keyset pages, with
  every id once and the meta total of 250;
- the gzip form is chunked too, states `Content-Encoding: gzip`, and decompresses to the same
  document;
- `HEAD` states no `Content-Length` and sends no body;
- a store failure answers the fixed message, and neither the database path nor a SQLite detail
  appears in the body;
- an unknown `/api/v1` route answers `no such route` without the requested path.

**Verify:** `cargo test --test web`

### Step 12 — Document streaming, the modules, and the error bodies

- `docs/PRODUCT.md` — `FR-26` states that the answers of the three data routes are streamed with
  `Transfer-Encoding: chunked` and compressed on the fly, that an error states a fixed message
  and never a database path or the request path.
- `docs/USER-GUIDE.md` — `## REST API` gains the streaming facts a client depends on (chunked,
  no `Content-Length`, `Content-Encoding: gzip` when accepted) and the fixed error messages.
- `docs/ARCHITECTURE.md` — `web::api` states the streaming writer, the keyset page size, and the
  bound of each route.
- `docs/ADR.md` — `ADR-0028` records streaming and the module split, with the rejected
  alternative of a buffered answer under the `limit` cap.

### Step 13 — State the target of a daily record

The user reported that `/api/v1/faults/daily` states no target, so a record cannot say what it is
about. Each record gains `target`: the canonical key the request resolved, or `null` when no
`target` was given and the answer is the aggregate series of every target. The 30-day default
window is confirmed as the contract, and the documentation states it next to the field, so a
reader who sees 31 records knows to pass `from` for an older history.

Files: `src/web/api/daily.rs` (the record), `assets/api/openapi.json` (`DayTotal`), `tests/web.rs`
(the documented keys and one test per shape), `docs/PRODUCT.md`, `docs/USER-GUIDE.md`,
`docs/ARCHITECTURE.md`.

**Verify:** `cargo test --test web`

### Step 14 — Read only the page of days the daily answer states

Review finding 1: the daily route built a bucket per day of the caller-chosen window and loaded
every timeline segment in it, so an unauthenticated request could make the process work in
proportion to the whole history. The user chose to keep the window unbounded and bound the work
instead.

`Page` in `src/web/api/daily.rs` computes the contiguous day range the request answers —
`offset`/`limit` applied to the day count of the window — and the route reads
`Store::daily_drops` and `Store::timeline` for that range only, starting the timeline at
`min(page_from, now - 1 day)` so the rolling window of the UTC day still running is covered. The
window may be any length; the memory and the queries follow `limit`, which the API caps at
1 000. `total` stays the day count of the whole window, computed arithmetically.

Two tests pin it: a ten-day window answers ten days with paging that starts on the requested day,
and a window since 1970 states a total beyond 20 000 while returning exactly `limit` records
starting at 1970-01-01. Measured on 20 000 events over two years, `from=1970&limit=100` went from
0.11 s / 196 KB / RSS 10→13 MB to 46 ms / 19.7 KB / RSS 10 MB.

**Verify:** `cargo test --test web`

### Step 15 — State the day boundaries of the daily answer once, and clip to them

Review round 3: `Page::of` builds the page from the day start (`first_day + skip * DAY_MS`) and
hands it to `Store::daily_drops` and `Store::timeline`, so the first day counts down time and
`up` runs from **before** the requested `from`. With a target down 09:00→11:00 on one UTC day,
`from=10:00` reports `downMs` 7 200 000 instead of 3 600 000, contradicting the contract the
OpenAPI and the user guide already state.

This step replaces the ad-hoc arithmetic of Step 14 with one stated rule and one function that
implements it. All of it lives in `src/web/api/daily.rs`.

**The rule, in one place in the code's documentation comment:**

1. The days of a window are the UTC days that contain `[from, to)`: the first is the day that
   contains `from`, the last is the day that contains `to - 1 ms`, and the count is
   `day_count(floor(from), to)`.
2. `offset` and `limit` select a contiguous slice of that list, `[skip, skip + take)`.
3. Every read is clipped to the requested window: the read range is
   `[max(day_start(skip), from), min(day_start(skip + take), to))`. A day at either edge therefore
   counts only the part that falls inside `[from, to)`.
4. The timeline starts at `min(read_from, now - 1 day)`, so the UTC day still running states the
   last 24 hours even when the page holds only that day.
5. A page past the end states `data: []` and `total` = the day count of the whole window; the
   reads are skipped in that case.

`Page` carries `from_ms`/`to_ms` already clipped by rule 3, `total` and `take`, so the route
never re-derives a boundary. Nothing else in the route changes.

Tests to add in `tests/web.rs`, one per edge, each with a fixed UTC day so it never depends on
when it runs:

- **first day clipped by a mid-day `from`**: a target down 09:00→11:00 on one day, asked with
  `from=10:00`, states `downMs` 3 600 000 and an `avgUpRunMs` that starts at 10:00, not 7 200 000
  / the whole day;
- **empty page past the end**: a ten-day window with `offset=1000` states `data: []` and
  `total: 10`;
- **wide window, narrow page**: a window since 1970 with `limit=5` states a total above 20 000 and
  exactly five records starting at 1970-01-01;
- **current UTC day**: a window ending now whose last day holds an `up` run of two hours and a
  `down` beginning an hour ago states the `up` figure of the last 24 hours and `partial: true`.

**Verify:** `cargo test --test web`, and the four probes of the "Manual" line of the verification
plan against a running `serve`

### Step 16 — Record the daily rules in the documents that already promise them

No new behaviour: the documents must state the rules Step 15 implements, in the same words, so
the next reader does not have to infer them from the code.

- `docs/USER-GUIDE.md` — `## REST API`, the paragraph on `/api/v1/faults/daily`: the window is
  clipped at its edges, and the UTC day still running states the last 24 hours.
- `assets/api/openapi.json` — the `/api/v1/faults/daily` operation description already says the
  edge days are clipped; add the sentence that the day still running states the last 24 hours and
  is flagged `partial`.
- `docs/ARCHITECTURE.md` — the `web::api` guarantees already state the split runs and the
  running day; name the clipping explicitly and point at the rule in `daily.rs`.

**Verify:** inspection against `src/web/api/daily.rs` and `cargo test --test web`


### Step 17 — Accept every instant that fits an integer, and overflow nowhere

Review round 4 (blocker): two expressions panic on extreme but syntactically valid instants —
`window_from_ms.div_euclid(DAY_MS) * DAY_MS` in `src/web/api/daily.rs` overflows at
`i64::MIN`, and `day += DAY_MS` in `Store::daily_drops` overflows near `i64::MAX`.

The accepted range is decided and needs no new bound: **any value that fits a signed 64-bit
integer**, which is exactly what `Params::parse_instant` already enforces by parsing `i64` —
a value beyond `i64::MIN`/`i64::MAX` fails the parse and is `400 INVALID_PARAMETER` today.
What must change is the arithmetic inside that range:

- `src/web/api/daily.rs`, `Page::of` — `first_day` is computed as
  `window_from_ms.div_euclid(DAY_MS).saturating_mul(DAY_MS)`, so no input can panic; the
  `saturating_add` already on the page start and end stays.
- `src/store/mod.rs`, `Store::daily_drops` — the bucket walk uses `saturating_add`, so a
  window whose last day sits at the top of the range ends instead of panicking. The day count
  already goes through `i128` in `Page::of`.
- `src/time.rs` — no change: `format_rfc3339` is only ever handed a day start (about
  ±1.07e11 days), not an `i64` extreme.

Saturation is defence in depth, not the contract: inside the accepted range the answer is a
well-formed envelope whose days are the ones the arithmetic yields; outside it the parse
already answers `400`.

Tests in `tests/web.rs`:

- `from=-9223372036854775808` and `to=9223372036854775807` each answer `200` with a body that
  parses as the envelope (`data`, `meta`) — a panic would drop the connection instead, so the
  `200` is the proof;
- `from=9223372036854775808` (one above `i64::MAX`) answers `400 INVALID_PARAMETER` naming
  `from`;
- the four day-boundary tests of Step 15 stay green.

**Verify:** `cargo test --test web`, and a manual `curl` of the three inputs above

### Step 18 — Make a panicking handler cost one request, never a slot

Review round 4 (blocker): `web/mod.rs` increments `active` before spawning the worker and
decrements it after `handle` returns, so a panic skips the decrement. The counter is the
`MAX_CONNECTIONS` (32) gate, so 32 panicking requests leave the server answering nothing —
page included — until restart. Reproduced: `/healthz` 200, 40 craft requests, `/healthz` and
`/` silent, 32 panic lines, process alive.

`src/web/mod.rs`:

- Add a small `Slot` guard: `Slot::enter(active: &Arc<AtomicUsize>)` increments and returns a
  value whose `Drop` decrements. The worker closure holds it for the whole of `handle`, so an
  unwind runs the decrement on the way out.
- Keep the behaviour otherwise identical: the same 32 limit, the same `drop(stream)` when the
  limit is reached.
- Nothing responds `500` after a panic: the status line may already be on the wire, so the
  contract stays "a panicking request loses its answer, not its slot".

Tests:

- `src/web/mod.rs`, unit: entering the guard increments and dropping decrements, including
  when the drop happens during a `catch_unwind` unwind;
- `tests/web.rs`, integration: 40 requests with the extreme `from` of Step 17 (which, after
  Step 17, answer `200`), then `/healthz` and `/` still answer `200`; and a second variant
  that asserts `MAX_CONNECTIONS + 8` sequential requests never leave the server silent.

**Verify:** `cargo test --lib web && cargo test --test web`

### Step 19 — State the accepted range and the failure contract

- `docs/USER-GUIDE.md`, `## REST API`: `from` and `to` accept any epoch-millisecond value that
  fits a signed 64-bit integer, or an RFC 3339 instant; a value outside that range is
  `400 INVALID_PARAMETER`. Add one sentence on what a client sees when the server fails inside
  a request: the connection closes with no answer, and the next request is served normally.
- `assets/api/openapi.json`: the `From`/`To` parameter descriptions state the accepted range.
- `docs/ARCHITECTURE.md`, `web` component guarantees: the connection counter is released even
  when a handler unwinds, so a failing request never consumes a slot.
- `docs/ADR.md`, `ADR-0028` consequences: the accepted instant range and the panic-safety of
  the connection accounting are part of the contract, with the DoS that motivated them.

**Verify:** inspection against `src/web/api/mod.rs`, `src/web/mod.rs`, and `src/store/mod.rs`,
plus `cargo test`

### Step 20 — Accept a fixed calendar domain, validated once at the boundary

Review round 5 (blocker): Step 17 promised "any value that fits a signed 64-bit integer" and
kept that promise in two expressions, while three others in the same call chain still break —
and the failure is worse than the one round 4 found. Measured on the `release-dev` build with a
single unauthenticated request,
`GET /api/v1/faults/daily?from=9223372036854000000&to=9223372036854775807&limit=1000`:
`src/web/stats.rs:72` adds `DAY_MS` to a day start that has no room, `release` wraps it
negative, the cursor moves backwards and the day walk never ends, pushing one `BTreeMap` entry
per pass — RSS 3 MB → 1.9 GB at 2 s → 7.9 GB at 5 s. `src/store/mod.rs:210` (`day_slices`)
panics in `debug` and `src/web/api/daily.rs:25` (`to - DEFAULT_DAYS * DAY_MS`) underflows when
the request states only a low `to`.

The root cause is the domain, not the formulas: epoch milliseconds in `i64` span ±292 million
years, while every step of the daily route is calendar arithmetic over that span. A ceiling
anchored to the clock does not work either — on 15 November, "this month" asks for
`to = 2026-12-01` and "this year" for `to = 2027-01-01`, both beyond `now + 1 day`, and a period
boundary in the future is how everyone writes a window.

**The domain is a fixed calendar window, not a mobile one:**

- `DOMAIN_MIN_MS: i64 = 0` — `1970-01-01T00:00:00Z`.
- `DOMAIN_MAX_MS: i64 = 4_102_444_800_000` — `2100-01-01T00:00:00Z`.
- An instant is accepted when `DOMAIN_MIN_MS <= instant <= DOMAIN_MAX_MS`; anything else is
  `400 INVALID_PARAMETER` naming the parameter that carried it.

Why this bounds everything at once: `2100-01-01` is 4.1e12 ms against an `i64` that reaches
9.2e18, so `day + DAY_MS` has 2.2 million times the room it needs and **no expression of the
daily path can overflow**; the whole domain holds 47 482 days, so `meta.total` states a number
that means something instead of 2.1e11; the future stays queryable, so a month or a year that
has begun can be asked for whole; and the contract stops depending on the process clock, so
two identical requests answer identically and no test depends on the hour it runs at.

Implementation:

- `src/web/api/mod.rs` — the two constants next to `DEFAULT_LIMIT`/`MAX_LIMIT`, documented with
  the dates they are; `parse_instant` applies the check **after** both parse paths, so an
  out-of-domain RFC 3339 instant and an out-of-domain integer are refused the same way. The
  message is one fixed sentence naming the parameter and both bounds, built from the constants
  with `time::format_rfc3339` so the text cannot drift from the values:
  `from must be an RFC 3339 UTC instant between 1970-01-01T00:00:00.000Z and
  2100-01-01T00:00:00.000Z, or epoch milliseconds in that range`. `parse_instant` serves `from`
  and `to` of both `faults` and `daily`, so one check covers every route.
- `src/web/api/daily.rs` — the resolved window is computed after the check and stays inside the
  domain: `let now = now_ms().clamp(DOMAIN_MIN_MS, DOMAIN_MAX_MS);` (a machine clock outside the
  domain must not turn a valid request into a refusal), and
  `let from = params.from.unwrap_or(to.saturating_sub(DEFAULT_DAYS * DAY_MS).max(DOMAIN_MIN_MS));`
  followed by the existing `from >= to` → `INVALID_RANGE`. A request that names only
  `to=1970-01-01T00:00:00Z` therefore answers `400 INVALID_RANGE` — an empty window, stated as
  such, instead of an underflowed `from`.
- `tests/web.rs` — **`an_instant_at_the_end_of_the_range_answers_an_envelope` is legitimately
  replaced**, not weakened: it asserted `200` for inputs that must now be refused, and it is the
  test that let the defects through because its database held no `up` run at the extremes. The
  permission to change it is recorded here. Its replacement is
  `an_instant_outside_the_accepted_domain_is_refused`, and
  `an_instant_beyond_the_integer_range_is_refused` stays as it is.

Tests in `tests/web.rs`:

- `from=-1`, `to=4102444800001`, `from=-9223372036854775808`, `to=9223372036854775807`, and
  `to=-9223372036854775800` each answer `400` with `"code": "INVALID_PARAMETER"` and the
  parameter name — five shapes, one of which (`to` alone, low) is the underflow of
  `daily.rs:25`;
- `from=1970-01-01T00:00:00Z` and `to=2100-01-01T00:00:00Z` are accepted, and so is the pair;
- **the natural period queries of the user**: on a database seeded around the current day,
  `from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z` answers `200`, `"total": 30`, and 30
  records whose last day is in the future and states `faults: 0`, `downMs: 0`,
  `partial: false`; `from=2026-01-01T00:00:00Z&to=2027-01-01T00:00:00Z` answers `200` with
  `"total": 365` and the same empty shape for the days after today;
- `to=1970-01-01T00:00:00Z` alone answers `400 INVALID_RANGE`;
- the four day-boundary tests of Step 15 and the round-2 `offset`-past-the-end test stay green.

Unit test in `src/web/api/mod.rs`: the two constants format to `1970-01-01T00:00:00.000Z` and
`2100-01-01T00:00:00.000Z`, so a wrong literal fails a test instead of reaching the guide.

**Verify:** `cargo test --test web` and `cargo test --lib web`; then, against a seeded database
and the `target/debug` binary, `curl -s -o /dev/null -w '%{http_code}\n'` on
`/api/v1/faults/daily?to=9223372036854775807` → `400`, on
`/api/v1/faults/daily?from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z` → `200`, and on
`/api/v1/faults/daily?to=1970-01-01T00:00:00Z` → `400`.

### Step 21 — Make the two day walks advance by construction, and prove it

The domain of Step 20 bounds the API, but `up_runs_per_day` and `day_slices` are library
functions with no such guard: `debug` panics and `release` wraps into a cursor that moves
backwards. The fix is structural, so a future caller cannot reintroduce the loop, and it is
tested at the edges rather than only through HTTP.

- `src/web/stats.rs`, `up_runs_per_day` — `let day = cursor.div_euclid(day_ms).saturating_mul(day_ms);`
  and `let end = day.saturating_add(day_ms).min(to_ms);`, then
  `if end <= cursor { break; }` before pushing. The loop can no longer fail to advance, whatever
  the arithmetic yields.
- `src/store/mod.rs`, `day_slices` — the same three changes (`saturating_mul`,
  `saturating_add`, `if next <= cursor { break; }`). `Store::daily_drops` already saturates its
  bucket walks from Step 17 and keeps them.
- The work stays proportional to the span, which the domain bounds for the API and the page's
  own window bounds for the page: this step guarantees termination, not a constant cost, and
  the tests reflect that by using edge-local ranges rather than the whole range.

Tests:

- `src/web/stats.rs`, unit: a segment spanning `[i64::MAX - 3 * DAY_MS, i64::MAX]` yields the
  slices of those days and returns; a segment spanning `[i64::MIN, i64::MIN + 3 * DAY_MS]`
  likewise; every returned day length is positive.
- `src/store/mod.rs`, unit: `day_slices(i64::MIN, i64::MAX, i64::MIN, i64::MIN + 5 * DAY_MS)`
  returns in bounded time with five positive slices, and `day_slices(0, i64::MAX,
  i64::MAX - 5 * DAY_MS, i64::MAX)` returns the last five days.
- `tests/web.rs` — after Step 20 no request can reach those edges, so the guard is proven by the
  unit tests; the integration suite must stay green and must not grow a slow test.

**Verify:** `cargo test --lib web && cargo test --lib store`, then `cargo test --test web`.

### Step 22 — State the domain in the four documents that promise the filters

The documents currently promise the `i64` range that Step 20 replaces; every one of them is
wrong the moment the check lands, so they change in the same commit.

- `docs/USER-GUIDE.md`, `## REST API` — replace the paragraph under the parameter table with the
  domain: an instant lies between `1970-01-01T00:00:00Z` and `2100-01-01T00:00:00Z`, both bounds
  accepted, in either form; a value outside is `400 INVALID_PARAMETER`; a `to` in the future is
  valid and the days it covers answer zero until the day arrives. Correct the `from` row of the
  table (`faults` no longer has "no bound on faults"; the accepted domain replaces it) and keep
  the failure-contract paragraph added in Step 19. One worked example: `this month`, as
  `from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z`.
- `assets/api/openapi.json` — the `From`/`To` descriptions state the same domain; the document
  stays valid JSON and the drift guard of `tests/web.rs` stays green.
- `docs/PRODUCT.md`, `FR-26` — one clause: `from` and `to` are accepted inside a fixed calendar
  domain and a value outside it is `400`, naming the parameter.
- `docs/ADR.md`, `ADR-0028` — decision 12 is rewritten: the accepted domain is the fixed window
  `1970-01-01 … 2100-01-01`, chosen because calendar arithmetic cannot cover `i64` and because
  the period boundaries people actually ask for lie in the future; the rejected alternative is
  recorded (a ceiling anchored to `now`, refused because it rejects "this month"); and the
  consequence states that a future caller passing a wider span to `up_runs_per_day` or
  `day_slices` still walks that span, which is why Step 21 makes the walk terminate rather than
  rely on the caller.
- `docs/ARCHITECTURE.md`, `web::api` guarantees — the domain replaces "any value that fits a
  signed 64-bit integer", and the `web`/`stats` note states that both day walks advance by
  construction.

**Verify:** inspection of each document against `src/web/api/mod.rs` (the two constants and
`parse_instant`) and `src/web/stats.rs`; `python3 -c "import json;json.load(open('assets/api/openapi.json'))"`;
`cargo test` green; `grep -rn "64-bit" docs/ assets/` returns nothing.

## Security considerations

- **Exposure.** The API adds no socket: it answers on the address the page already binds, and
  only when `--listen` is given (`BR-8`). It exposes the same history the page does, to the same
  unauthenticated audience, and it cannot write.
- **Input.** Every parameter is validated against a closed set and bound as a SQL parameter;
  `limit` is capped at 1000, so one request cannot ask for an unbounded answer. A parameter
  that is not recognised is refused rather than ignored.
- **CORS.** `Access-Control-Allow-Origin: *` is deliberate: the data is already public on the
  same host, and a wildcard lets a dashboard on another origin read it. It grants no write and
  no credential, and no cookie or authorization header is read, so the wildcard cannot expose a
  session.
- **Secrets.** The bot token of `FR-25` never reaches the API: the records carry the chat-free
  history of targets and faults only, and `--show-config` remains the only renderer of the
  configuration.
- **Residual risk.** An operator who does not want the API exposed must not pass `--listen`
  (today's rule for the page). This is stated in the user guide rather than solved with a flag.
- **Denial of service (review rounds 4 and 5, blocker).** Round 4: an extreme instant panicked a
  handler, and a panic skipped the connection-slot decrement, so 32 unauthenticated requests made
  the page and the API deaf until restart. Step 18 fixed the accounting — a slot is released on
  unwind, verified with 80 panicking requests — but round 5 showed the deeper half was still
  open: with an `up` run ending at the top of `i64`, `up_runs_per_day` wraps in `release`, the
  cursor moves backwards and one request hangs a worker while its memory grows to gigabytes
  (measured 7.9 GB in 5 s), and 33 such requests leave the surface silent.
- **Remediation (Steps 20 and 21).** The accepted instants become a fixed calendar domain
  (`1970-01-01 … 2100-01-01`), validated once in `parse_instant`, so no request can reach an
  expression that overflows and the largest window a client can ask for holds 47 482 days;
  both day walks (`up_runs_per_day`, `day_slices`) advance by construction, so a caller outside
  the API cannot loop either.
- **Residual risk.** A client that opens 32 slow requests still delays the others, which is the
  pre-existing behaviour of the page server and is documented. A library caller that passes a
  span wider than the domain to `up_runs_per_day` or `day_slices` still walks that span day by
  day: termination is guaranteed, the cost is proportional to the span, and no path of this
  binary does it. The domain is a pair of constants; widening them to `9999-12-31` would keep
  the arithmetic safe but raise the largest window to about 2.9 million days.

## Verification plan

- **Automated:** the store unit tests of Step 1, the HTTP contract tests of Step 4, and the
  OpenAPI drift guard of Step 5.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  (the `quality.all` command of `.spec-workflow.yml`).
- **Manual:** a mid-day `from` on a seeded day, an `offset` past the end, a `limit` smaller than a
  wide window, and the current UTC day, each compared with the rule of Step 15; the domain refusals
  of Step 20 (`from=-1`, `to=4102444800001`, `from=-9223372036854775808`, `to=9223372036854775807`,
  `to=-9223372036854775800`) each answering `400 INVALID_PARAMETER`, and `from=1970-01-01T00:00:00Z`
  / `to=2100-01-01T00:00:00Z` answering `200`; the natural period queries answering whole periods
  (`this month` → `total` 30, `this year` → `total` 365, the future days stating zero); and 40
  sequential requests with a valid maximum-width window followed by `/healthz` and `/` still
  answering `200`, run against **the `release` profile as well as `debug`**, because a `debug`
  panic and a `release` hang are different failures and only the second one ships. Then start
  `./target/debug/uptime-monitor serve --database <file> --listen 127.0.0.1:8642`
  and run `curl -s '127.0.0.1:8642/api/v1/targets?limit=2'`,
  `curl -s '127.0.0.1:8642/api/v1/faults?target=icmp://1.1.1.1&from=2026-11-01T00:00:00Z'`,
  `curl -s '127.0.0.1:8642/api/v1/faults/daily?from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z'`,
  `curl -si -X OPTIONS 127.0.0.1:8642/api/v1/targets`,
  `curl -si -X POST 127.0.0.1:8642/api/v1/targets`, and
  `curl -s 127.0.0.1:8642/api/v1/openapi.json`; each answer is checked against the plan.
- **Resource envelope:** the widest legal daily window (`from=1970-01-01T00:00:00Z&
  to=2100-01-01T00:00:00Z&limit=1000`) answers in well under a second with RSS unchanged, because
  the route reads only the page it returns.
- **Rollback:** revert the commit. Nothing is migrated, written, or configured, so an older
  binary reads the same database and serves the same pages; only the `/api/v1` paths disappear.

## Open questions

None. Every contract decision is stated above; the deviations from the interview and the two
decisions of the round-5 revision are recorded here so the implementer does not re-open them:

- [x] A fault record omits `state` and `rttUs` (both constant by construction) and carries
      `interface` instead — confirmed by the user before the plan was written.
- [x] The UTC day still running states its average `up` run over the last 24 hours, like the
      detail page, and is flagged `partial: true` — mirrors `FR-17` rather than inventing a
      second definition.
- [x] **The accepted instant domain is a fixed calendar window, `1970-01-01T00:00:00Z` …
      `2100-01-01T00:00:00Z`**, not the full `i64` range and not a ceiling anchored to `now`
      (Step 20). *Resolved 2026-11-15 with the user:* the full range cannot be covered by
      calendar arithmetic, and a mobile ceiling refuses "this month" and "this year" — on
      15 November those ask for `to = 2026-12-01` and `to = 2027-01-01`, both beyond
      `now + 1 day`. `2100-01-01` is 4.1e12 ms against `i64::MAX` 9.2e18, so nothing can
      overflow, and the whole domain holds 47 482 days, so `meta.total` stays meaningful.
- [x] **A `to` in the future is valid**, and the days after today answer `0` faults and `0`
      downMs with `partial: false`; only the UTC day still running is `partial`. *Resolved
      2026-11-15:* it is the only way a client can ask for a whole month or year.
- [x] **Out-of-domain is `400 INVALID_PARAMETER`, not `INVALID_RANGE`.** *Resolved
      2026-11-15:* the value the parameter carries is not acceptable, which is what
      `INVALID_PARAMETER` already means for `limit`; `INVALID_RANGE` stays for a `from` that is
      not earlier than `to`.

## Implementation checklist

- [x] Step 1 — Extend the store filters and add the fault queries
- [x] Step 2 — Move the JSON primitives into their own module
- [x] Step 3 — Add `web::api` and dispatch it from the server
- [x] Step 4 — Pin the HTTP contract with tests
- [x] Step 5 — Publish the OpenAPI document
- [x] Step 6 — Document the contract
- [x] Step 7 — Read the faults in keyset pages
- [x] Step 8 — Split `web/api` into one module per route
- [x] Step 9 — Stream the answers
- [x] Step 10 — Harden the error bodies
- [x] Step 11 — Pin streaming and the hardened errors with tests
- [x] Step 12 — Document streaming, the modules, and the error bodies
- [x] Step 13 — State the target of a daily record
- [x] Step 14 — Read only the page of days the daily answer states
- [x] Step 15 — State the day boundaries of the daily answer once, and clip to them
- [x] Step 16 — Record the daily rules in the documents that already promise them
- [x] Step 17 — Accept every instant that fits an integer, and overflow nowhere
- [x] Step 18 — Make a panicking handler cost one request, never a slot
- [x] Step 19 — State the accepted range and the failure contract
- [x] Step 20 — Accept a fixed calendar domain, validated once at the boundary
- [x] Step 21 — Make the two day walks advance by construction, and prove it
- [x] Step 22 — State the domain in the four documents that promise the filters
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

### Step 1 — store filters and fault queries

- **Decision.** The four copies of the `Fault` row mapper (`recent_faults`,
  `recent_faults_for`, `faults_after`, and the new `faults`) collapsed into one private
  `Store::fault(row)`: adding the `interface` column would otherwise have been four identical
  edits and a fifth tomorrow.
- **Deviation (test fixtures only).** `Fault` gained a field, so the literals in the test
  modules of `src/web/page.rs`, `src/web/detail.rs`, and `src/store/mod.rs` state
  `interface: None`. No assertion was changed or removed.
- **Decision.** The `interface` column is appended after the two cause columns, so no
  existing positional read shifts; `CAUSE_COLUMNS` and the legacy `NULL` substitution are
  untouched.
- **Discovery.** `daily_drops` opened on a writer-only view of one target; it now takes
  `Option<&str>` and the detail page passes `Some(&target.target)`. The SQL already aggregated
  in Rust, so `None` needed only `(?1 IS NULL OR e.target = ?1)`.
### Step 2 — JSON primitives

- **Decision.** `src/json.rs` also carries three unit tests for the escaping and the null
  forms, because the API now depends on this text and the `query` renderer no longer owns it.
- **Deviation.** None: the four helpers moved verbatim, and the `query` output is byte for
  byte the same (the `query --format json` assertions of `tests/cli.rs` are untouched).
### Step 3 — `web::api` and the dispatch

- **Discovery.** The server computed `include_body` as `method == "GET"`, which is right for
  `HEAD` and wrong for a refused `POST`: the `405` stated a `Content-Length` and wrote no
  body. It is now `method != "HEAD"`. The page routes only see `GET` and `HEAD`, so their
  behaviour is unchanged.
- **Decision.** `Reply<'a>` groups `include_body`, `gzip`, and the extra headers, so the
  handlers stay under the argument-count lint and the CORS header is stated once.
- **Decision.** Each route declares the parameters it accepts and anything else is a `400`
  naming it, so `/api/v1/targets?target=…` is refused instead of silently ignored.
- **Deviation.** `/api/v1/openapi.json` is not routed in this step: the asset arrives in
  Step 5, and `include_str!` cannot read a file that does not exist. Until then the path
  answers `404 NOT_FOUND` like any unknown route.

### Step 4 — the HTTP contract

- **Decision.** The tests live in `tests/web.rs` and reuse `Server` and `seeded_database`, as
  the plan said; the helper `Server::json` returns the body with the headers stripped, and
  `field_number` reads one rendered integer, because the project has no JSON parser.
- **Decision.** The bound assertions read the instant from the first answer instead of
  computing it: the seed is relative to `now`, so an absolute bound would be flaky.
- **Deviation.** The gzip test compares the document structurally (`data`, `total`) instead
  of byte for byte: two requests are two answers, and `generatedAt` and `upForMs` differ by
  design.
### Step 5 — the OpenAPI document

- **Decision.** The document uses full paths (`/api/v1/targets`) with `servers: [{"url": "/"}]`,
  so a reader can paste a path into `curl` and the drift test can look for the same string the
  route answers.
- **Decision.** The drift test keeps the expected property names in the test file and checks
  both directions — the document states each one and the live answer renders it — because the
  project ships no JSON parser to read either back.
- **Discovery.** `assets/api/openapi.json` is embedded with `include_str!`, like the icon and
  the templates, so the published contract travels inside the single executable.
### Step 6 — documentation

- **Decision.** `FR-26` states the whole contract in one requirement, the way `FR-17` states the
  page; `BR-8` now names the API as part of the one read-only socket, so the business rule and
  the implementation cannot drift apart.
- **Decision.** The user guide gains a `REST API` section with the parameter table, one envelope
  per shape, the error codes, the paging caveat, and three `curl` lines; the `--compression` row
  now mentions the JSON answers, because that flag governs them too.
- **Decision.** `ADR-0028` records the envelope, the pagination, the no-new-crate choice, the
  OpenAPI document, the omission of constant fields, and the CORS wildcard, with the rejected
  alternatives.
- **Manual verification.** A history imported from CSV served by
  `serve --database api.db --listen 127.0.0.1:18642` answered `/api/v1/targets`,
  `/api/v1/faults?limit=1`, `/api/v1/faults/daily?from=…&to=…`, `/api/v1/openapi.json`, `400`
  for `limit=0`, `404` for an unknown target, and `404` for an unknown route, each in the shape
  this plan states.
### Step 7 — keyset pages

- **Decision.** The cursor rides on `FaultFilter` (`after_ms`, `after_id`) instead of a second
  type: the filter already carries every other bound, and `count_faults` simply ignores the two
  new fields.
- **Decision.** `offset` stays in the filter even though the API will not use it: the store
  contract is public and the page/export paths may still need it, and the cursor is an
  additional bound, not a replacement.
### Step 8 — one module per route

- **Decision.** The split was done mechanically and checked by the unchanged suite: `mod.rs` holds
  `handle`, `Reply`, `Rejection`, `Params`, `Meta` plus the writers, and each route file states its
  own record builder. The shared items are `pub(super)`, so the route modules see the contract and
  nothing else of the API leaks into `web`.
- **Discovery.** `include_str!` resolves against the directory of the file that contains it, so the
  OpenAPI path gained one level when the module became a directory: `../../../assets/api/openapi.json`.
### Step 9 — streamed answers

- **Decision.** The envelope is written by `Envelope` in `api/mod.rs`: `begin` writes the head
  and returns a writer that is either a `HEAD` sink, the `Chunked` writer of `web::mod`, or a
  `GzEncoder` over it. `record` opens or continues `data`, `finish` closes the array and writes
  `meta`. The response format is stated in one place, so the three routes cannot drift.
- **Decision.** The chunked head is written by the same `begin_chunked` the CSV export uses,
  with `CORS`, `Vary`, and — when compressing — `Content-Encoding: gzip` passed as its extra
  headers. No second HTTP writer was introduced.
- **Decision.** `HEAD` goes through `Envelope::begin` with `Body::Head`: it writes the same head
  and never a chunk, so a `HEAD` states `Transfer-Encoding: chunked` and no length, which is the
  truth for a body that has none.
- **Decision.** The first keyset read honours the request's `offset`, every later one continues
  from its cursor, so `limit`/`offset` keep working while the memory stays one page.
- **Deviation.** The plan said a write failure leaves a truncated chunked body. It does: the
  terminating chunk is written only after `Envelope::finish` succeeded, and `truncated` logs the
  reason and returns.
- **Discovery.** The test harness read a body by splitting the headers, which broke on chunk
  framing; `Server::json` now de-chunks, and the gzip test de-chunks before decompressing.
- **Manual verification.** 250 imported transitions (125 faults) answered `Transfer-Encoding:
  chunked`, no `Content-Length`, 125 records with 125 distinct ids in descending order; the gzip
  form decompressed to the same document; `HEAD` stated `chunked`, no length, and no body.
### Step 10 — hardened error bodies

- **Decision.** The store detail moved into `Rejection::detail`, stated only by the log line of
  `reject`. `message` is now the fixed `query failed`, `cannot read the event database`, or
  `no such route`; a `grep error.to_string()` over `src/web/api/` finds it in one place, the
  constructor, and nowhere near a response.
- **Decision.** `UNKNOWN_TARGET` still names the requested *parameter value*, which the client
  already sent and which is JSON-escaped; only the request **path** stopped being echoed.

### Step 11 — the streaming and hardening tests

- **Decision.** The multi-page test seeds 250 faults so the 200-row keyset page must be crossed;
  it also asks for the last ten with `offset` to prove that the request's paging still works on
  top of the cursor.
- **Discovery.** `a_missing_database_is_reported_per_request` already removes the file after the
  server starts, which is exactly the way to reach the `503` of the API; the new test reuses it.
### Step 12 — documentation

- **Decision.** `FR-26` now states streaming, the keyset pages, and the fixed error text in the
  same requirement, so the product contract and the API cannot drift on any of the three.
- **Decision.** The served OpenAPI document states the streaming too, in its `info.description`:
  it is the contract a client reads first, and a `Transfer-Encoding` it does not expect is a bug
  report waiting to happen.
- **Extension (approved by the change).** `docs/DEVELOPER-GUIDE.md` lists the new `web/api/`
  directory, because the split of Step 8 moved the file the layout still named `api.rs`.
### Step 13 — the target of a daily record

- **Decision (user).** One series with a nullable `target`, not one record per target and day: the
  aggregate stays one series, and a filtered answer names the key on every record.
- **Decision (user).** The 30-day default window stays. The count the user saw — 31 records for
  `limit=1000` — is the window, not the limit; the user guide and the served OpenAPI now say so
  beside the field, and the `curl` examples show `from` for an older history.
- **Discovery.** `meta.filters.from` already echoed the applied window, so the truncation was
  visible in the answer; only the record was silent about its target.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(api): add the read-only JSON API under /api/v1`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/web/api/daily.rs` | The daily window has no upper bound: `from` is caller-controlled and the route builds a bucket per day of the window and loads every timeline segment in it (`Store::timeline`), so the work and the memory of one unauthenticated request grow with the history. Measured on 20 000 events over two years: `from=1970-01-01` → 0.11 s, 196 KB, RSS 10→13 MB against 6.5 KB and 10 MB for the default window. The plan states the window is "bounded by the days of the window, which `limit` caps at 1 000"; `limit` caps the records, not the window. The page bounds the same computation (`MAX_SPAN_MS` 30 days, and 360 days at most on `/target?days=…`). | open |
| 2 | minor | `README.md` | The API was absent from the quick start and from the "Status" feature list. | fixed in review |
| 3 | minor | `src/web/mod.rs` | `/api/v1` (no trailing slash) answered the page's `text/plain` 404 while `/api/v1/…` answered the JSON envelope. Now `path == "/api/v1"` is dispatched to the API: 404 `NOT_FOUND`, `no such route`. | fixed in review (permission granted) |
| 4 | minor | `src/web/api/mod.rs` | A repeated parameter kept the last value silently while an unknown one was refused. `Params::parse` now answers `400 INVALID_PARAMETER` naming it (`limit is given more than once`). | fixed in review (permission granted) |
| 5 | nit | `src/web/api/mod.rs` | An empty `target=` is read as "no filter". Behaviour kept and now documented in the user guide. | fixed in review |

### Evidence
- Goals: 9/9 achieved, each with a test in `tests/web.rs` (41 tests) or a unit test in `src/store/mod.rs` (28 tests).
- Steps: 13/13 ticked; checklist 18/18.
- Scope: every changed file is in `## Affected modules`. `src/web/page.rs` and `src/web/detail.rs` were touched only for the `Fault { interface: None }` fixtures, recorded in the Step 1 notes. No out-of-scope file.
- No existing test weakened: `tests/web.rs` gained 554 lines and lost none (the file held no API test before this change); the `HEAD` test was replaced inside this same uncommitted change and the plan records it.
- Edge probes on the wire: `from` in RFC 3339 with and without milliseconds, epoch milliseconds, percent-encoded target key, `offset` past the end, `offset` above `u32`, `from == to`, and an unknown parameter all answer as documented.
- Security: SQL parameters are bound, `limit` capped at 1 000, answers carry fixed error messages with the store detail in the log only, no request path echoed, no token or configuration in any record. Finding 1 is the one resource-exposure gap.
- Documentation: `FR-26` and `BR-8`, `ADR-0028` (record and index row), `ARCHITECTURE` `web::api`, `GLOSSARY` (Envelope, Day total), `DEVELOPER-GUIDE` layout, and the user guide `## REST API` all match the code. The guide is self-contained (no links to repository documents), and the `--compression` row now covers the JSON answers.

### Corrections applied in this round
- `README.md` — quick start gained `curl -s http://127.0.0.1:8642/api/v1/faults?limit=10`; "Status" now reads "…a 30-day per-target view, and a read-only JSON API under `/api/v1` that serves the targets, the faults, and the per-day totals to a dashboard."
- `src/web/mod.rs` — `if path == "/api/v1" || path.starts_with("/api/v1/")`.
- `src/web/api/mod.rs` — duplicate-parameter refusal in `Params::parse`.
- `docs/USER-GUIDE.md` — the 400 paragraph now names a repeated parameter, and states that an empty `target=` means none.
- `tests/web.rs` — `the_api_root_and_a_repeated_parameter_answer_the_envelope` pins both.

### Gap analysis
- Goals: 9/9 achieved.
- Scope: in scope; no out-of-scope file.
- Deviations: `Fault` fixtures, the replaced `HEAD` test, `include_body = method != "HEAD"`, and the `include_str!` depth after the module became a directory — all recorded in `## Implementation Notes`.
- Additions: Step 13 (the daily record's `target`, the user's decision) and the README repair of this review.
- Deferred: nothing planned was dropped.
- Known limitations: finding 1; `/targets` and `/faults/daily` write record by record from a bounded slice rather than streaming from SQLite (documented); `offset` paging over a list that grows at the top can repeat a record (documented, with `from`/`to` as the remedy).

### Recommendations for future work
- Decide the daily bound in `spec-implement`: either refuse a window longer than the 360 days `/target?days=…` already offers (`400 INVALID_RANGE`, named in the guide and the OpenAPI), or compute the day buckets only for the requested page. The first keeps the contract simple; the second keeps long windows available at the cost of more queries.
- An `Allow` header on the `405` is already there; a `Link` header on the packed pages would let a client navigate without reading `meta`.
### Step 14 — the daily page

- **Decision (user).** The window stays unbounded and the work is bounded instead: only the days
  the page returns are read, so `from=1970-01-01&limit=100` costs a hundred days, not seventy
  years. Capping the span at the 360 days the detail page offers was the alternative and was
  rejected, because it would have hidden history the page can already show.
- **Decision.** The timeline starts at `min(page_from, now - 1 day)`: the UTC day still running
  states its average over the last 24 hours, which can reach before the page's first day when the
  page holds only that day.
- **Evidence.** 20 000 events across two years: `from=1970&limit=100` → 46 ms, 19.7 KB, RSS 10 MB
  (before: 0.11 s, 196 KB, RSS 13 MB); `limit=1000` → 111 ms, RSS 10 MB, so the cost tracks
  `limit` and not the span.

## Review Notes

- **Round**: 2
- **Review date**: 2026-11-14
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(api): add the read-only JSON API under /api/v1`
- **Quality gate**: tests ✓ (43 web, 280 lib, full suite) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (round 1) | `src/web/api/daily.rs` | The daily window had no upper bound and every day of it was computed. Fixed in Step 14: the route reads only the page of days it returns. Measured on 20 000 events over two years: `from=1970&limit=100` went from 0.11 s / 196 KB / RSS 10→13 MB to 46 ms / 19.7 KB / RSS 10 MB. | fixed in `spec-implement` |
| 2 | major | `src/web/api/daily.rs` | **Regression of Step 14.** `Page::of` returns `None` when `offset` reaches the day count, and the route answers that branch with `"total": 0`. Probed: `/api/v1/faults/daily?offset=1000&limit=5` on a 31-day window → `"data": []`, `"total": 0`. The contract states the opposite (user guide: "`meta.total` is how many records match the filters, whatever the page holds"; OpenAPI `Meta.total`: "whatever the page holds"), and the previous behaviour reported 31. `Page` must carry the window's day count on the empty page. No test covers an `offset` past the end of a daily page, which is how it escaped. | open |
| 3 | minor (round 1) | `README.md` | The API was missing from the quick start and the status list. | fixed in review (round 1) |
| 4 | minor (round 1) | `src/web/mod.rs` | `/api/v1` answered the page's `text/plain` 404; it now answers `404 NOT_FOUND` in the envelope. Re-verified. | fixed in review (round 1) |
| 5 | minor (round 1) | `src/web/api/mod.rs` | A repeated parameter was kept silently; it is now `400` naming it. Re-verified. | fixed in review (round 1) |
| 6 | nit (round 1) | `src/web/api/mod.rs` | An empty `target=` reads as "no filter"; documented in the guide. Re-verified. | fixed in review (round 1) |

### Gap analysis
- Goals: 9/9 achieved, but finding 2 breaks the `meta` goal on the empty page.
- Steps: 14/14 ticked; checklist 19/19.
- Scope: in scope; every changed file is in `## Affected modules`. `README.md` was added by review round 1.
- Deviations: all recorded in `## Implementation Notes` (Step 1 fixtures, Step 3 `include_body`, Step 5 `include_str!` depth, Step 9 replaced HEAD test, Step 14 paging).
- Additions: Step 13 (the daily `target`, user decision) and Step 14 (the daily page, user decision).
- Deferred: nothing.
- Known limitations: `/targets` and `/faults/daily` write record by record from a bounded slice rather than streaming from SQLite (documented); `offset` paging over a list that grows at the top can repeat a record (documented, with `from`/`to` as the remedy).

### Evidence for round 2
- Round-1 finding 1: measured before/after on the same 20 000-event, two-year database; the cost now follows `limit` (`limit=1000` → 111 ms, RSS 10 MB).
- Round-1 corrections re-verified on the wire: `/api/v1` → `404 application/json`, `limit=5&limit=6` → `400 INVALID_PARAMETER`, `target=` → `200`.
- Finding 2 reproduced on the wire, quote above.
- Quality suite re-run in this session after every change.

### Recommendations for future work
- Fix finding 2 by giving `Page` a `total` valid even when the page is empty (`Page::of` → `Some(Page { from_ms, to_ms, total, take: 0 })`, or return the total beside the optional range), and pin it with a test: a 10-day window asked with `offset=1000` answers `"data": []` and `"total": 10`.
### Review round 2 — the empty daily page

- **Finding 2 fixed.** `Page` now carries `take`, and `Page::of` no longer returns `None` when
  `offset` reaches the day count: the route answers an empty `data` with the window's `total`,
  as the user guide and the OpenAPI state. `None` is reserved for a window that holds no day at
  all, which cannot happen while `from < to`, and the fallback states `total: 0` for it.
- **Decision.** The queries are skipped when `take == 0` (`daily_drops` and `timeline` are not
  called), so an offset past the end costs one arithmetic step and no database read.
- **Evidence.** On the wire: `?limit=5&offset=1000` on the 31-day default window → 0 records,
  `total: 31`; on an explicit ten-day window → 0 records, `total: 10`. Test:
  `a_daily_page_past_the_end_still_states_the_total_of_the_window`.

## Review Notes

- **Round**: 3
- **Review date**: 2026-11-15
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(api): add the read-only JSON API under /api/v1`
- **Quality gate**: tests ✓ (44 web, 280 lib, full suite) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (round 1) | `src/web/api/daily.rs` | The daily window was unbounded and every day of it was computed. Fixed in Step 14; measured 0.11 s / RSS 13 MB → 46 ms / RSS 10 MB on 20 000 events over two years. | fixed in `spec-implement` |
| 2 | major (round 2) | `src/web/api/daily.rs` | `offset` past the end answered `"total": 0`. Fixed: `Page` carries `take` and the empty page states the window total. Re-verified on the wire: 0 records, `total: 31` and `total: 10`. | fixed in `spec-implement` |
| 3 | major | `src/web/api/daily.rs` | **New regression of Step 14.** `Page::of` builds the page from the day start (`first_day + skip * DAY_MS`) and passes it to `Store::daily_drops` and `Store::timeline`, so the first day is no longer clipped to the requested `from`: down time and `up` runs from before `from` are counted. Probed with a target down 09:00→11:00 on one UTC day: `from=10:00` reports `downMs` 7 200 000, exactly as `from=00:00`, where the contract says 3 600 000. The OpenAPI `DayTotal` description states "A day that the window clips at its edges counts only what falls inside it". No test uses a `from` at mid-day. | open |
| 4–6 | minor/nit (round 1) | `README.md`, `src/web/mod.rs`, `src/web/api/mod.rs` | API missing from the README; `/api/v1` answered `text/plain` 404; a repeated parameter was kept silently; an empty `target=` undocumented. | fixed in review (round 1) |

### Gap analysis
- Goals: 9/9 stated, but finding 3 breaks the day-total goal at the window edge.
- Steps: 14/14 ticked; checklist 19/19.
- Scope: in scope; every changed file is in `## Affected modules` (`README.md` added by review round 1).
- Deviations: recorded in `## Implementation Notes`.
- Additions: Step 13 and Step 14, both by user decision.
- Deferred: nothing.

### Escalation — three review rounds on one route

The daily route has taken three rounds, and **each round's fix introduced a new edge-case defect in the same computation**:

1. Round 1: the window was unbounded → Step 14 restricted the reads to the page of days.
2. Round 2: restricting the reads emptied the `total` on a page past the end → `Page` gained `take`.
3. Round 3: building the page from the day start dropped the `from` clipping → the first day counts time outside the window.

The pattern is the plan, not the code: Step 14 was written as an amendment to a route whose day-boundary rules were never stated in one place. Every reviewer-visible defect lives at an edge of the same three-way intersection — day boundaries, the requested window, and the page. Escalated to the user, who chose to **re-plan the daily route** with an explicit day-boundary design instead of a fourth patch.

### Required shape of the re-plan (evidence for `spec-plan`)

- State, once, how the days of a window are derived from `from`/`to`: the first bucket is the UTC day containing `from`, the last is the UTC day containing `to - 1 ms`, and every read is clipped to `[from, to)` so an edge day counts only what falls inside it.
- State how `offset`/`limit` select a contiguous slice of that list, and that the page's reads are `[max(slice_start, from), min(slice_end, to))`.
- State what an empty page states: `data: []` and the window's `total`.
- Tests that pin all four: first day clipped by a mid-day `from` (`downMs` and `avgUpRunMs`), an empty page past the end, a wide window returning only `limit` days, and the current UTC day still stating the last 24 hours.

### Known limitations
- `/targets` and `/faults/daily` write record by record from a bounded slice rather than streaming from SQLite (documented); `offset` paging over a list that grows at the top can repeat a record (documented, with `from`/`to` as the remedy).

### Recommendations for future work
- After the re-plan, keep the four edge tests above next to the route: every defect this route has shown lives on one of those edges.
### Step 15 — the day boundaries, stated once and clipped

- **Decision.** `Page` carries the *clipped* read range (`from_ms: page_start.max(window_from)`,
  `to_ms: page_end.min(window_to)`), so the route never re-derives a boundary and no later edit can
  drop the clipping again. The five rules live in the type's documentation comment.
- **Discovery.** Clipping the first day changes `faults`, not only `downMs`: a fault that opened
  before `from` is down time inside the window but not a drop of it, which is what
  `Store::daily_drops` already documents ("a disconnection that opened before the window is not
  counted as a drop"). The test asserts both numbers for that reason.
- **Evidence.** On the wire, one UTC day with the target down 09:00→11:00: `from=00:00` → faults 1,
  `downMs` 7 200 000, `avgUpRunMs` 25 200 000; `from=10:00` → faults 0, `downMs` 3 600 000,
  `avgUpRunMs` 46 800 000.
- **Tests.** `the_first_day_of_a_daily_page_is_clipped_to_the_requested_from`,
  `the_current_utc_day_states_the_up_runs_of_the_last_24_hours`, plus the existing
  `a_daily_page_past_the_end_still_states_the_total_of_the_window` and
  `a_window_since_1970_states_its_total_and_returns_only_the_page`: one test per edge.

### Step 16 — the documents

- **OpenAPI.** No edit: the `/api/v1/faults/daily` description already stated both rules ("a day
  that the window clips at its edges counts only what falls inside it", "the UTC day still running
  is flagged `partial`: its average and longest up run cover the last 24 hours"), which is what the
  review quoted. Verified by inspection against `Page`'s rule and by the tests.
- **Decision.** The user guide and the architecture now name the clipping and the fault-vs-down-time
  distinction, so a reader does not have to infer them from the code.

## Review Notes

- **Round**: 4
- **Review date**: 2026-11-15
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(api): add the read-only JSON API under /api/v1`
- **Quality gate**: tests ✓ (46 web, 280 lib, full suite) | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major (round 1) | `src/web/api/daily.rs` | Unbounded daily window. Fixed in Step 14 (reads follow the page). | fixed in `spec-implement` |
| 2 | major (round 2) | `src/web/api/daily.rs` | `offset` past the end answered `"total": 0`. Fixed and re-verified. | fixed in `spec-implement` |
| 3 | major (round 3) | `src/web/api/daily.rs` | The first day was not clipped to `from`. Fixed in Step 15: `Page` carries the clipped read range, and the rule is stated once in its documentation. Re-verified on the wire: `from=10:00` → `downMs` 3 600 000, `avgUpRunMs` 46 800 000. | fixed in `spec-implement` |
| 4 | **blocker** | `src/web/api/daily.rs:143`, `src/store/mod.rs:892`, `src/web/mod.rs:130` | **Unauthenticated denial of service.** An extreme but syntactically valid instant panics the handler: `window_from_ms.div_euclid(DAY_MS) * DAY_MS` overflows at `from=-9223372036854775808` (`attempt to multiply with overflow`), and `day += DAY_MS` overflows near `i64::MAX` (`attempt to add with overflow`). A panicking thread never reaches `active.fetch_sub(1)` in `web/mod.rs`, so each panic permanently consumes one of the 32 `MAX_CONNECTIONS` slots: after 32 requests every answer — page and API — is dropped. Reproduced: `/healthz` → `200 OK`, 40 craft requests, `/healthz` and `/` → no response, 32 panic lines. The process stays up, so a supervisor does not restart it. | open |
| 5–8 | minor/nit (round 1) | `README.md`, `src/web/mod.rs`, `src/web/api/mod.rs` | API missing from the README; `/api/v1` answered `text/plain` 404; a repeated parameter was kept silently; an empty `target=` undocumented. | fixed in review (round 1) |

### Gap analysis
- Goals: 9/9 for the documented inputs; finding 4 shows the accepted input domain is not bounded.
- Steps: 16/16 ticked; checklist 21/21.
- Scope: in scope; every changed file is in `## Affected modules`.
- Deviations: recorded in `## Implementation Notes`.
- Additions: Steps 13, 14, 15 by user decision; Step 16 documents them.
- Deferred: nothing.

### Evidence
- Round-3 finding re-verified on the wire, one target down 09:00→11:00 on a fixed UTC day: `from=00:00` → faults 1, `downMs` 7 200 000, `avgUpRunMs` 25 200 000; `from=10:00` → faults 0, `downMs` 3 600 000, `avgUpRunMs` 46 800 000; a mid-day `to`, a two-day window, `offset` on the last day, and an empty page past the end all match the rule in `Page`.
- Finding 4 reproduced twice: 0 responses after 40 craft requests, 32 panic lines on stderr, process still running.
- Full suite re-run in this session after every change: green.

### Escalation — fourth round, third defect class on the daily route

1. Round 1: unbounded window → bound the reads (Step 14).
2. Round 2: bounding emptied the `total` → `Page` carries `take` (Step 14 fix).
3. Round 3: building from the day start lost the `from` clipping → clip the read range (Step 15).
4. Round 4: the arithmetic of those boundaries panics on the edges of `i64`, and a panic wedges the connection accounting.

The route's *domain* has never been stated: which instants are accepted, and what the server does when a handler fails. Escalated; the user chose to **plan** the fix rather than patch again.

### Required shape of the re-plan (evidence for `spec-plan`)

- Accept `from`/`to` only inside a stated domain (the project stores epoch milliseconds and no event can be later than now: `0 ..= now + 1 day` is the natural one), and answer `400 INVALID_PARAMETER` outside it. One rule removes the whole arithmetic class instead of saturating every expression.
- Use saturating or checked arithmetic in `Page::of` and in `Store::daily_drops` as defence in depth, so a future caller outside the API cannot panic either.
- Make the connection accounting panic-safe: decrement `active` from a guard whose `Drop` runs on unwind, so a panic in any handler costs one request and never a slot.
- Tests: an extreme `from` and an extreme `to` answer `400`; a panicking handler does not consume a slot (a test-only route or a direct call of the guarded wrapper); the four day-boundary probes of Step 15 stay green.

### Known limitations
- `/targets` and `/faults/daily` write record by record from a bounded slice rather than streaming from SQLite (documented); `offset` paging over a list that grows at the top can repeat a record (documented, with `from`/`to` as the remedy).
- Until finding 4 is fixed, the API must not be published: a single client can make the page and the API deaf.

### Recommendations for future work
- Add the extreme-instant probes to the manual verification of every plan that accepts an instant.
- Consider the same guard for the static routes: the panic-safety of the accounting is a server-wide property, not an API one.

### Step 17 — Accept every instant that fits an integer

- **Decision.** The accepted range is every value that fits an `i64`, as the user chose: no
  new bound is invented, the parse already rejects everything beyond it with `400`. Inside the
  range the arithmetic is saturating: `first_day` uses `saturating_mul` in `Page::of`, the
  page start and end keep `saturating_add`, and both walks of `Store::daily_drops` and its
  bucket lookup go through a saturating `day + DAY_MS`.
- **Discovery — the fix that a green build hid.** The new test
  `an_instant_at_the_end_of_the_range_answers_an_envelope` did not fail, it *hung*: the test
  binary never finished. Isolated with `sample <pid>` on a server serving the same database,
  which showed the worker spinning in `web::stats::up_runs_per_day`. The arithmetic of the
  step was not the cause — with `to = i64::MAX` the window holds 31 days, the page takes 3,
  and every `Page` figure was correct. The cause was rule 4 of `Page::timeline_from_ms`:
  `min(read from, now - 1 day)` with a page wholly in the future returned `now - 1 day`, so
  the timeline spanned the whole calendar, and `up_runs_per_day` walked from the present to
  `i64::MAX` one day at a time — about 3.8e10 iterations — for every target still `up`.
- **Fix.** `Page::timeline_from_ms` reaches back to `now - 1 day` only when the page reaches
  the present (`read from <= now`), and returns `read from` otherwise: a page in the future
  holds no running day, so it needs no rolling window. The rule is stated in the `Page` doc
  comment, next to the other four. `src/web/stats.rs` stays untouched, as *Affected modules*
  requires, and a window that reaches the present is unchanged — the timeline still starts at
  the page and still includes the last 24 hours.
- **Verify.** `cargo test --test web` → 48 passed in 1.7 s (no hang);
  `cargo test --lib store` → 28 passed. Manual: against a database holding an open fault,
  `from=-9223372036854775808&limit=3` → `200` in 92 ms,
  `to=9223372036854775807&limit=3` → `200` in 102 ms,
  `from=9223372036854775808` and `to=-9223372036854775809` → `400 INVALID_PARAMETER` naming
  the parameter, each in about 100 ms.
- **Note for the reviewer.** The extreme-instant test is the regression proof of the hang, so
  its failure mode is a timeout, not an assertion: without the fix, `cargo test --test web`
  never finishes. A fast-failing guard would need a read timeout in the test harness, which
  is outside this plan.

### Step 18 — Make a panicking handler cost one request, never a slot

- **Decision.** `Slot::enter(&active)` counts the worker and the guard's `Drop` releases the
  count, so the worker closure owns the slot for the whole of `handle`. The limit is read where
  it always was, before the spawn, and a refused connection is still `drop(stream)`: the change
  moves the decrement, nothing else.
- **Tests.** `src/web/mod.rs` unit: a slot counts and releases; a slot taken inside a
  `catch_unwind` that panics is still released, with the panic hook silenced for that block so
  the suite stays readable. `tests/web.rs` integration:
  `a_run_of_requests_never_consumes_a_connection_slot_for_good` sends 40 requests with the
  extreme `from` of Step 17 and then asks `/healthz` and `/` — both `200`, and the page still
  holds the history.
- **Verify.** `cargo test --lib web` → 96 passed; `cargo test --test web a_run_of_requests` →
  1 passed in 4.5 s.
- **Note.** After Step 17 no input of the API panics, so the guard cannot be exercised over the
  wire; it is proven by the unit test, and it stays as the property of the server rather than of
  one route.

### Step 19 — State the accepted range and the failure contract

- `docs/USER-GUIDE.md`, `## REST API`: a sentence under the parameter table states the accepted
  range (`-9223372036854775808` to `9223372036854775807`) and the `400 INVALID_PARAMETER`
  beyond it; a paragraph before the examples states the failure contract — the connection closes
  with no status line, the reason goes to the log, and the next request is served normally.
- `assets/api/openapi.json`: the `From` and `To` descriptions state the accepted range; the
  document is still valid JSON and the drift test in `tests/web.rs` still passes.
- `docs/ARCHITECTURE.md`, `web::api` guarantees: the range, the saturation, and the future-page
  timeline rule; `web` guarantees: the connection count is released on unwind.
- `docs/ADR.md`, `ADR-0028`: decision 12 (the accepted range and the saturating arithmetic),
  decision 13 (the panic-safe slot), and two consequences — the failure contract a client must
  read, and the daily route's page-scoped timeline.
- **Verify.** Inspection against `src/web/api/mod.rs` (`parse_instant` parses `i64`),
  `src/web/mod.rs` (`Slot`), `src/web/api/daily.rs` (`Page`), and `src/store/mod.rs`
  (saturating day walk); `cargo test` green.

## Review Notes

- **Round**: 5
- **Review date**: 2026-11-15
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(api): add the read-only JSON API under /api/v1`
- **Quality gate**: tests ✓ (282 lib, 49 web, 17 cli, 7 config, 18 import, 6 monitor_e2e, 5 notify, 6 reload, 4 store; 12 ignored) | lint ✓ | fmt ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | **blocker** | `src/web/stats.rs:72`, `src/store/mod.rs:210`, `src/web/api/daily.rs:25` | **Round-4 denial of service still open, in a worse shape, and Step 17 does not cover it.** Step 17 names two overflow sites; three remain in the same path. `up_runs_per_day` (`stats.rs:72`) is reached with an `up` run that ends at `to_ms = i64::MAX`, because an open `up` segment ends at the end of the window: `let end = (day + day_ms).min(to_ms)` is a plain add. In `debug` it panics (`attempt to add with overflow`); in `release` — the profile that ships — it wraps to `-9223372036794351616`, so `cursor` moves **backwards**, the `while cursor < to_ms` loop never terminates, and each pass pushes a `BTreeMap` entry: measured `RSS 3 MB → 1.9 GB at 2 s → 7.9 GB at 5 s` for **one** unauthenticated request `GET /api/v1/faults/daily?from=9223372036854000000&to=9223372036854775807&limit=1000` (≈2.1e11 days still to walk). 33 such requests answer `ConnectionResetError` on `/healthz` **and** on `/`: the page and the API are deaf, exactly as in round 4. `store::day_slices` (`store/mod.rs:210`) has the same plain `day + DAY_MS` and panics in debug on `to=9223372036854775807&offset=30&limit=5`. `daily.rs:25` (`to - DEFAULT_DAYS * DAY_MS`) panics in debug on `to=-9223372036854775800`, and in release wraps so the route answers `400` for an input combination the contract accepts. The documented contract (`docs/USER-GUIDE.md` "any epoch-millisecond value that fits a signed 64-bit integer", `ADR-0028` decision 12) is therefore not delivered. | open |
| 2 | major | `tests/web.rs` | The Step 17 tests do not reach any of the three sites: `a_instant_at_the_end_of_the_range_answers_an_envelope` sends `from=-9223372036854775808&limit=3` and `to=9223372036854775807&limit=3` against a database whose extreme days hold no `up` run, so `up_runs_per_day` is entered with an empty run list and the wrap is never exercised. No test uses an extreme `to` **without** `from` (the `daily.rs:25` underflow), no test puts an open `up` run at the top of the range, and no test asserts the same answer in the `release` profile, where the failure is a hang rather than a panic. | open |
| 3 | major | `tech-plans/005-rest-api.md` (Step 17, *Affected modules*) | **The plan, not the implementation, is what is wrong.** Step 17 enumerates two expressions and *Affected modules* line 113 states "`src/web/stats.rs` — reused as is …; no change", while the daily route calls `stats::up_runs_per_day` on a window it now allows to reach the top of `i64`. The implementer did what the plan says; the goal "Accept every instant that fits an integer, and overflow nowhere" cannot be met with the scope the plan declares. | open |
| 4 | note | `docs/USER-GUIDE.md`, `docs/ADR.md` | The guide and `ADR-0028` decision 12 state the accepted range as a fact. They are not stale — they state the intended contract — so the reviewer does **not** weaken them: the code has to meet them (finding 1). No documentation hunk is proposed this round. | open (blocked by 1) |

### Evidence

Quality suite, run in this session: `cargo fmt --check` → clean; `cargo clippy --all-targets -- -D warnings` → clean; `cargo test` → all suites green (see the gate line above).

Round 1–4 findings re-verified on the wire today, one target down 09:00→11:00 UTC on 2026-11-10:

| Probe | Result |
|---|---|
| `from=00:00&to=24:00` | `faults 1`, `downMs 7 200 000`, `avgUpRunMs 39 600 000` |
| `from=10:00&to=24:00` | `faults 0`, `downMs 3 600 000`, `avgUpRunMs 46 800 000` — round-3 clipping still exact |
| `offset=1000&limit=5` | 0 records, `total 1` — round-2 empty page still states the window total |

Step 18, verified on a live server: 40 panicking requests on the low extreme and 40 on the high one → 80 panic lines, then `/healthz` `200` and `/` `200` with the full page. **The slot guard works.** The blocker is therefore no longer the accounting: it is a request that never ends, which holds its slot legitimately.

The release-profile measurements behind finding 1, `target/release-dev` binary on a database with one open `up` run:

| Input | debug | release |
|---|---|---|
| `to=9223372036854775807&limit=3` | panic `stats.rs:72` | (page takes the low days) |
| `from=9223372036854000000&to=9223372036854775807&limit=1000` | panic `stats.rs:72` | no answer in 8 s; RSS 3 → 1 916 → 7 900 MB |
| `to=9223372036854775807&offset=30&limit=5` | panic `store/mod.rs:210` | no answer in 10 s |
| `to=-9223372036854775800` | panic `daily.rs:25` | `400` (wrapped `from` makes `from >= to`) |
| 33 × the hanging request | — | `/healthz` and `/` `ConnectionResetError` |

### Gap analysis
- Goals: 10/11 achieved. The goal of the failure contract ("every failure is a JSON object with a stable `code`") holds for refusals; the goal that an accepted-integer instant never overflows is **not** achieved (finding 1).
- Scope: in scope — the 23 files of the diff are the files *Affected modules* lists. Finding 3 is the mirror image: the plan's declared scope is too narrow for its own goal.
- Deviations: none undocumented. The timeline rule added in Step 17 (`Page::timeline_from_ms` returns the page for a future window) is recorded in *Implementation Notes* and in the `Page` doc comment, and it fixed a genuine non-termination on a page wholly in the future.
- Additions: none beyond the plan.
- Deferred: nothing planned was dropped.

### Known limitations
- A request that never terminates keeps its connection slot; `MAX_CONNECTIONS` (32) bounds the concurrency but not the memory or the CPU of the 32 workers (`stats.rs` builds a `BTreeMap` keyed by day, so its memory is proportional to the days it walks).
- `/targets` and `/faults/daily` write record by record from a bounded slice rather than streaming from SQLite (documented); `offset` paging over a list that grows at the top can repeat a record (documented, with `from`/`to` as the remedy).

### Escalation — fifth round, same defect class, second time open

1. Round 1: the daily window was unbounded → Step 14 restricted the reads to the page of days.
2. Round 2: restricting the reads emptied the `total` on a page past the end → `Page` gained `take`.
3. Round 3: building the page from the day start dropped the `from` clipping → the first day clipped to the window.
4. Round 4: the arithmetic of those boundaries panicked on the edges of `i64`, and a panic wedged the connection accounting → Step 17 (saturating arithmetic) and Step 18 (`Slot` guard).
5. Round 5: the panic is gone from the two expressions Step 17 named, and the accounting is fixed, but the **same input class still takes the server down** — now by hanging one worker and growing its memory to gigabytes, through three sites the plan left out of scope.

The pattern is that each round fixes the reported expression and the next one in the same call chain surfaces. Asking for a sixth patch of three more expressions is not the fix; the plan has to state the rule for the whole path — the daily route, the store walk, and the stats split — and how a page that touches the edges of `i64` terminates. Recommended path: **`spec-plan` revision of 005**, then `spec-implement`, then `spec-review` again. Patching inside the current Step 17 wording would leave `src/web/stats.rs` out of scope for a third time.

### Recommendations for future work
- Make the accepted domain of a time filter a single validated rule at parse time (`Params::parse_instant`) rather than a promise kept by every downstream formula, so a new caller cannot reintroduce the class.
- Add a bounded-termination property test for `up_runs_per_day`, `day_slices`, and `Store::daily_drops`: for any pair of instants, the walk yields at most the number of days the answer states.
- Run the extreme-instant probe matrix against the `release` profile in the verification plan of every plan that accepts an instant; `debug` panics hide `release` hangs that are strictly worse.

### Step 20 — the accepted calendar domain

- **Implemented as planned.** `DOMAIN_MIN_MS = 0` and `DOMAIN_MAX_MS = 4_102_444_800_000` in
  `src/web/api/mod.rs`, checked in `parse_instant` after both parse paths, with the message built
  from the constants through `time::format_rfc3339`; the daily route clamps `now_ms()` into the
  domain and protects the default window with `to.saturating_sub(…).max(DOMAIN_MIN_MS)`.
- **Decision — a `Rejection` carries no `Debug`, and the tests do not need one.** The unit tests
  match on `Ok`/`Err` instead of `unwrap`/`expect_err` rather than deriving `Debug` on a
  production type only for a test. No production API changed for testing.
- **Deviation — the days a future window reaches state nothing, and that needed the store.** The
  step's test asserts `faults: 0, downMs: 0` for a day after today, which the store did not
  provide: `Store::daily_drops` ended an open disconnection at `to_ms`, so a 20-day-future window
  reported 24 h of down time *per future day*, and `Store::timeline` filled `up` runs to the same
  end, so `avgUpRunMs` stated 86 400 000 for days that have not happened. Both asserted a future
  outcome, i.e. invented data. Fix, inside `src/store/mod.rs` and `src/web/api/daily.rs` (both in
  *Affected modules*): `daily_drops` takes `now_ms` and ends an open disconnection at
  `to_ms.min(now_ms)`, and the daily route reads the timeline only to `page.to_ms.min(now)`. The
  detail page passes `now` as both, so its numbers are unchanged.
- **Deviation — the period queries are built relative to the clock, not from fixed dates.** The
  step wrote `from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z`; a fixed date would make the
  test fail after 2026-12-01 and the "days after today are empty" assertion would be untestable
  forever. The test derives the same window from `now` (ten days before today to twenty after =
  the 30 days of a month; 360 before to 5 after = 365 days of a year), which asserts the same
  property and stays true in 2030.
- **Deviation — the Step 18 slot test changed its request.** It sent the extreme `from` that is
  now refused; it now alternates a valid 100-record request and a refused one, 40 times, which
  proves a rejected request does not leak a slot either. It is the only existing test outside the
  one the step names, and Step 20's domain change is what invalidated its input.
- **Verify.** `cargo test --lib web::api` → 3 passed; `cargo test --test web` → 53 passed; manual,
  against a seeded database: `to=9223372036854775807` → `400` in 25 ms,
  `from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z` → `200`, `"total": 30`,
  `to=1970-01-01T00:00:00Z` → `400 INVALID_RANGE`,
  `from=1970-01-01T00:00:00Z&to=2100-01-01T00:00:00Z&limit=1000` → `200` in 101 ms / 196 KB (only
  the page is read: the whole domain holds 47 482 days and the test states `total` 47 482 with
  1 000 records).

### Step 21 — the two day walks

- **Implemented as planned.** `up_runs_per_day` and `day_slices` use `saturating_mul` and
  `saturating_add` and stop when a step does not move forward.
- **Discovery — the count of slices is not the count of days at the very edges of `i64`.**
  `i64::MIN` is not a UTC day boundary, and the day that contains it starts at the clamped
  minimum, so a five-day window near either edge yields six slices, the first or last being the
  part that fits. The tests therefore assert what actually holds — bounded count, every slice
  positive, slices summing to the span — instead of an exact count that would encode a false
  claim about the calendar. The invariant the plan promised, bounded termination, is what they
  pin.
- **Verify.** `cargo test --lib store` → 29 passed (1 new); `cargo test --lib web` → 101 passed
  (2 new); `cargo test --test web` → 53 passed in 8.7 s, unchanged and green.

### Step 22 — the domain in the documents

- **Implemented as planned.** `docs/USER-GUIDE.md` (the paragraph under the parameter table and
  both `from`/`to` rows, which no longer promise "no bound on faults", with the worked
  "November 2026 on any day of November" example), `assets/api/openapi.json` (`From`/`To`
  descriptions, still valid JSON and still green against the drift guard),
  `docs/PRODUCT.md` (`FR-26`), `docs/ADR.md` (`ADR-0028` decision 12 rewritten, with the
  `now`-anchored ceiling recorded as rejected, and two consequences: future days carry no
  measurement, and a library caller still pays for a span wider than the domain), and
  `docs/ARCHITECTURE.md` (`web::api` guarantees).
- **Verify.** `grep -rn "64-bit" docs/ assets/` returns nothing; `python3 -c "import json;
  json.load(open('assets/api/openapi.json'))"` loads; full suite green; the document statements
  were checked against `DOMAIN_MIN_MS`/`DOMAIN_MAX_MS` and `parse_instant`, not against memory.

### The round-5 blocker, re-measured on the profile that ships

`target/release-dev/uptime-monitor serve`, a database with an open `up` run, RSS 4 MB idle:

| Request | Before (round 5) | After |
|---|---|---|
| `from=9223372036854000000&to=9223372036854775807&limit=1000` | no answer, RSS 7 900 MB at 5 s | `400` in 0.07 s |
| `to=9223372036854775807&offset=30&limit=5` | no answer in 10 s | `400` in 0.10 s |
| `from=1970-01-01T00:00:00Z&to=2100-01-01T00:00:00Z&limit=1000` | — | `200`, 196 KB, 0.12 s |
| `from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z` | — | `200`, 0.09 s |
| 33 concurrent widest-window requests | `/healthz` and `/` reset | RSS 4 → 10 MB, `/healthz` `200`, page `200` |

## Review Notes

- **Round**: 6
- **Review date**: 2026-11-15
- **Verdict**: PASS
- **Suggested commit**: `feat(api): add the read-only JSON API under /api/v1`
- **Quality gate**: tests ✓ (405 passed, 0 failed) | lint ✓ (`cargo clippy --all-targets -- -D warnings`) | fmt ✓ (`cargo fmt --check`)

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | minor | `docs/USER-GUIDE.md` | The `from`/`to` rows of the parameter table stated the domain bounds as **defaults** (`1970-01-01T00:00:00Z` on faults, `2100-01-01T00:00:00Z` on faults). `/faults` applies no default bound at all: `faults.rs` passes `from_ms: params.from` / `to_ms: params.to` straight to `FaultFilter`. Verified on the wire: with both parameters absent, an imported `1965-01-01` fault and an imported `2200-01-01` fault are both returned, and `from=1970-01-01T00:00:00Z` excludes the first. Corrected to `none on faults` for both rows, keeping the domain sentence that follows the table. | fixed in review |
| 2 | minor | `docs/USER-GUIDE.md`, `docs/ARCHITECTURE.md`, `assets/api/openapi.json` | Three statements still promised a daily window of **any length** ("The window may be any length", "it may be any length: only the days the page returns are read"). The domain now bounds it: a window holds 47 482 days at most. Reworded to "any width the domain allows (47 482 days at most)", and the OpenAPI description of `GET /api/v1/faults/daily` gained the rule the code enforces — a day that has not begun states no faults, no down time and no up run. Verified against `daily.rs` and by the wire probe in the evidence table. | fixed in review |
| 3 | nit | `src/web/api/mod.rs` | `from=+1000` is accepted: `str::parse::<i64>` takes a leading `+`, so it reads as epoch-millisecond `1000`. The answer is well defined and inside the domain, so this is leniency, not a defect. | open (no action) |
| 4 | nit | `src/store/mod.rs` | `daily_drops` now takes four positional arguments and the page passes `now` twice (`daily_drops(target, from, now, now)`). A narrower signature would read better; behaviour is correct and the redundancy is confined to one call site. | open (no action) |

### Evidence

Quality suite, run in this session after the documentation corrections: `cargo fmt --check` → clean; `cargo clippy --all-targets -- -D warnings` → clean; `cargo test` → **405 passed, 0 failed** (lib 288, web 53, cli 17, config_precedence 7, import 18, monitor_e2e 6, notify 5, reload 6, store 4, icmp_privileged 1; 10 ignored, all environment-gated). `validate 005 --strict` → 0 warnings; `checklist 005` → 27/27.

`release-dev` binary, a database with an `up` run open at the edge of the range and imported 1965/2200 events:

| Check | Result |
|---|---|
| `from=9223372036854000000&to=9223372036854775807&limit=1000` (round-5 killer) | `400` in 0.06 s — was no answer, RSS 7 900 MB at 5 s |
| `to=9223372036854775807&offset=30&limit=5` | `400` in 0.09 s | 
| 40 concurrent `from=1970-01-01T00:00:00Z&to=2100-01-01T00:00:00Z&limit=1000` | RSS 5 → 17 MB, `/healthz` `200`, `/` `200` |
| `from=2026-11-01T00:00:00Z&to=2026-12-01T00:00:00Z` | `200`, 30 records; a future day `2026-11-30` states `faults 0, downMs 0, avgUpRunMs null, longestUpRunMs null, partial false` |
| `from=0&to=86400000` / whole domain | `200`, `total 1` / `total 47 482` with 1 000 records read |
| `from=2100-01-01T00:00:00Z&to=2100-01-01T00:00:00Z` | `400 INVALID_RANGE` |
| `limit=0`, `limit=1001` | `400 INVALID_PARAMETER` |
| `offset=4294967295` | `200`, 0 records, `total 31` |
| envelope, `Transfer-Encoding: chunked`, no `Content-Length`, `no-store`, CORS | as specified |
| `HEAD`, gzip on the fly, `OPTIONS` → `204` with `Allow-Methods`, `POST` → `405` in the envelope | as specified |
| `/api/v1/nope` | `404 application/json`; unknown parameter → `400` naming it; unknown target → `404` |
| `/api/v1/openapi.json` | OpenAPI `3.1.0`, the four paths; the drift guard in `tests/web.rs` passes |
| `/` and `/healthz` | `200` — the page is unchanged |

Scope: 24 files, all inside `## Affected modules`; the only exception is `README.md`, added by review round 1 with permission and recorded there.
Tests: `tests/web.rs` gains 981 lines over the committed baseline and deletes none of the pre-existing tests; the two tests this feature replaced (`an_instant_at_the_end_of_the_range_answers_an_envelope`, and the input of `a_run_of_requests_never_consumes_a_connection_slot_for_good`) are the ones Step 20 authorised, and the second is recorded in *Implementation Notes*. New tests exercise real paths: the domain refusals and both bounds through HTTP, the widest legal window, a future-ending month and year, the `to`-at-the-lower-bound range error, the two day walks at both edges of `i64`, and the constants against the dates they claim.

### Gap analysis
- Goals: 11/11 achieved and observable. The instant goal now reads as a fixed calendar domain and is verified at both ends and outside.
- Scope: in scope.
- Deviations: three, all recorded in *Implementation Notes* and each forced by the plan's own assertions — `daily_drops` gained `now_ms` so future days carry no invented down time, the period tests are derived from the clock instead of fixed dates, and the slot test's input changed because its old input is now refused. All legitimate; the first one also corrects a real defect the plan's Step 20 test exposed.
- Additions: none beyond the plan.
- Deferred: nothing planned was dropped.

### Known limitations
- A library caller that hands `up_runs_per_day` or `day_slices` a span wider than the domain still walks it day by day. Termination is guaranteed by the advance guard; the cost is proportional to the span, and no path of this binary does it (`ADR-0028` consequence, `docs/ARCHITECTURE.md`).
- The 32-connection gate still lets a client with 32 slow requests delay the others, which is the page's pre-existing behaviour.
- `/targets` and `/faults/daily` write record by record from a bounded slice rather than streaming from SQLite; `offset` paging over a list that grows at the top can repeat a record, with `from`/`to` as the remedy.

### Recommendations for future work
- Derive `parse_instant`'s domain from the same constants the OpenAPI document and the user guide state, or generate those two texts from them, so a future change of bounds cannot leave a document behind.
- Consider the same advance guard for any future per-day loop: it is three lines and removes a whole failure class.
- When a bound must change, the tests to revisit are the two constant assertions in `src/web/api/mod.rs` and the four domain tests in `tests/web.rs`; both fail loudly on purpose.
