---
title: Keep a name, a key, a code, an instant and an average unbroken in the tables
date: 2026-11-15
status: draft
type: fix
impact: low
area: web
version_bump: patch
tags: [web, css, tables, wrapping, typesetting]
depends_on: []
branch_recommended: false
---

## Summary

The two tables of the page — the last 10 faults, drawn on the timeline and on the target
page, and the per-day table of the target page — are made to break only where a reader can
still read them. A target name, a target key (the URL), the code of a cause, the date half
of an instant, the time half of an instant, and the average `up` run with its spread
(`7h54±6h12`) are each one unbreakable token. An instant may still fall on two lines, the
date above the time, because that is the only break the layout offers. Nothing else about the
page changes: the data, the CSV export, the REST API and the visual identity of the pages stay
as they are, and a table that needs more room widens and scrolls inside its own box instead of
moving the page.

## Why

A table cell is laid out with `white-space: normal`, so the browser may break a line at any
space and after any hyphen. On a narrow window — or with the fixed 15/30/45/10 per cent
shares the fault table currently uses — this produces rows a reader has to reassemble:

- a target label such as `edge-router north` breaks into two lines, and the key under it,
  `tcp://edge-router.north.example:443`, breaks after the hyphens and after the scheme;
- `2026-11-15 01:00:00 UTC` breaks after `2026-` or between `01:00:00` and `UTC`, so the
  instant is read as two fragments;
- a cause code such as `HTTP.UNEXPECTED_STATUS` breaks inside the code, which is the one
  string an operator copies into a search;
- `7h54±6h12` breaks after the `±`, turning one figure into two numbers.

The code backs the rule with a contract that documents the opposite: `style.css` sets
`table.faults { table-layout: fixed; }` with the four column shares as the remedy for "a long
name never squeezes the date", and `docs/PRODUCT.md` (`FR-17`), `docs/USER-GUIDE.md` and
`docs/ARCHITECTURE.md` (the `web` component) all restate those shares. A fixed share only
decides where a column starts; it does not stop the browser from splitting what is inside it.
Leaving this unfixed means every narrow window and every long label produces a table that
states a different value than the one stored, one fragment per line.

## Goals

- A target name (`.fault-name`) never occupies more than one line in either table.
- A target key (`.fault-key`, the canonical URL) never occupies more than one line.
- A fault cause code (`button.cause`) never occupies more than one line.
- The average `up` run and its spread (`7h54±6h12`, or a single `3h00`) never occupies more
  than one line; the `max …` line under it keeps breaking on its own `<br>` only.
- An instant states its date and its time as two unbreakable halves: it may break between
  the two, never inside either. This holds for the `started` column of the fault table, for
  the `day (UTC)` column of the per-day table, and for the no-JavaScript fallback text.
- When the unbreakable content of a column is wider than its stated share, the table widens
  and scrolls inside its existing `.table-scroll` wrapper instead of overlapping or clipping
  text; the page itself never scrolls sideways.
- Every existing test that asserts table markup is updated to the new markup, and the quality
  gate (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`) passes.
- Behaviour outside the two tables is unchanged: the chart, the masthead, the stat cards, the
  footer, the CSV export, the REST API and the SQLite schema are untouched.

## Non-goals

- No new wrapping rule anywhere that is not a table cell: the masthead target identity
  (`.target-label`, `.target-address`), the stat cards, the chart's SVG lane labels (already
  shortened by `short()`) and the `window_span` line of the toolbar keep the wrapping they have
  today. `window_span` renders its instants through the helper Step 2 introduces, so its markup
  gains the two halves, but the line keeps breaking exactly where it breaks now.
- The CLI table of `query` (`src/query/render.rs`) is out of scope: a terminal does not wrap
  the padded columns this plan is about.
- The duration column (`.num`, `lasted` / `down time`) and the `last 24 h` tag keep breaking
  as they do today; only the tokens listed in the Goals become atomic.
- No change to the fault data, the CSV export, the REST API, or the database.
- No new dependency, no framework, no client-side rendering: the page stays server-side HTML
  plus the one embedded script.

## Affected modules

**Source**
- `src/web/static/style.css` — add the `.nowrap` rule; add `white-space: nowrap` to
  `.fault-name`, `.fault-key`, and `button.cause`; change `table.faults` from
  `table-layout: fixed` to `table-layout: auto` and rewrite the comment that states the four
  column shares so it states the new contract.
- `src/web/static/app.js` — replace the `element.textContent = …` assignment of the localising
  loop with a helper that writes an instant as two `.nowrap` halves for an HTML `<time>`, and
  keeps a plain string for every other element (the SVG chart labels).
- `src/web/page.rs` — a new `instant_halves` / `instant_text` pair replacing `utc_text` where an
  instant is rendered; `faults_table` and `window_span` use it; its `#[cfg(test)]` module is
  updated.
- `src/web/detail.rs` — `value_with_max` wraps the mean-and-spread in `.nowrap`; `days_table`
  marks its `<time>` element `class="nowrap"`; its `#[cfg(test)]` module is updated.

**Tests**
- `src/web/page.rs` (`#[cfg(test)] mod tests`) — update
  `the_window_label_and_the_axis_carry_the_instant_for_the_browser` and
  `the_start_of_a_disconnection_is_localised_like_every_other_instant`, and add the
  stylesheet and markup tests of Steps 1 and 2.
- `src/web/detail.rs` (`#[cfg(test)] mod tests`) — update
  `the_table_lists_every_day_newest_first_with_its_down_time`,
  `the_last_faults_sit_above_the_day_by_day_table`,
  `each_day_states_the_average_of_the_up_runs_inside_it`,
  `the_longest_run_sits_next_to_the_mean_it_outweighs`,
  `the_day_still_running_states_the_average_of_the_last_24_hours` and
  `every_instant_is_machine_readable_and_localised_by_the_browser`, and add the test of
  Step 4.
- `tests/web.rs` — one new integration test over the served page, stylesheet and script.

**Documentation / configuration**
- `docs/PRODUCT.md` — `FR-17`: the sentence stating that "the four columns keep a stated share
  of the width — 15, 30, 45 and 10 per cent" and the sentence on the per-day table.
- `docs/ARCHITECTURE.md` — the `web` component section (the `Output` bullet that restates the
  column shares).
- `docs/USER-GUIDE.md` — section **The web page**: the fault-table paragraph ("The four
  columns keep a stated share of the width …") and the per-day table description.

**No change**
- `src/web/templates/page.html`, `src/web/templates/detail.html` — the markup the renderers
  produce already carries the classes; nothing page-level changes.
- `docs/GLOSSARY.md`, `docs/ADR.md`, `docs/DEVELOPER-GUIDE.md`, `NOTES.md` — no term, decision,
  command or deployment step changes.

> **Working tree note for the implementer.** `src/web/static/style.css`, `src/web/static/app.js`,
> `tests/web.rs`, `docs/PRODUCT.md` and `docs/USER-GUIDE.md` already carry uncommitted edits
> that draw an open cause as a note of its row (`tr.fault.expanded`). Those edits are part of
> the branch, not of this plan: build on them, never revert or reformat them away.

## Architecture impact

No architectural change: the work lives inside the presentation of `web` — the stylesheet, the
one embedded script, and the two server-side renderers — behind the same component contract.
The two paragraphs of `docs/ARCHITECTURE.md` that restate the column shares are corrected in
the same change so the document and the code keep saying the same thing.

## Decision-record impact

No new decision record: the change selects no technology and reverses no decision. It refines
an existing one — the server-rendered page with one small stylesheet and one small script
(`docs/ARCHITECTURE.md`, `web`) — and the trade-off it introduces (a table that widens and
scrolls rather than one that clips its content) is a presentation consequence of that
decision, recorded where the shares are stated: the `style.css` comment and `FR-17`.

## Proposed solution

The rule is carried by the stylesheet — one `.nowrap` class plus three existing selectors — and
by markup only where the text to protect is composed of several elements (an instant, and the
mean with its spread). Nothing is protected by a hard-coded width, so a longer label in a
future configuration is covered by the same rule.

### Step 1 — Pin the atomic tokens in the stylesheet

`src/web/static/style.css`:

1. Add, next to the table rules, a one-line rule and its comment:
   `.nowrap { white-space: nowrap; }` — "a name, a key, a code, an instant half or a figure is
   one token: the browser never breaks it; the table widens and scrolls instead".
2. Add `white-space: nowrap;` to the `.fault-name` rule, to the `.fault-key` rule, and to the
   `button.cause` rule.
3. Replace `table.faults { table-layout: fixed; }` with `table.faults { table-layout: auto; }`
   and rewrite the comment above the four `nth-child` width rules: the percentages are the
   share a column *starts* with, and a column whose unbreakable content needs more widens the
   table, which the `.table-scroll` wrapper scrolls sideways instead of breaking the text.
   The four percentage rules themselves stay as they are.

Also add the stylesheet half of the new test next to
`a_table_announces_where_it_begins_and_keeps_its_header` in `src/web/page.rs`:

```
fn the_tokens_of_a_cell_are_never_broken()
```

asserting that `stylesheet()` contains `.nowrap { white-space: nowrap; }`, that the
`.fault-name` block, the `.fault-key` block and the `button.cause` block each contain
`white-space: nowrap`, that it no longer contains `table-layout: fixed`, and that it still
contains `.table-scroll { overflow-x: auto; }`.

**Verify:** `cargo test --lib web::page::tests::the_tokens_of_a_cell_are_never_broken` passes.

### Step 2 — Render an instant as a date half and a time half, server side

`src/web/page.rs`: split the rendering of an instant into two helpers beside the existing
`utc_text`

- `fn utc_halves(at_ms: i64) -> (String, String)` — from `format_rfc3339(at_ms)`, the date
  (`&stamp[0..10]`) and the time with its zone (`&stamp[11..19]` plus `" UTC"`);
- `fn instant_text(at_ms: i64) -> String` — `<time datetime="{rfc3339}"><span
  class="nowrap">{date}</span> <span class="nowrap">{time} UTC</span></time>`.

`utc_text` stays as the plain-string form, now written as the two halves joined by one space, so
the three shapes of one instant — the table cell, the toolbar line, and the SVG tooltip of a
chart band (`utc_text`, used at the `rect` tooltip of `chart`) — come from one source and cannot
drift apart. The tooltip must keep a plain string: it lives in an SVG `<title>`, which holds no
elements.

`faults_table` replaces its `<td><time datetime="{at}">{shown}</time></td>` with the output of
`instant_text(fault.started_at_ms)`, and `window_span` replaces its two `<time …>{…}</time>`
fragments with the same helper (a refactor of that one call site: the toolbar line keeps
wrapping exactly as it does now, but its instants gain the same two halves).

Update the two assertions that pin the old markup — in
`the_start_of_a_disconnection_is_localised_like_every_other_instant`
(`<td><time datetime="1970-01-01T01:00:00.000Z">1970-01-01 01:00:00 UTC</time></td>`) and in
`the_window_label_and_the_axis_carry_the_instant_for_the_browser`
(`<time datetime="1970-01-01T00:00:00.000Z">1970-01-01 00:00:00 UTC</time>`) —
and add `fn an_instant_may_break_between_its_date_and_its_time()` asserting the rendered row
contains `<td><time datetime="1970-01-01T01:00:00.000Z"><span class="nowrap">1970-01-01</span>
<span class="nowrap">01:00:00 UTC</span></time></td>`.

The no-JavaScript page now carries the two halves too, so the guarantee holds without the
script.

**Verify:** `cargo test --lib web::page::tests` passes, including the new test.

### Step 3 — Split the localised instant the same way in the browser

`src/web/static/app.js`, in the localising loop at the end of the file:

1. Keep `FORMATS` and the `label` helper for the plain-string case.
2. Add `const DATE_PARTS = new Set(['weekday', 'era', 'year', 'month', 'day'])` and a helper
   `halves(instant, kind)` that formats with `Intl.DateTimeFormat(undefined, FORMATS[kind])`
   **`formatToParts`**, walks the parts once and appends each part to the date half or the time
   half (`DATE_PARTS` ⇒ date; everything else, literals included, ⇒ the half of the part before
   it, `date` for kinds `date` and `day`, `time` for kind `time`, `date` to open for `datetime`
   and `daytime`), then trims both halves.
3. Add `writeInstant(element, instant, prefix, suffix)`:
   - when `element` is an HTML `<time>` element, set `element.replaceChildren()` the date half
     and the time half as two `<span class="nowrap">` separated by a `' '` text node, skipping a
     half that is empty and trimming the joining space;
   - otherwise (the SVG `<text data-datetime>` labels of the chart, which cannot hold elements)
     assign `element.textContent = prefix + joined + suffix` exactly as today.

   The `until` case joins the two instants with `' → '` as it does now, each instant keeping its
   two halves, so a reader may break before the arrow, after it, or between a date and its time
   — never inside either.

**Verify:** `rg -n "formatToParts|nowrap" src/web/static/app.js` shows the new helper, and the
integration test written in Step 6 asserts that `/app.js` as served contains `formatToParts` and
`nowrap`. The chart labels keep working: open the page and confirm the axis labels are still
plain text (a `formatToParts` result inside an SVG `<text>` would throw).

### Step 4 — Keep the average and the day of the per-day table whole

`src/web/detail.rs`:

1. `value_with_max` wraps the mean in `.nowrap`:
   `<span class="nowrap">{format_runs(runs)}</span>` followed by the existing
   `<br><span class="sub">max {…}</span>`, so the spread stays on one line and the `max` line
   keeps starting after the `<br>`.
2. `days_table` renders its day cell as
   `<td><time datetime="{date}" data-format="date" class="nowrap">{&date[..10]}</time></td>`.

Update the five assertions that pin the old markup (`the_table_lists_every_day_newest_first_with_its_down_time`,
`the_last_faults_sit_above_the_day_by_day_table`, `each_day_states_the_average_of_the_up_runs_inside_it`,
`the_longest_run_sits_next_to_the_mean_it_outweighs`,
`the_day_still_running_states_the_average_of_the_last_24_hours`) and add
`fn the_mean_and_the_day_are_never_broken()` asserting that the rendered page contains
`<span class="nowrap">3h00</span><br><span class="sub">max 3h00</span>` and
`data-format="date" class="nowrap"`.

**Verify:** `cargo test --lib web::detail::tests` passes.

### Step 5 — Update the documents that state the column contract

- `docs/PRODUCT.md`, `FR-17`: replace the sentence "The four columns keep a stated share of the
  width — 15, 30, 45 and 10 per cent — so a long name never squeezes the date and a short code
  never leaves a gap, and every row is the same height, so the table reads as a grid." with one
  that states the shares as the starting point and the rule that follows from it: the name, the
  key, the cause code, the date half and the time half of an instant, and the average `up` run
  with its spread are never broken, an instant may fall on two lines with the date above the
  time, and a column that cannot hold its token widens the table, which scrolls sideways under
  its own header. In the same requirement, extend the per-day-table sentence ("A table with one
  row per day states the number of faults, the total down time, and the average `up` run of
  that day, with the longest run of the same window under the mean.") with the rule that the
  average and its spread are never broken and that the day is a date that never breaks.
- `docs/ARCHITECTURE.md`, `web` → `Output`: replace "the four columns keeping a stated share of
  the width" with the same rule in one clause.
- `docs/USER-GUIDE.md`, **The web page**: rewrite the fault-table sentence ("The four columns
  keep a stated share of the width (15, 30, 45 and 10 per cent) and every row is the same
  height.") so the user learns what they will see: the columns start at those shares, a name, a
  key, a cause code, each half of an instant and the average `up` run are never split, an
  instant may occupy two lines — the date above the time — and a table too wide for the window
  scrolls sideways. Keep the guide's three-line summary untouched and keep the wording free of
  the internal class names.

**Verify:** `rg -n "15, 30, 45" docs/ src/` returns only the rewritten sentences (the shares are
still stated, nothing still claims a fixed share that the browser must honour); reading the
three edited passages against `src/web/static/style.css` shows the same contract.

### Step 6 — Prove it over the served files

`tests/web.rs`: add `fn no_cell_of_a_table_breaks_a_token()`, starting the server the way the
neighbouring tests do, requesting `/` and `/target?target=…&days=30d` (the seeded target), and
asserting that both carry `<span class="nowrap">` in the start cell, that the target page carries
`data-format="date" class="nowrap"` and `<span class="nowrap">` around a mean, that `/style.css`
carries `.nowrap { white-space: nowrap; }` and no `table-layout: fixed`, and that `/app.js`
carries `formatToParts` and `nowrap`.

**Verify:** `cargo test --test web` passes.

## Security considerations

Not applicable: no input path, no query, no credential and no socket changes. The change adds no
dynamic content — every value it wraps is already escaped where it is rendered — and the script
continues to write text nodes (`textContent` of the created spans and of the SVG labels), never
`innerHTML`, so no new injection surface is opened.

## Verification plan

- **Automated:** the four new tests and the updated assertions listed in the steps; each one
  fails on today's code (the stylesheet has no `.nowrap`, the cells carry one plain `<time>`,
  the mean is unwrapped) and passes after the change.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  (the `quality.all` command of `.spec-workflow.yml`).
- **Manual:** with a debug build (`serve --listen 127.0.0.1:8642 --database …`) and a target
  whose label and key are long, narrow the browser window until the fault table is under
  pressure and confirm: the label, the key, the cause code, the average and each half of the
  instant stay whole; an instant may show the date above the time; the fault table scrolls
  sideways inside its own box rather than moving the page; the chart axis labels still render;
  the open-cause note still joins its row (the uncommitted `tr.fault.expanded` work).
- **Rollback:** revert the four source files and the three documents; no data migration, no API
  change and no schema change is involved, so a revert restores the previous presentation
  exactly.

## Open questions

- [ ] When a column cannot hold its token, is widening the table and scrolling it sideways
      (`.table-scroll`) the wanted answer, or should the column shares stay authoritative and
      the text be allowed to overflow its cell? *Best guess: widen and scroll* — it is the only
      option that never breaks and never hides a value. Owner: maintainer.
- [ ] Do the duration column (`lasted`, `down time`) and the `last 24 h` tag need the same
      rule? *Best guess: no* — a duration is short and never the string an operator copies.
      Owner: maintainer.
- [ ] Must the masthead target identity follow the same rule? *Best guess: no* — it is not a
      table and its two parts wrap on their own line today. Owner: maintainer.

## Implementation checklist

- [ ] Step 1 — `.nowrap` rule, the three nowrap selectors, `table-layout: auto`, stylesheet test
- [ ] Step 2 — `instant_text` helper, `faults_table` and `window_span`, markup tests
- [ ] Step 3 — browser splits a localised instant into two nowrap halves
- [ ] Step 4 — `.nowrap` around the mean, `class="nowrap"` on the day cell, detail tests
- [ ] Step 5 — `FR-17`, the `web` section of the architecture, the user guide
- [ ] Step 6 — integration test over the served page, stylesheet and script
- [ ] All open questions resolved
- [ ] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [ ] Documentation listed under Affected modules is updated
- [ ] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [ ] `## Implementation Notes` added or updated in this plan
