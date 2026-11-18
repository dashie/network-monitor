---
title: Add a regular-sample timeseries API derived from recorded events
date: 2026-11-18
status: done
type: feature
impact: high
area: web api, history, time, docs
version_bump: minor
tags: [api, timeseries, sampling, timezone, relative-time]
depends_on: []
branch_recommended: false
---

## Summary

A new read-only route, `GET /api/v1/timeseries`, turns one target's recorded transitions into
a series of samples on a regular grid. Every `step` seconds the series states whether the
target was `up`, `down`, or `unknown` at that instant; `limit` and `offset` page the samples,
and `from`/`to` bound the window. The samples are derived on demand from the existing
timeline segments — nothing new is stored.

The plan also upgrades the instant grammar of the JSON API. `from` and `to` accept `now`,
relative expressions (`-1h`, `-2d`, `-1mo`, `-1y`), and dates or date-times without an
explicit offset; a new `tz` parameter (fixed offset or IANA name, through `chrono` +
`chrono-tz`) supplies the zone those values are read in. `/api/v1/faults` and
`/api/v1/faults/daily` adopt the same grammar. The page, the CLI, the export/import path, the
database schema, and every existing answer for an RFC 3339 `Z` or epoch-millisecond input stay
unchanged.

Plan `015` reserves `FR-36` and `ADR-0041`; plan `016` reserves `FR-37` and `ADR-0042`. This
plan therefore uses `FR-38` and `ADR-0043` / `ADR-0044`. Plan `015` records monitor downtime
as `unknown`; when it lands, those episodes appear as `unknown` samples through the same
timeline, so this plan neither depends on it nor conflicts with it.

## Why

The API publishes transitions and per-day aggregates, but a dashboard, a CSV export, or a
comparison between targets needs the state at regular instants. Today a client must fetch the
events, rebuild the intervals, and sample them itself — a second implementation of
`timeline_segments` in every consumer. Persisting per-probe samples would contradict
[ADR-0004](../docs/ADR.md#adr-0004--persist-transitions-only-in-embedded-sqlite-with-wal) and
[ADR-0022](../docs/ADR.md#adr-0022--drop-the-down-time-and-availability-columns-from-the-timeline);
deriving them from the events keeps the storage decision intact and makes the series an exact
projection of the history.

The instant grammar is a second gap. Operators think in local dates and relative windows
("yesterday", "the last 6 hours"), while the API accepts only UTC RFC 3339 or epoch
milliseconds. Every consumer therefore hard-codes its own conversion and cannot express a
local calendar day across a DST change. Closing both gaps in one plan is cheaper than
teaching the grammar twice, and both changes live in the same request parser.

## Goals

- `GET /api/v1/timeseries` answers one page of samples for exactly one target, each record
  `{"target", "at", "atMs", "state"}` where `state` is `"up"`, `"down"`, or `"unknown"`.
- Samples lie on the epoch grid of `step` seconds: `from <= at < to`, `at` is a multiple of
  `step * 1000`, `step` is an integer `1`–`86400` defaulting to `5`.
- `target` is required; a value naming no target is `404 UNKNOWN_TARGET`, and a missing
  `target` is `400 INVALID_PARAMETER` naming `target`.
- `total` is the number of samples in the whole window; `limit` (`1`–`1000`, default `100`)
  and `offset` (`>= 0`, default `0`) count samples, and only the timeline covering the
  returned page is read, so the cost of a request follows `limit`, not the window width.
- A sample at an instant after the request's `now` is `unknown`; before a target's first
  event, or while no event has established a state, it is `unknown`; between two events it is
  the state the earlier one established.
- `to` defaults to `now`, `from` defaults to `to - 1h`, and `from` must be earlier than `to`
  (`400 INVALID_RANGE` otherwise).
- Every JSON API route that accepts `from`/`to` also accepts `tz` and the new instant forms,
  with identical rules and identical `400` refusals, and keeps its current answers for RFC
  3339 `Z` and epoch-millisecond input.
- A response's `meta` states the resolved `step` and `tz` in addition to the existing page,
  filters, total, and generated instant, so a client can reproduce the grid.

## Non-goals

- No stored samples, no schema change, no migration, and no new table.
- No cross-target series in one request: one call answers one target, and a client loops over
  `/api/v1/targets`.
- No `rtt`, label, cause, or interface in a sample; the record is state only.
- No sampling of the page, the SVG chart, the export, or the CLI; `query --since` and the
  page keep their current parsers.
- No per-parameter timezone: one `tz` applies to every value of a request that lacks an
  explicit offset.
- No ISO 8601 duration strings (`PT1H`); the accepted relative grammar is the suffixed form
  below.
- No authentication, no write route, and no change to `Access-Control-Allow-Origin: *`.

## Affected modules

**Source**

- `Cargo.toml`, `Cargo.lock` — add `chrono` and `chrono-tz` (its IANA database is embedded at
  build time, so Linux/macOS/Windows and the musl release need no system tzdb).
- `src/web/api/instant.rs` *(new)* — pure parser for `tz`, absolute instants, `now`, and
  relative expressions; calendar arithmetic in the effective zone.
- `src/web/api/timeseries.rs` *(new)* — the route and the pure `sample_segments` projection.
- `src/web/api/mod.rs` — register the route, extend `Params` with `tz`, extend `Meta` with
  optional `step`/`tz`, make the unknown-parameter message reflect the route's accepted list,
  and add the new rejections.
- `src/web/api/faults.rs`, `src/web/api/daily.rs` — accept `tz` and resolve `from`/`to`
  through the shared parser instead of the deleted local `parse_instant`.
- `src/history.rs` — add `History::timeline_of(target, from_ms, to_ms)`; re-export nothing new.
- `src/store/mod.rs` — implement `timeline_of` with a `target = ?` filter, reusing the
  existing segment query.
- `src/sim/history.rs` — implement `timeline_of` for the simulated source.

**Tests**

- `src/web/api/instant.rs` — unit tests for every accepted form, relative units, offsets, IANA
  zones, DST gaps/ambiguity, and every refusal.
- `src/web/api/timeseries.rs` — unit tests for grid alignment, page arithmetic, and the
  future-is-`unknown` rule, independent of HTTP.
- `tests/web.rs` — end-to-end route cases, error codes, `HEAD`/gzip/CORS, and the OpenAPI
  drift guard.
- `tests/store.rs` — `timeline_of` returns one lane and the same segments as `timeline`.
- `tests/simulate.rs` — a timeseries answer against the simulated history.

**Documentation / configuration**

- `assets/api/openapi.json` — new path, `Step`/`Tz` parameters, `TimeSeriesEnvelope` and
  `Sample` schemas; update the `From`/`To` parameter descriptions with the new grammar.
- `docs/PRODUCT.md` — `FR-26` (route, sampling, paging) and a new `FR-38` for the API instant
  grammar and `tz`; reference the new ADRs.
- `docs/ARCHITECTURE.md` — API contract and route list, the `History` contract, and the
  "no stored samples" consequence.
- `docs/DEVELOPER-GUIDE.md` — the source tree listing under `src/web/api/` gains
  `timeseries.rs` and `instant.rs`.
- `docs/USER-GUIDE.md` — the REST API section: route table, parameter table (`target` becomes
  required on this route; new `step` and `tz`), instant forms, relative examples, and `curl`
  examples.
- `docs/GLOSSARY.md` — **Sample**, **Step**, **Time series**, and **Time zone**.
- `docs/ADR.md` — index rows and records for `ADR-0043` (derived regular series) and
  `ADR-0044` (API instant grammar and the two time crates).

No configuration file or CLI option changes: the new options are HTTP query parameters,
documented in `assets/api/openapi.json` and `docs/USER-GUIDE.md` rather than in the TOML.

## Architecture impact

The web API gains a fifth data route and a shared parser, and the read contract
`History` gains one narrow method so a request reads one lane instead of every lane. The
architectural invariant "transitions are the only stored fact" is unchanged: the series is a
derived projection. `docs/ARCHITECTURE.md` must list `timeseries.rs`/`instant.rs` under the
API component, add `timeline_of` to the `History` contract, and state that sampling happens in
the route and never in the store.

## Decision-record impact

Two new records are required:

- **ADR-0043 — Derive a regular series from the timeline instead of storing samples.** Records
  the epoch-aligned grid, the half-open window, the `unknown` rule before the first event and
  after `now`, and the paging over samples. It supersedes nothing; it extends ADR-0004 and
  ADR-0028.
- **ADR-0044 — Accept relative and zoned instants on the JSON API through `chrono`/`chrono-tz`.**
  Records the grammar, the UTC default, the IANA-plus-fixed-offset `tz`, the calendar semantics
  of `d`/`w`/`mo`/`y`, the DST rules, and the cost of the two crates. It amends the parameter
  description in ADR-0028 without replacing its envelope, error, or CORS decisions.

## Proposed solution

### Step 1 — Add the time crates and prove they build on every target

Add `chrono` (with `default-features = false`, `features = ["clock"]` if needed for `Utc::now`,
otherwise use the existing `now_ms`) and `chrono-tz` to `Cargo.toml`. `chrono-tz` embeds the
IANA database in the binary, so the musl release and Windows need no system data. Keep
`src/time.rs` as the UTC formatter used by every answer.

**Verify:** `cargo check` and `cargo build --release` succeed; `cargo tree -i chrono-tz` shows
one version.

### Step 2 — Write the shared instant parser

Create `src/web/api/instant.rs` with pure functions and no I/O:

- `parse_tz(value: Option<&str>) -> Result<Tz, Rejection>`: `None` and `""` mean UTC; `Z` and
  `UTC` mean UTC; `±HH:MM` becomes a fixed offset; any other value is parsed with
  `chrono_tz::Tz::from_str`. An unknown name is `400 INVALID_PARAMETER` naming `tz`.
- `parse_instant(name, value, tz, now_ms) -> Result<i64, Rejection>` accepting, in order:
  - `now`;
  - `[+-]N` followed by `s`, `m`, `h`, `d`, `w`, `mo`, or `y`;
  - a plain integer, read as epoch milliseconds;
  - `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM`, `YYYY-MM-DDTHH:MM:SS`, each with an optional `.fff`
    fraction (1–3 digits) and an optional `Z` or `±HH:MM` suffix.
- Resolution rules: `s`/`m`/`h` are exact milliseconds; `d`/`w`/`mo`/`y` are calendar
  additions in the effective zone (`w` = 7 days, `y` = 12 months, a month clamps the
  day-of-month as `chrono` does); an instant without a suffix is read in `tz`; an explicit
  suffix wins over `tz`. A local time inside a DST gap is `400 INVALID_PARAMETER`; an
  ambiguous local time takes the earlier occurrence. Every result must fall inside the
  existing `DOMAIN_MIN_MS..=DOMAIN_MAX_MS`, or the existing out-of-domain rejection fires.
- Keep the parser free of `now_ms()` calls: the caller passes one `now` for the whole request.

**Verify:** `cargo test --lib web::api::instant` covers each form, each unit, a zone with DST,
a gap, an ambiguity, and every refusal.

### Step 3 — Wire the parser into the shared request parameters

In `src/web/api/mod.rs`:

- Delete the local `parse_instant` and have `Params::parse` take the request's single `now`,
  resolve `tz` once, and call `instant::parse_instant` for `from` and `to`. `Params` gains
  `tz: Option<String>` (the resolved zone's display name).
- `Params::parse` already receives the accepted names; pass that list to
  `Rejection::unknown_parameter(name, accepted)` so the message names the parameters the route
  actually accepts.
- Extend `Meta` with `step: Option<u32>` and `tz: Option<&str>`, and render `"step"` next to
  `limit`/`offset` and `"tz"` inside `filters` **only when present**, so existing routes keep
  byte-identical answers for existing requests.
- Add `Rejection::missing_required("target")` and keep all existing codes unchanged.

Update `faults.rs` and `daily.rs` to list `tz` in their accepted parameters and to resolve
`now` once before `Params::parse`; their existing defaulting (`daily`'s 30-day window, the
`to`-in-the-future handling) stays as it is on top of the resolved instants.

**Verify:** `cargo test --test web` stays green; `curl` with `from=2026-11-01T00:00:00Z` and
with `from=1761955200000` answers exactly as before, and `from=-1h&tz=Europe/Rome` is accepted
on `/api/v1/faults`.

### Step 4 — Add the targeted timeline read

Extend `History` in `src/history.rs` with
`fn timeline_of(&self, target: &str, from_ms: i64, to_ms: i64) -> Result<TargetTimeline, HistoryError>;`
documenting that it returns the same segments `timeline` would, for one target only. Implement
it in `src/store/mod.rs` by reusing the state-before and transition queries with a
`WHERE target = ?` predicate, and in `src/sim/history.rs` by selecting the one configured
target. Do not change `timeline`.

**Verify:** `cargo test --test store` asserts `timeline_of` equals the matching lane of
`timeline` for a fixture with two targets; `cargo test --test simulate` covers the simulated
source.

### Step 5 — Write the pure sampler

In `src/web/api/timeseries.rs`, a pure function
`sample_segments(segments: &[TimelineSegment], first_ms: i64, step_ms: i64, count: usize, now_ms: i64) -> Vec<(i64, TargetState)>`
walks the segments with one cursor and emits `count` states:

- sample `k` is at `first_ms + k * step_ms`;
- the state is the one of the segment whose `[from_ms, to_ms)` contains the instant, advancing
  the segment cursor without rescanning;
- when the instant is greater than `now_ms`, the state is `TargetState::Unknown`, whatever the
  segment says.

Also a page helper: `first_sample(from_ms, step_ms) -> i64` using `div_euclid` (the smallest
grid instant `>= from`), `sample_count(from_ms, to_ms, step_ms) -> i64`, and
`page(total, offset, limit) -> (skip, take)` with saturating arithmetic.

**Verify:** unit tests cover: `from` on and off the grid, a `to` that is not a multiple, an
exact-boundary event at a sample instant (the new state wins), a target whose first event is
after `from` (`unknown`), a window wholly after `now` (all `unknown`), `offset == total`, and
`offset > total` (empty page, correct `total`).

### Step 6 — Add the route

In `src/web/api/timeseries.rs`, implement `route(stream, query, source, reply)`:

1. `Params::parse(query, &["target", "from", "to", "step", "limit", "offset", "tz"])`, with
   `step` parsed as an integer `1`–`86400` defaulting to `5`.
2. Require `target`; open the history; resolve the target to its canonical key (`resolve`).
3. Resolve the window: `now` once, `to = params.to.unwrap_or(now)`,
   `from = params.from.unwrap_or(to - 1h)`; `from >= to` is `INVALID_RANGE`.
4. Compute the grid and page. When `take > 0`, call
   `timeline_of(key, page_first, page_last + step_ms)` — never the whole window — and project
   the page with `sample_segments`; when `take == 0`, read nothing.
5. Stream through `Envelope::begin`/`record`/`finish`, one record per sample:
   `{"target": key, "at": "…Z", "atMs": n, "state": "up|down|unknown"}`.
6. Build `Meta` with the resolved `limit`, `offset`, `total`, target, `from`, `to`, `generatedAt`,
   plus `step` and the resolved `tz`.

Register `"/api/v1/timeseries" => timeseries::route(...)` in `src/web/api/mod.rs`.

**Verify:** `cargo test --test web timeseries` covers the happy path, paging, a label as
`target`, a missing/unknown target, `step=0`/`step=86401`/`step=abc`, a future window,
`HEAD`, gzip, and CORS.

### Step 7 — Publish the contract

Add the path and components to `assets/api/openapi.json`:

- `/api/v1/timeseries` with `operationId: listTimeSeriesSamples`, the seven parameters, and the
  same `400`/`404`/`500`/`503` responses as the other data routes.
- `components.parameters.Step` (`integer`, `1..86400`, default `5`) and
  `components.parameters.Tz` (`string`, default `UTC`, examples `Europe/Rome`, `+02:00`).
- `components.schemas.Sample` and `components.schemas.TimeSeriesEnvelope`, and `step`/`tz` as
  optional properties of `Meta`/`Filters`.
- Rewrite the `From`/`To` descriptions with the accepted forms; describe `target` as required
  on this route only.

**Verify:** the existing drift test in `tests/web.rs` passes and names the new route; the
rendered reference at `/api/v1/docs` shows the route with its parameters.

### Step 8 — Update the documents and the records

- `docs/PRODUCT.md`: extend `FR-26` with the route, the sample record, the grid, the paging,
  and the `unknown` rule; add `FR-38` for the instant grammar, `tz`, and the relative forms.
- `docs/ARCHITECTURE.md`: route list, `instant.rs`/`timeseries.rs`, and `timeline_of` in the
  `History` contract.
- `docs/DEVELOPER-GUIDE.md`: the `src/web/api/` tree listing.
- `docs/USER-GUIDE.md`: the route and parameter tables, the accepted instant forms, the
  default window, a `curl` example per form, and the note that future instants are `unknown`.
- `docs/GLOSSARY.md`: **Sample**, **Step**, **Time series**, **Time zone**.
- `docs/ADR.md`: index rows and records for `ADR-0043` and `ADR-0044`.

**Verify:** `rg -n "timeseries|FR-38|ADR-0043|ADR-0044|Time series" docs assets/api/openapi.json`
shows one definition per fact, and the `USER-GUIDE.md` route table lists the new route with its
required `target` and new `step`/`tz`.

### Step 9 — State a sample as `ts` and `v`

The user reviewed the first shape and asked for the compact, conventional record a timeseries
API usually states. Replace `{"target": …, "at": …, "atMs": …, "state": …}` with
`{"ts": "<RFC3339 UTC>", "v": "up|down|unknown"}`:

- The target is not repeated in every record: the answer is already one target, and
  `meta.filters.target` carries the canonical key.
- `atMs` is not repeated either: `ts` is the instant, and a client parses it if it needs a
  number. A numeric field can be added later as an optional additive member.
- Update the OpenAPI `Sample` schema, the route record in `src/web/api/timeseries.rs`, the
  route and drift tests in `tests/web.rs`, the `Sample` glossary entry, the timeseries paragraph
  of `docs/USER-GUIDE.md`, and the timeseries sentence of `FR-26`.
- This is a breaking change to a route that has not been released, so the version stays the
  plan's `minor`.

**Verify:** `cargo test --test web timeseries` and the OpenAPI drift test pass, and
`curl -s '127.0.0.1:8642/api/v1/timeseries?target=icmp://1.1.1.1'` shows records with exactly
`ts` and `v`, the target in `meta.filters.target`, and no `target`/`at`/`atMs`/`state`.

### Step 10 — Read `step` as a suffixed duration

The user asked whether `step` accepts `2m`, `1h`, and the like. It accepts an integer number of
seconds, or one number with the unit `s`, `m`, `h`, or `d` (`30s`, `2m`, `1h`, `1d`); the value
is still bounded to `1`–`86400` seconds after conversion, and the plain integer keeps meaning
seconds for compatibility. `mo`, `y`, `w`, a compound value such as `1h30m`, and a suffix
without a number are refused. `meta.step` continues to state the resolved number of seconds.

**Verify:** `cargo test --test web timeseries` covers `step=2m` (a 120-second grid) and the
refusals `2d`, `0m`, `1mo`, and `5x`; the unit test of `parse_step` covers `5`, `30s`, `2m`,
`1h`, and `1d`.

### Step 11 — Always state `t`, and `ts` only on request

The user asked for the numeric instant to be the standard member and the textual one to be
opt-in:

- Every sample states `t`, the instant in epoch milliseconds, next to `v`.
- A new `ts` parameter adds `ts`, the same instant in RFC 3339, only when the request gives
  `ts` bare or `ts=true`; `ts=false` and an absent parameter state no `ts`.
- When the request names a `tz`, `ts` is localised in that zone (a numeric offset such as
  `+01:00`); without `tz` it is UTC (`Z`). The instant is unchanged, only its rendering is.
- Any other `ts` value is `400 INVALID_PARAMETER` naming `ts`.
- Update the `Sample` and the `Ts` parameter of the OpenAPI document, the route and drift
  tests, the `Sample` glossary entry, the timeseries paragraph of the user guide, and `FR-26`.

**Verify:** `cargo test --test web timeseries` covers `t` by default, `ts` bare in UTC,
`ts=true&tz=Europe/Rome` with its `+01:00` offset, `ts=false` without `ts`, and a refused
`ts=maybe`; the `Zone::format` unit test covers UTC, a fixed offset, and an IANA name.

### Step 12 — Give the timeseries its own page bounds

The user asked for a wider page on the streamed timeseries: a page holds `1000` samples by
default and up to `100000`, while the list routes keep `100`/`1000`.

- `Params::parse_with` takes the default and maximum of `limit`; `Params::parse` keeps calling
  it with the shared `100`/`1000`, so no other route changes.
- The timeseries passes `1000`/`100000`. The answer is written through `Envelope` one chunk per
  sample, so a wide page does not build the document in memory; the samples of the page are
  the only allocation.
- The OpenAPI document gains a `LimitWide` parameter used only by this route, and the user
  guide, `FR-26`, and the architecture note state the two bounds.

**Verify:** `cargo test --test web timeseries` covers the default `1000`, an accepted
`limit=100000`, and a refused `limit=100001` with the wide bound in the message; the existing
test still pins `/api/v1/faults?limit=1001` as a `400`.

### Step 13 — Stream the same page as JSON Lines

The user asked for a streaming form a client can consume record by record, because a JSON
array is awkward for a large page. Add `GET /api/v1/timeseries/stream`:

- Media type `application/x-ndjson`, one object per line terminated by `\n`, no array and no
  commas.
- The first line is `{"meta": {…}}` with the facts of the non-streaming `meta`; every following
  line is one sample, `{"t": …, "v": …}` plus `ts` when requested.
- The request parameters, the window, the grid, the paging, the `ts`/`tz` rendering, the
  `limit` bounds, and the refusals are the ones of `/api/v1/timeseries`.
- `web::api::Lines` writes the body through the same `begin_body`/`Body` as `Envelope`, so the
  answer stays chunked and gzipped, answers `HEAD`, and gains no second streaming path.
- A refusal before the first line is the usual JSON error object; a failure after it closes
  the stream and stays in the log.
- Add the path to the OpenAPI document, and the route to the user guide, `FR-26`, the
  architecture note, the glossary (**JSON Lines**), and a new decision record `ADR-0045`.

**Verify:** `cargo test --test web stream` covers the `application/x-ndjson` header, the meta
first line, one sample per line with the expected values, `ts` localised in the stream, the
`400`/`404` refusals, and `HEAD`; the OpenAPI drift test names the new path.

## Security considerations

- The new dependencies are the main surface: `chrono` and `chrono-tz` are widely used and
  embed their data at build time, but they enlarge the binary and must be pinned by the lock
  file. No network, filesystem, or system tzdb access is added.
- Query values are untrusted input. The parser returns a typed refusal for any malformed
  value and never echoes it into a log line; only the parameter name and a fixed sentence
  reach the client. `Tz::from_str` bounds the accepted names to the compiled database.
- The route is read-only and unauthenticated by design (`BR-8`); it adds no write path and
  respects the existing connection cap and `limit` ceiling, so one request cannot force more
  than `limit` samples into memory.
- Calendar arithmetic cannot overflow: every operation is checked and a result outside the
  existing domain is refused.

## Verification plan

- **Automated:** the unit tests of Steps 2, 4, and 5, the end-to-end cases of Steps 3, 6, and
  7, and the existing suite, which must stay green without weakening any assertion.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
- **Manual:** run `serve` against a seeded database and compare
  `GET /api/v1/timeseries?target=icmp://1.1.1.1&from=-1d&to=now&step=60&tz=Europe/Rome` with
  the same window at `step=5` and with `/api/v1/faults/daily`; confirm the runs line up at the
  expected boundaries and that a window past `now` ends in `unknown`.
- **Rollback:** delete the route and the two modules, remove `timeline_of`, revert the two
  crates, and restore the previous `parse_instant`; no data or schema is affected.

## Open questions

- [x] Does the earlier occurrence rule for an ambiguous DST local time hold, or should the
      request be refused like a gap? **Resolved:** take the earlier occurrence and refuse a
      gap; recorded in ADR-0044 and pinned by a unit test.
- [x] Should `meta.filters.tz` echo the request's spelling (`+02:00`) or the canonical zone
      name (`Europe/Rome` converted to its current offset)? **Resolved:** echo the request's
      spelling exactly, defaulting to `"UTC"` only when the parameter was given but empty;
      an absent `tz` stays absent from the meta.

## Implementation checklist

- [x] Step 1 — Add `chrono` and `chrono-tz`
- [x] Step 2 — Write the shared instant parser
- [x] Step 3 — Wire the parser into the shared parameters
- [x] Step 4 — Add `History::timeline_of`
- [x] Step 5 — Write the pure sampler
- [x] Step 6 — Add the `timeseries` route
- [x] Step 7 — Publish the OpenAPI contract
- [x] Step 8 — Update documents and decision records
- [x] Step 9 — State a sample as `ts` and `v`
- [x] Step 10 — Read `step` as a suffixed duration
- [x] Step 11 — Always state `t`, and `ts` only on request
- [x] Step 12 — Give the timeseries its own page bounds
- [x] Step 13 — Stream the same page as JSON Lines
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan


## Implementation Notes

### Decisions

- The parser restores a `+` that form-decoding turned into a space, so both a literal `+02:00`
  and the standard `%2B02:00` are accepted. A client is still documented to percent-encode `+`.
- `now` is matched case-insensitively; every other form is case-sensitive.
- `meta.step` and `meta.filters.tz` are optional in `Meta` and render only when present, so the
  answers of the existing routes are byte-identical for requests that do not name a zone.
  `tz` echoes the request's spelling; an empty `tz=` is reported as `"UTC"`.
- `Store::timeline_of` answers one lane and an empty lane for a target without a row; the route
  resolves the key first, so it never sees that empty lane.
- Two decision records are added as `ADR-0043` and `ADR-0044`; `ADR-0041` and `ADR-0042` stay
  reserved by the open plans `015` and `016`.

### Deviations

- `chrono` is declared as `chrono = "0.4"` with its default features. The plan suggested
  `default-features = false` plus `clock`/`std`; both are in the default set and the explicit
  feature selection is rejected by this toolchain's `cargo add`, so the default declaration is
  the simpler equivalent.
- The OpenAPI document adds a `TargetRequired` parameter for this route rather than changing the
  shared `Target`: OpenAPI cannot override `required` on a `$ref`, and the shared parameter must
  stay optional for the other routes.
- `src/web/api/targets.rs` is touched although *Affected modules* did not list it: the planned
  `Params::parse` signature gained `now` and the planned `Meta` gained `step`/`tz`, so its two
  call sites had to follow. The change is mechanical (one extra argument, two `None` fields) and
  changes no answer.

### Extensions (user-approved)

- **Step 9 — sample shape `ts`/`v`.** After the first review the user asked to reuse this plan
  instead of opening a new one, and to replace the per-record `target`/`at`/`atMs`/`state` of
  the timeseries with the conventional `{"ts", "v"}`. The target now lives once in
  `meta.filters.target`. The route, the OpenAPI `Sample` schema, the route and drift tests, the
  `Sample` glossary entry, the timeseries paragraph of the user guide, and `FR-26` were
  updated in the same change.
- **Step 10 — suffixed `step`.** The user asked why `step` did not accept `2m`, `1h`, and the
  like. `Params::parse` now reads an integer of seconds or one number with `s`, `m`, `h`, or
  `d`; the resolved value stays `1`–`86400` seconds, plain integers keep meaning seconds, and
  `meta.step` states the resolved seconds. The OpenAPI `Step` schema, the route tests, the
  `Step` glossary entry, `FR-26`, and the user guide were updated in the same change.
- **Step 11 — `t` always, `ts` opt-in.** The user asked for the numeric epoch as the standard
  member and the textual instant only on request, localised in `tz` when one is given. Every
  sample now states `t`; the new `ts` flag adds `ts` (RFC 3339, `Z` in UTC or a numeric offset
  in the requested zone), `ts=false` omits it, and any other value is `400`. The OpenAPI
  `Sample`/`Ts`, the route and drift tests, the `Sample` glossary entry, `FR-26`, and the user
  guide were updated in the same change.
- **Step 12 — wider streamed page.** The user asked for `1000` as the default and `100000` as
  the maximum on the timeseries. `Params::parse_with` now takes the bounds, the list routes
  keep `100`/`1000`, and the route streams each sample as its own chunk. OpenAPI gained
  `LimitWide`; `FR-26`, the architecture note, the user guide, and the route tests were
  updated.
- **Step 13 — JSON Lines stream.** The user asked for a record-by-record form because a JSON
  array is awkward for large pages. `GET /api/v1/timeseries/stream` writes the same plan as
  `application/x-ndjson`: `{"meta": …}` first, one sample per line after it, same parameters
  and refusals, chunked and gzipped through the new `Lines` writer. OpenAPI, `ADR-0045`,
  `FR-26`, the architecture note, the glossary, the developer tree, the user guide, and the
  route/drift tests were updated.

### Discoveries

- `tests/simulate.rs::every_answer_costs_what_it_returns_at_the_widest_settings` failed once
  during the first parallel run with a 70-second read timeout (`Connection reset by peer`) and
  passed both alone and in the final full run. The failure is environmental (parallel process
  load), not a behaviour change: `/healthz` is served by the page and no API code is on that
  path.

### Verification

- `cargo fmt --check` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo test` — 482 lib tests (2 ignored), 66 web, 52 cli, 18 import, 10 config, 9 init, 8
  reload, 7 simulate, 7 manual, 7 notify, 6 monitor, 5 notify channels, 4 store, 3 snapshot, and
  the remaining binaries green; 0 failures.
- New evidence: `web::api::instant` (forms, zones, DST gap and ambiguity, refusals),
  `web::api::timeseries` (grid, paging, `unknown`), the store `timeline_of` equality test, and
  the route tests in `tests/web.rs` (grid, paging, future, zone, errors, OpenAPI drift).


## Review Notes

- **Round**: 1
- **Review date**: 2026-11-18
- **Verdict**: PASS
- **Suggested commit**: `feat(web): add timeseries samples api and zoned instants`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `eb08ee7abe9606ec3e55bd35a20630958c55e949a1ff358bad453f4380db46d1`
- **Disclosure**: this is a self-review performed in the same invocation as the implementation.

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | minor | `src/web/api/targets.rs` | The file is not listed under *Affected modules* but had to follow the planned `Params::parse` and `Meta` signatures; the change is one argument and two `None` fields, with no answer change, and is recorded under Implementation Notes → Deviations. | accepted, documented |

### Gap analysis

- Goals: 8/8 achieved — the route, its grid and paging, the required target, the `unknown` rules, the defaults, the shared grammar and `tz`, the unchanged existing answers, and the echoed `step`/`tz` are all exercised by `tests/web.rs` and the unit tests.
- Scope: in scope after the documented `targets.rs` deviation; every other changed file is under *Affected modules*.
- Deviations: `chrono` uses its default features instead of the suggested `default-features = false`; the OpenAPI document uses a `TargetRequired` parameter because a `$ref` cannot override `required`; `targets.rs` follows the shared signatures.
- Additions: none beyond the plan.
- Deferred: none.

### Known limitations

- One answer is one target; a client loops over `/api/v1/targets`.
- Instants after `now` are `unknown`, so a future window measures nothing yet.
- The page, the CLI, the export, and the import keep the old parsers; the grammar is an API contract only.
- A local time repeated by DST takes its earlier occurrence, which is stated but not selectable.

### Recommendations for future work

- Consider a multi-target series (`target` repeated) if a dashboard asks for it; the paging order and total must be defined first.
- Consider offering the second occurrence of an ambiguous DST hour if a consumer needs it; it would be a new parameter, not a change to the default.


## Review Notes

- **Round**: 2
- **Review date**: 2026-11-18
- **Verdict**: PASS
- **Suggested commit**: `feat(web): add timeseries samples api and zoned instants`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `62f42716aaa21378117ca31825dbf1e0f2a141e3973cf7a6a8e08ab05d4e7f7a`
- **Disclosure**: this is a self-review performed in the same invocation as the implementation.

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| — | — | — | No findings. | — |

### Gap analysis

- Goals: 8/8 unchanged, plus the user-approved Step 9 achieved — a sample is exactly
  `{"ts", "v"}`, the target is stated once in `meta.filters.target`, and no `target`/`at`/
  `atMs`/`state` key remains in a sample.
- Scope: the Step 9 edit touches only files already under *Affected modules*.
- Deviations: none new; the `targets.rs` deviation of Round 1 still stands.
- Additions: Step 9, requested by the user and recorded under Implementation Notes → Extensions.
- Deferred: none.

### Known limitations

- The shape change is breaking for `GET /api/v1/timeseries`, which has not been released.
- A numeric instant is no longer stated per record; a client parses `ts` if it needs one.
- One answer is one target; a client loops over `/api/v1/targets`.

### Recommendations for future work

- Add an optional numeric member such as `tsMs` only if a client proves it needs one; it is
  additive and does not reopen this contract.


## Review Notes

- **Round**: 3
- **Review date**: 2026-11-18
- **Verdict**: PASS
- **Suggested commit**: `feat(web): add timeseries samples api and zoned instants`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `e9e8754142aabd77264edd8b8fbb0cf2fb2d51303455b071fc4c7d28091ae59c`
- **Disclosure**: this is a self-review performed in the same invocation as the implementation.

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| — | — | — | No findings. | — |

### Gap analysis

- Goals: 8/8 unchanged, plus Step 9 (`ts`/`v`) and Step 10 (suffixed `step`) achieved and
  covered by tests.
- Scope: both extensions touch only files already under *Affected modules*.
- Deviations: none new; the `targets.rs` deviation of Round 1 still stands.
- Additions: Step 10, requested by the user and recorded under Implementation Notes →
  Extensions.
- Deferred: none.

### Known limitations

- `step` accepts one unit only (`1h30m` is refused), and `mo`/`y`/`w` are not step units.
- A value that converts past one day is refused, so `2d` is out of range even though it is a
  valid duration elsewhere in the API.
- The shape change of Step 9 is breaking for a route that has not been released.

### Recommendations for future work

- If a coarse grid is ever needed, raise the maximum in one place (`MAX_STEP`) and document
  `w`/`mo`/`y` then; no other layer needs to change.


## Review Notes

- **Round**: 4
- **Review date**: 2026-11-18
- **Verdict**: PASS
- **Suggested commit**: `feat(web): add timeseries samples api and zoned instants`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `eddb067931f49e0f8d572176023351e144a7b23b1061fddd632597dd2b18d742`
- **Disclosure**: this is a self-review performed in the same invocation as the implementation.
- **Note**: this round reviews a user-requested extension (Step 11), not a repeated finding.

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| — | — | — | No findings. | — |

### Gap analysis

- Goals: 8/8 unchanged, plus Steps 9, 10, and 11 achieved and covered by tests.
- Scope: Step 11 touches only files already under *Affected modules*.
- Deviations: none new; the `targets.rs` deviation of Round 1 still stands.
- Additions: Step 11, requested by the user and recorded under Implementation Notes →
  Extensions.
- Deferred: none.

### Known limitations

- `ts` is a rendering of `t`, so it is redundant when it is requested; the default record stays
  numeric-only.
- A bare `ts` and `ts=` are both true; the OpenAPI document states this in prose because it
  cannot express a valueless query flag.
- `step` accepts one unit only (`1h30m` is refused).

### Recommendations for future work

- If a client needs both textual and numeric instants everywhere, `meta.filters.tz` already
  makes the localisation reproducible; no per-record duplicate is needed.


## Review Notes

- **Round**: 5
- **Review date**: 2026-11-18
- **Verdict**: PASS
- **Suggested commit**: `feat(web): add timeseries samples api and zoned instants`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `63466d4874aee4ea21533391f85ec20815e89cdaeb35fd4d24181089aab62d57`
- **Disclosure**: this is a self-review performed in the same invocation as the implementation.
- **Note**: this round reviews a user-requested extension (Step 12), not a repeated finding.

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| — | — | — | No findings. | — |

### Gap analysis

- Goals: 8/8 unchanged, plus Steps 9–12 achieved and covered by tests.
- Scope: Step 12 touches only files already under *Affected modules*.
- Deviations: none new; the `targets.rs` deviation of Round 1 still stands.
- Additions: Step 12, requested by the user and recorded under Implementation Notes →
  Extensions.
- Deferred: none.

### Known limitations

- A 100 000-sample page materialises the page's samples before they are written; at 16 bytes
  each that is about 1.6 MB, and the chunked writer sends them one by one.
- The wide bound applies to the timeseries only; the list routes keep `100`/`1000`.
- The known simulate flake reappeared once under parallel load and passed in the next full run;
  it is unrelated to this change.

### Recommendations for future work

- If the page allocation ever matters, `sample_segments` can become an iterator feeding
  `Envelope::record` directly; the pure projection is already the shape an iterator needs.


## Review Notes

- **Round**: 6
- **Review date**: 2026-11-18
- **Verdict**: PASS
- **Suggested commit**: `feat(web): add timeseries samples api and zoned instants`
- **Quality gate**: tests ✓ | lint ✓ | types ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `c3ee34eda401a842552e9965564ea160876cdd337bda4416f3a0e757458f59b9`
- **Disclosure**: this is a self-review performed in the same invocation as the implementation.
- **Note**: this round reviews a user-requested extension (Step 13), not a repeated finding.

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| — | — | — | No findings. | — |

### Gap analysis

- Goals: 8/8 unchanged, plus Steps 9–13 achieved and covered by tests.
- Scope: Step 13 touches only files already under *Affected modules*; no new source file.
- Deviations: none new; the `targets.rs` deviation of Round 1 still stands.
- Additions: Step 13, requested by the user and recorded under Implementation Notes →
  Extensions.
- Deferred: none.

### Known limitations

- The JSON Lines protocol has one implicit rule: the first line is meta, the rest are samples;
  there is no per-line `type` discriminator.
- A mid-stream failure cannot change the status; the connection closes and the reason stays in
  the log, as on every chunked route.
- The known simulate flake remains environmental and unrelated.

### Recommendations for future work

- If a consumer needs to multiplex several targets in one stream, add a `target` member to the
  sample line and define the ordering; the current one-target contract is deliberate.

## Closure

- **Closed**: 2026-11-18
- **Version**: 0.14.0 → 0.15.0
- **Executed by**: build (`deepseek/deepseek-flash`)
- **Tokens**: 61,023,803 (input 711,656, output 104,828, reasoning 83,031, cache read 60,124,288)
- **Cost**: $0.399837
