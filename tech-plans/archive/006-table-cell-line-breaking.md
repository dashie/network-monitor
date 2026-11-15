---
title: Keep every table value unbroken and wrap the four widgets first to last
date: 2026-11-15
status: done
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
date above the time, because that is the only break the layout offers.

The row of four widgets at the top of the target page gets a stated wrapping order instead of
leaving it to the browser: four across while they fit, then two and two, then the first widget
— the state — alone on its row above the other three. Nothing else about the page changes: the
data, the CSV export, the REST API and the visual identity of the pages stay as they are, and
neither a table nor a widget ever splits the value it states.

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

The row of widgets above the chart has the same problem one level up. `.stats` is a flex row of
`flex: 1 1 150px` tiles, so the browser decides how many share a line: at about 700 px it drops
to three-and-one, then two-and-two, then one per line. The state — the tile that says whether
the target is up at all, and the only coloured one — is therefore as likely as any other to be
pushed onto a line with nothing else, and the shape of the row changes at widths nobody chose.
The reader has to re-find the state tile at every window size, and a four-tile row that becomes
three-and-one reads as a ragged row rather than as a layout.

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
- The four widgets of the target page keep their order — state, faults, down time, average `up`
  run — and take exactly one of three stated shapes: four across, then two and two, then the
  state alone on its row above the other three. No width ever produces three-and-one, and no
  widget is ever moved below a widget that follows it.
- The figure of a widget (`7h54±6h12`, `4h10`, `12m`, `0s`) never occupies more than one line;
  the label under it may still wrap between its words.
- Under 700 px — the width at which four columns stop fitting — a fault row states its four
  values as two columns of two rows: the target with its instant under it in the first column,
  the cause with its duration under it in the second. The header row takes the same mapping, so
  every label stays above the value it names, and a note row that opens under a fault spans both
  columns.
- Under the same 700 px a day-by-day row states the day with its fault count beside it on the
  first line, and the down time with the average `up` run on the second. Its header row takes the
  same mapping.
- Neither layout loses what it states: the table keeps its table semantics for a screen reader
  after the display changes, and no value of either table is clipped or overlaps another.
- Behaviour outside the two tables and the widget row is unchanged: the chart, the masthead,
  the footer, the CSV export, the REST API and the SQLite schema are untouched, and the four
  widgets keep the text, the order and the colours they have today.
- Every existing test that asserts table or widget markup is updated to the new markup, and the
  quality gate (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
  passes.

## Non-goals

- No new wrapping rule anywhere that is not a table cell or a widget: the masthead target
  identity (`.target-label`, `.target-address`), the chart's SVG lane labels (already shortened
  by `short()`) and the `window_span` line of the toolbar keep the wrapping they have today.
  `window_span` renders its instants through the helper Step 2 introduces, so its markup gains
  the two halves, but the line keeps breaking exactly where it breaks now.
- The three widgets other than the state keep the surface they have today (no new colour, no
  reordering, no new widget); the plan changes where they sit, never what they say.
- No card-per-row layout and no second copy of the data: the stacked layouts are the same
  `<table>`, the same rows and the same cells, rearranged by CSS at one width. Nothing is hidden,
  repeated, reordered in the markup, or rendered by the script.
- No responsive work above 700 px, and none outside the two tables and the widget row: the
  statement line of the toolbar, the chart and the footer keep the layout they have.
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
  `.fault-name`, `.fault-key`, `button.cause` and `.stat b`; turn `.stats` into a grid with the
  three stated shapes and drop `flex: 1 1 150px` from `.stat`; change `table.faults` from
  `table-layout: fixed` to `table-layout: auto` and rewrite the comment that states the four
  column shares so it states the new contract; and add the one `@media (max-width: 700px)`
  block that turns both tables into the two-column layouts of Steps 8 and 9.
- `src/web/static/app.js` — replace the `element.textContent = …` assignment of the localising
  loop with a helper that writes an instant as two `.nowrap` halves for an HTML `<time>`, and
  keeps a plain string for every other element (the SVG chart labels).
- `src/web/page.rs` — a new `utc_halves` / `instant_text` pair beside `utc_text`;
  `faults_table` and `window_span` use it; `faults_table` states the explicit table roles; its `#[cfg(test)]` module is updated.
- `src/web/detail.rs` — `value_with_max` wraps the mean-and-spread in `.nowrap`; `days_table`
  marks its `<time>` element `class="nowrap"`, gives the table a `.days` class and states the
  explicit table roles; its `#[cfg(test)]` module is
  updated.

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
  Step 4, the widget test of Step 5 and the responsive-table test of Step 9.
- `tests/web.rs` — one new integration test over the served page, stylesheet and script, extended
  in Step 10 with the responsive block of the served stylesheet.

**Documentation / configuration**
- `docs/PRODUCT.md` — `FR-17`: the sentence stating that "the four columns keep a stated share
  of the width — 15, 30, 45 and 10 per cent", the sentence on the per-day table, the sentence
  that describes the four widgets of the target page, and a new statement of the two stacked
  layouts and the width they take.
- `docs/ARCHITECTURE.md` — the `web` component section (the `Output` bullet that restates the
  column shares and the widgets).
- `docs/USER-GUIDE.md` — section **The web page**: the fault-table paragraph ("The four
  columns keep a stated share of the width …"), the per-day table description, and the
  description of the widgets at the top of the target page.

**No change**
- `src/web/templates/page.html`, `src/web/templates/detail.html` — the markup the renderers
  produce already carries the classes, and the widget flow is decided by the stylesheet alone;
  no HTML changes.
- The four widgets, their order and their text stay as they are: no widget is added, removed,
  recoloured or reworded by any step of this plan.
- `docs/GLOSSARY.md`, `docs/ADR.md`, `docs/DEVELOPER-GUIDE.md`, `NOTES.md` — no term, decision,
  command or deployment step changes.

> **Note for the implementer on a moving tree.** The plan was written against `0.6.0` and the
> branch has moved since (`577f450` `simulate`, and the lane `priority`, the held-reload banner
> and the toolbar-based target identity before it). Every fact it relies on was re-checked
> against the tree: `.stats` is still a flex row, `.stat` still carries `flex: 1 1 150px`,
> `table.faults` is still `table-layout: fixed`, `utc_text` still renders every instant, no
> `.nowrap` exists, and `src/web/templates/detail.html` still holds the four widgets in order.
> The two passages most likely to drift again are the widget row and the toolbar sentence in
> `docs/USER-GUIDE.md` and `docs/ARCHITECTURE.md`; re-read them before editing, and if a quoted
> sentence no longer matches, edit the sentence that says the same thing rather than the
> quotation.

## Architecture impact

No architectural change: the work lives inside the presentation of `web` — the stylesheet, the
one embedded script, and the two server-side renderers — behind the same component contract.
The paragraphs of `docs/ARCHITECTURE.md` that restate the column shares and the widgets are
corrected in the same change so the document and the code keep saying the same thing.

## Decision-record impact

No new decision record: the change selects no technology and reverses no decision. It refines
an existing one — the server-rendered page with one small stylesheet and one small script
(`docs/ARCHITECTURE.md`, `web`) — and the two trade-offs it introduces (a table that widens and
scrolls rather than one that clips its content; a widget row laid out by three stated shapes,
built on the CSS grid the stylesheet already has available, rather than by the flow of a flex
row) are presentation consequences of that decision, recorded where the layout is stated: the
`style.css` comments and `FR-17`.

## Proposed solution

The rule is carried by the stylesheet — one `.nowrap` class plus three existing selectors — and
by markup only where the text to protect is composed of several elements (an instant, and the
mean with its spread). The widget row is a grid of four, so its three shapes are three
`grid-template-columns` declarations and not a width chosen by the browser. Nothing is
protected by a hard-coded width, so a longer target label in a future configuration is covered
by the same rule.

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

### Step 5 — State the three shapes of the widget row

The four widgets of the target page are already four `<div class="stat">` children of one
`<div class="stats">`, in the wanted order (`src/web/templates/detail.html`). The shape of the
row is therefore a stylesheet decision, and no markup changes.

`src/web/static/style.css`, replacing the `.stats` / `.stat` block:

1. `.stats` becomes `display: grid;` with `grid-template-columns: repeat(4, 1fr);` and keeps
   `gap: 12px; margin: 20px 0 0;`.
2. `.stat` keeps its padding, border and radius and **loses** `flex: 1 1 150px;` — grid
   replaces the flow, and `1fr` with `auto` minimum is what keeps a tile from being squeezed
   below its own content.
3. `.stat b` gains `white-space: nowrap;` so the figure is one token (the label under it stays
   free to wrap between its words).
4. Two media queries, below the `.stat` rules:

   ```css
   /* Four widgets fit one row until the content is narrower than four readable tiles: they
      then halve, and under the last width the state owns a row of its own and the three
      figures share the next one, so the order never scrambles and no tile is left alone on
      a line with nothing beside it. */
   @media (max-width: 700px) {
     .stats { grid-template-columns: repeat(2, 1fr); }
   }
   @media (max-width: 460px) {
     .stats { grid-template-columns: repeat(3, 1fr); }
     .stat.state { grid-column: 1 / -1; }
   }
   ```

   The thresholds are the widths at which the four tiles stop being readable at a stated
   content width (four tiles of about 150 px plus the gaps); they are layout constants of the
   stylesheet, not configuration.

Grid placement is row-major, so the widgets always fill in source order: at every width the
state is the first cell, and no media query moves a widget above one that precedes it. That is
what turns a browser decision into the stated order: 4 → 2+2 → 1+3, never 3+1 and never a
reordered row.

In `src/web/detail.rs` (`#[cfg(test)] mod tests`), add
`fn the_four_widgets_keep_their_order_and_take_the_three_stated_shapes()` asserting that
`stylesheet()` contains `grid-template-columns: repeat(4, 1fr)`, the two `@media (max-width:`
blocks with `repeat(2, 1fr)` and `repeat(3, 1fr)`, and `.stat.state { grid-column: 1 / -1; }`;
that it no longer contains `flex: 1 1 150px`; and that `target_page(...)` still emits the four
widgets in order (reuse the count of `the_first_widget_states_the_state_and_how_long_it_has_held`).

**Verify:** `cargo test --lib web::detail::tests` passes.

### Step 6 — Update the documents that state the column contract

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
  average and its spread are never broken and that the day is a date that never breaks. In the
  same requirement, extend the sentence that introduces the widgets of the target page — "the
  totals of the period sit under it, opened by the current state of the target and how long it
  has held — white on a tile of the colour the chart gives that state, the only widget drawn
  that way —, then the faults, the down time, and the average `up` run of the period" — with
  the order of the four widgets and the three shapes they take as the page narrows.
- `docs/ARCHITECTURE.md`, `web` → `Output`: replace "the four columns keeping a stated share of
  the width" with the same rule in one clause, and name the three shapes of the widget row in
  the same bullet.
- `docs/USER-GUIDE.md`, **The web page**: rewrite the fault-table sentence ("The four columns
  keep a stated share of the width (15, 30, 45 and 10 per cent) and every row is the same
  height.") so the user learns what they will see: the columns start at those shares, a name, a
  key, a cause code, each half of an instant and the average `up` run are never split, an
  instant may occupy two lines — the date above the time — and a table too wide for the window
  scrolls sideways. In the `/target?target=<key>&days=30d` bullet of the same section, extend
  the sentence "Under the toolbar, four widgets: the first states the state of the target …,
  and the other three state the faults, the down time, and the average `up` run of the period."
  with the order the four keep and the three shapes they take as the window narrows — four
  across, then two and two, then the state alone above the other three. Keep the guide's
  three-line summary untouched and keep the wording free of the internal class names.

**Verify:** `rg -n "15, 30, 45" docs/ src/` returns only the rewritten sentences (the shares are
still stated, nothing still claims a fixed share that the browser must honour); reading the
three edited passages against `src/web/static/style.css` shows the same contract, including the
two breakpoints and the widget order.

### Step 7 — Prove it over the served files

`tests/web.rs`: add `fn no_cell_of_a_table_breaks_a_token()`, starting the server the way the
neighbouring tests do, requesting `/` and `/target?target=…&days=30d` (the seeded target), and
asserting that both carry `<span class="nowrap">` in the start cell, that the target page carries
`data-format="date" class="nowrap"` and `<span class="nowrap">` around a mean, that `/style.css`
carries `.nowrap { white-space: nowrap; }`, `grid-template-columns: repeat(2, 1fr)` and
`.stat.state { grid-column: 1 / -1; }` and no `table-layout: fixed`, and that `/app.js`
carries `formatToParts` and `nowrap`.

**Verify:** `cargo test --test web` passes.

### Step 8 — Collapse the fault table into two columns under 700 px

The collapsed layout is the same table: same `<table>`, same `<tr>`, same four `<td>`, only
placed differently. Nothing is duplicated and nothing is hidden.

1. `src/web/page.rs`, `faults_table`: state the roles the display override would otherwise
   erase — `role="table"` on the `<table>`, `role="rowgroup"` on `<thead>` and `<tbody>`,
   `role="row"` on each `<tr>` (the cause note row included), `role="columnheader"` on each
   `<th>`, `role="cell"` on each `<td>`. They match the implicit semantics, so the wide layout
   is unaffected and a screen reader keeps reading a table in the collapsed one.
   `colspan="4"` on the note and empty-state rows stays: it is what the wide layout needs, and
   the collapsed layout places those cells with `grid-column: 1 / -1`.
2. `src/web/static/style.css`, one block at the end of the table rules:

   ```css
   /* Under the width where four columns stop fitting, a row states its four values as two
      columns of two rows: the target with its instant under it, the cause with its duration
      under it. The header takes the same mapping, so every label stays above its value. */
   @media (max-width: 700px) {
     table.faults, table.faults thead, table.faults tbody,
     table.faults tr { display: block; }
     table.faults tr {
       display: grid;
       grid-template-columns: minmax(0, 1fr) auto;
       column-gap: 12px;
       row-gap: 2px;
       align-items: baseline;
       padding: 8px 0;
     }
     table.faults th, table.faults td {
       padding: 0;
       border-bottom: 0;
     }
     table.faults tr { border-bottom: 1px solid var(--rule); }
     table.faults tr:last-child { border-bottom: 0; }
     table.faults tr > *:nth-child(1) { grid-area: 1 / 1; }
     table.faults tr > *:nth-child(2) { grid-area: 2 / 1; }
     table.faults tr > *:nth-child(3) { grid-area: 1 / 2; }
     table.faults tr > *:nth-child(4) { grid-area: 2 / 2; }
     table.faults td[colspan] { grid-column: 1 / -1; }
     table.faults th { position: static; border-top: 0; }
     tr.fault { height: auto; }
     table.faults .ongoing { display: inline; }
   }
   ```

   `tr.fault { height: 52px }` is what kept every row of the wide grid the same height; in the
   collapsed layout the row is as tall as its two lines, so it goes back to `auto`. The sticky
   header becomes static because it is now a two-line block whose second line would cover the
   first row it belongs to.

**Verify:** `cargo test --lib web::page::tests` passes, and opening `/` at 390 px shows each
fault as two columns — `edge-router north` over `icmp://1.1.1.1` over the instant on the left,
`ICMP.NO_REPLY` over `3m` on the right — with no row clipped.

### Step 9 — Collapse the day-by-day table into a 2×2 under 700 px

The same treatment for the four columns of the per-day table, laid out as the request states:
the day with its fault count on the first line, the down time with the average `up` run on the
second.

1. `src/web/detail.rs`, `days_table`: give the table `class="days"` and state the same roles as
   Step 8 (`role="table"`, `role="rowgroup"`, `role="row"`, `role="columnheader"`,
   `role="cell"`).
2. `src/web/static/style.css`, inside the same `@media (max-width: 700px)` block:

   ```css
     table.days, table.days thead, table.days tbody, table.days tr { display: block; }
     table.days tr {
       display: grid;
       grid-template-columns: minmax(0, 1fr) auto;
       column-gap: 12px;
       row-gap: 2px;
       align-items: baseline;
       padding: 8px 0;
       border-bottom: 1px solid var(--rule);
     }
     table.days th, table.days td { padding: 0; border-bottom: 0; }
     table.days tr:last-child { border-bottom: 0; }
     table.days tr > *:nth-child(1) { grid-area: 1 / 1; text-align: left; }
     table.days tr > *:nth-child(2) { grid-area: 1 / 2; }
     table.days tr > *:nth-child(3) { grid-area: 2 / 1; text-align: left; }
     table.days tr > *:nth-child(4) { grid-area: 2 / 2; }
     table.days td[colspan] { grid-column: 1 / -1; }
     table.days th { position: static; border-top: 0; }
   ```

   The down time is a `.num` cell that lands in the left column: `text-align: left` on the first
   and third children keeps it under the day it belongs to instead of hanging at the right end of
   a column that is not numeric. The fault count and the average, in the right column, stay
   right-aligned.
3. Add `fn both_tables_state_their_roles_and_their_two_columns()` in
   `src/web/detail.rs` (`#[cfg(test)] mod tests`) asserting that the served stylesheet contains
   the `@media (max-width: 700px)` block with the four `grid-area` rules of each table, that
   `table.days` and `role="row"` are in the rendered page, and that `role="cell"` is on a day
   cell.

**Verify:** `cargo test --lib web::detail::tests` passes, and at 390 px each day of
`/target?target=…&days=30d` reads `15 nov 2026   59` over `2h30   15m±14m`.

### Step 10 — State the two stacked layouts in the documents and pin them in the served files

- `docs/PRODUCT.md`, `FR-17`: after the sentence on the stated shares, state that under the width
  where four columns stop fitting the fault table states the target with its instant under it in
  one column and the cause with its duration under it in the next, the header taking the same
  mapping; and that the per-day table states the day with its fault count beside it and the down
  time with the average `up` run under them. Same requirement, one sentence each.
- `docs/ARCHITECTURE.md`, `web` → `Output`: one clause naming the two stacked layouts and the
  width they take.
- `docs/USER-GUIDE.md`, **The web page**: extend the fault-table paragraph and the `/target`
  bullet with what a narrow window shows — two columns of two lines, the same values, the same
  order, nothing hidden — plus the note that the header moves with the cells. No class names.
- `tests/web.rs`, `no_cell_of_a_table_breaks_a_token`: extend it with the responsive block of the
  served stylesheet (`@media (max-width: 700px)`, `table.days`, and the four `grid-area` rules of
  the fault table) and with `role="row"` in the served page.

**Verify:** `rg -n "max-width: 700px" src/ docs/` shows the one block in the stylesheet and the
passages that describe it; `cargo test --test web` passes.

## Security considerations

Not applicable: no input path, no query, no credential and no socket changes. The change adds no
dynamic content — every value it wraps is already escaped where it is rendered — and the script
continues to write text nodes (`textContent` of the created spans and of the SVG labels), never
`innerHTML`, so no new injection surface is opened.

## Verification plan

- **Automated:** the five new tests and the updated assertions listed in the steps; each one
  fails on today's code (the stylesheet has no `.nowrap`, no grid, the cells carry one plain
  `<time>`, the mean is unwrapped) and passes after the change.
- **Quality gate:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  (the `quality.all` command of `.spec-workflow.yml`).
- **Manual:** with a debug build (`serve --listen 127.0.0.1:8642 --database …`) and a target
  whose label and key are long, narrow the browser window until the fault table is under
  pressure and confirm: the label, the key, the cause code, the average and each half of the
  instant stay whole; an instant may show the date above the time; the fault table scrolls
  sideways inside its own box rather than moving the page; the chart axis labels still render;
  the open-cause note still joins its row (the uncommitted `tr.fault.expanded` work). Then, on
  `/target`, sweep the same window from wide to narrow and confirm the widget row passes
  through exactly three shapes — four across, two and two, the state alone above the other
  three — with the state always first and no three-and-one row at any width, and that no
  figure of a widget wraps.
- **Rollback:** revert the four source files and the three documents; no data migration, no API
  change and no schema change is involved, so a revert restores the previous presentation
  exactly.

## Open questions

- [x] Which shapes does the widget row take as the page narrows? **Decided: four across, then
      two and two, then the state alone above the other three** — the order the request states,
      and the only one where the state, which is the headline of the page, is never left on a
      line with nothing beside it. Owner: maintainer.
- [x] Below the tightest width the grid keeps its three columns and the row scrolls itself
      rather than breaking a figure or reordering a tile. **Decided during implementation: the
      row scrolls, the page never does** — `.stats` carries `overflow-x: auto`, and the
      tightest step gives up its own gap (12 → 8 px) and tile pad so the three tiles stand
      unclipped from 375 px up; measured on the simulator at 460, 430, 414, 390, 375, 360, 344
      and 320 px. Owner: maintainer.
- [x] When a column cannot hold its token, widening the table and scrolling it sideways is the
      answer. **Decided: yes** — measured on the simulator: at 400 and 340 px the fault table is
      461 px wide, `.table-scroll` scrolls it (scrollWidth 461 > clientWidth 308–368) and
      `document.scrollingElement.scrollWidth` stays equal to the viewport, so the page around it
      never moves. Owner: maintainer.
- [x] Do the duration column (`lasted`, `down time`) and the `last 24 h` tag need the same rule?
      **Decided: no** — a duration is already one short token and measured as one line at every
      width; pinning it would only add rules. Owner: maintainer.
- [x] At which width do the two tables collapse, and does each collapse at its own width?
      **Decided: one width, 700 px, for both tables and for the widget row** — the widget row
      and the tables change shape at the same width, so no window ever mixes a stacked row with
      a four-across one beside it. Owner: maintainer.
- [x] What happens to the header row of a stacked table? **Decided: it stays, mapped into the
      same 2×2 as the cells** — `target` above `started` in the first column and `cause` above
      `lasted` in the second; `day (UTC)` beside `faults` over `down time` beside `avg up`. The
      per-day table runs to hundreds of rows, and hiding its labels would leave the numbers
      unexplained; the mapping also keeps every label above the value it names. Owner: maintainer.
- [x] Must the target identity of the toolbar follow the same rule? **Decided: no** — it is not a
      table cell, it keeps the wrapping it had, and the plan left it untouched. Owner: maintainer.

## Implementation checklist

- [x] Step 1 — `.nowrap` rule, the three nowrap selectors, `table-layout: auto`, stylesheet test
- [x] Step 2 — `instant_text` helper, `faults_table` and `window_span`, markup tests
- [x] Step 3 — browser splits a localised instant into two nowrap halves
- [x] Step 4 — `.nowrap` around the mean, `class="nowrap"` on the day cell, detail tests
- [x] Step 5 — the widget row: grid, the two breakpoints, the state on its own row, `.stat b` nowrap
- [x] Step 6 — `FR-17`, the `web` section of the architecture, the user guide
- [x] Step 7 — integration test over the served page, stylesheet and script
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every
      configuration option, or the plan records why it needs no change
- [x] Step 8 — the fault table as two columns of two under 700 px, with its roles
- [x] Step 9 — the day-by-day table as a 2×2 under 700 px, with its roles and its test
- [x] Step 10 — the two stacked layouts in `FR-17`, the architecture and the user guide
- [x] `## Implementation Notes

### Decisions (second run — the responsive tables)

- **One breakpoint, 700 px, for both tables and the widget row.** Chosen so no window ever
  shows a stacked row beside a four-across one; recorded in the plan's open questions.
- **The header row stays and takes the same mapping as the cells.** The per-day table can hold
  hundreds of rows and would lose every label if its header were hidden; mapping the header
  into the same 2×2 also keeps each label above the value it names.
- **Explicit `role` attributes in the markup.** Overriding `display` on `table`, `thead`,
  `tbody`, `tr`, `th` and `td` erases their implicit roles, so `faults_table` and `days_table`
  now state `table`, `rowgroup`, `row`, `columnheader` and `cell` themselves. They match the
  implicit semantics, so the wide layout is unchanged.
- **The two columns of a stacked row are stated as shares of the row** — `55%/45%` for the
  fault table and `60%/40%` for the per-day table — not as a fit to the content. Each row is a
  grid of its own, so an `auto` column is measured per row; with `auto` the header landed
  41 px away from the values it names (measured), which is the defect this choice removes.
  The gutter between the two columns comes from `padding-left` on the right-hand cells,
  because a `column-gap` would push the shares past the width of the row.
- **The per-day table's `down time` is left-aligned in the stacked form.** It is a `.num` cell
  that lands in the left column, under the date it belongs to; left-aligned it reads as part of
  that day, and only the two right-hand figures stay right-aligned.

### Deviations (second run)

- **`table.faults tr[hidden] { display: none; }` was added**, which the plan did not name. A
  `display: grid` on the row beats the rule that hides a closed cause note, so every note under
  every fault was showing in the collapsed layout. Found by looking at the page, not by a test;
  the panel below the table is what caught it.
- **`.stats` keeps the `overflow-x: auto` and the tighter spacing of the first run**; the
  responsive block was appended after them and does not touch the widget row.

### Discoveries (second run)

- **A header cannot align with a body when every row is its own grid.** The first
  implementation used `grid-template-columns: minmax(0, 1fr) auto` on each `tr`; the header
  cells were then measured against their own row, and `cause`/`faults` sat tens of pixels away
  from the values they label. The fix is a content-independent share, verified by measuring the
  left edge of every header cell against the cell below it at 390 px.
- **`display: grid` beats `[hidden]`**, which is why a closed cause note became visible. The
  stylesheet now restates the hiding for the collapsed layout.
- **The page's own tooling resizes the viewport while taking screenshots**, so a measurement
  must be read from `window.innerWidth` in the same evaluate call; two screenshots taken after
  a resize looked wrong for that reason and not because of the layout.

### Verification performed (second run)

- `cargo fmt --check` → pass; `cargo clippy --all-targets -- -D warnings` → pass;
  `cargo test` → 471 tests, 0 failed, 10 ignored.
- Browser, 390 px, against `simulate`: the fault row measured `target`/`started` at x=16 (two
  lines) and `cause`/`lasted` at x=213, with the header cells at exactly 16 and 213; the day row
  measured `date`/`faults` on y=1023 and `down time`/`avg up` on y=1048, with the header mapped
  to the same two lines. A closed cause note stayed hidden; opening one showed it across the
  full 358 px of the table. The page never scrolled sideways.
- Browser, 900 px: rows stayed `table-row`, the fault row kept its 52 px height, the header
  stayed `sticky`, and nothing of the collapsed layout applied.
` added or updated in this plan

## Implementation Notes

### Decisions

- **The widget row scrolls itself, never the page.** `.stats` carries `overflow-x: auto`, so a
  row of tiles that cannot fit is clipped by its own box and scrolled, exactly as a wide table
  is. The plan's best guess ("accept the sideways overflow") would have let `document` grow:
  measured at 380 px, the page scrolled 5 px to the right. The row now scrolls instead.
- **The tightest step gives up its own spacing.** At ≤460 px the grid uses `gap: 8px` and
  `.stat { padding: 8px 10px }`. With the wide-page spacing the three tiles needed 369 px of
  content and only had 348 at 380 px, so the third tile was cut mid-figure — on the widths real
  telephones have. With the tighter spacing the 1+3 shape is unclipped from 375 px up. No type
  size changes, so the tiles look the same, only closer together.
- **Breakpoints 700 px and 460 px** are layout constants of the stylesheet, recorded where they
  are declared; they are not configuration and no profile documents them.
- **`utc_text` stays**, re-expressed through `utc_halves`, because the chart's SVG `<title>`
  tooltips cannot hold elements: the table cell gets `instant_text`, the tooltip keeps the
  plain string, and both halves come from one function.

### Deviations

- **The `label` helper of `app.js` was removed, not kept** as Step 3 said. With one
  `writeInstant` that handles both an HTML `<time>` and an SVG `<text>`, nothing was left to
  call `label`, and keeping a dead helper is worse than dropping it. The plain-string path
  lives in the `else` branch of the same loop.
- **`value_with_max` wraps the mean in `.nowrap` in both branches**, not only the one with a
  longest run, so a cell that states a mean and no `max` line is covered by the same rule.
- **The Step 5 test asserts the two facts of the 460 px block separately** (`@media (max-width:
  460px)` and `repeat(3, 1fr); gap: 8px;`), because a comment now sits inside that block and an
  exact multi-line match would break the next time the comment is edited.
- **`src/web/detail.rs`'s test module gained `use crate::web::templates::stylesheet;`** — the
  widget test reads the stylesheet, and the module did not import it.

### Discoveries

- **`styles.css` was not the only place the contract lived**: `docs/PRODUCT.md` `FR-17`, the
  `web` bullet of `docs/ARCHITECTURE.md` and two passages of `docs/USER-GUIDE.md` all restated
  the fixed shares. All four are updated in this change.
- **The tree moved under the plan** (`577f450` `simulate`, the lane `priority`, the held-reload
  banner, the toolbar-based target identity). Every string the plan quotes was re-checked before
  editing; the widget description of `docs/ARCHITECTURE.md` and the `/target` bullet of the user
  guide had been rewritten by those commits, so the sentences edited are the ones that now say
  the same thing.
- **`tests/simulate.rs::another_seed_serves_another_history` failed once** while the suite ran
  repeatedly, and passed on ten consecutive re-runs of that suite. It compares two seeds through
  `/api/v1/faults` and never reads the page, the stylesheet or the script, so this change cannot
  reach it; it is recorded here as an intermittent test of an untouched module, not fixed.
- **The simulator makes this class of defect measurable**: `simulate --target 'icmp:1.1.1.1,label=edge-router north'`
  produces the long label and the real cause codes the plan's manual step asks for. The
  verification below used it, and the temporary server and screenshots were removed afterwards.

### Extensions

- None beyond the plan, except the `overflow-x: auto` and the tighter spacing of the last
  breakpoint, both recorded under Decisions and both approved by the resolution of the plan's
  second open question.

### Verification performed

- `cargo fmt --check` → pass; `cargo clippy --all-targets -- -D warnings` → pass;
  `cargo test` → 470 tests, 0 failed, 10 ignored.
- `node --check src/web/static/app.js` → pass, and the page was reopened after the final edit to
  confirm the three shapes of the script still hold: a table cell localises into two `nowrap`
  spans, the chart's axis labels stay plain text, and a band tooltip stays one string.
- Browser measurement against the simulator (`/` and `/target?…&days=30d`) at 900, 760, 700,
  520, 470, 460, 430, 414, 401, 400, 390, 380, 375, 360, 344, 340, 320 px: the name, the key,
  the cause code, each half of an instant and the mean measured one line box at every width;
  the widget row measured 4, then 2+2, then 1+3 with the state always first; the page never
  scrolled sideways. At 390 px the started cell stacked as `15 nov 2026` over `17:58:59`,
  which is the one break the change allows.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-15
- **Verdict**: PASS
- **Suggested commit**: `fix(web): keep table values whole and stack the tables when narrow`
- **Quality gate**: tests ✓ (471 passed, 0 failed, 10 ignored) | lint ✓ (`cargo clippy --all-targets -- -D warnings`) | types ✓ (`cargo check`) | format ✓ (`cargo fmt --check`)

### Findings

| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | minor | `docs/USER-GUIDE.md` | The fault-table paragraph stated `"and counting" on a line of its own while the target is still down`. Under 700 px the override `table.faults .ongoing { display: inline; }` (Step 8) puts it beside the duration: measured at 390 px, `getComputedStyle(.ongoing).display` is `inline` and the cell is one line; at 900 px it is `block` and the cell is 52 px. The guide now states both forms. | fixed in review |
| 2 | minor | `docs/PRODUCT.md` | `FR-17` stated the duration with `"and counting"` `under it in smaller type`, without the stacked exception. Same evidence as #1. The requirement now reads "… under it in smaller type, `"and counting"` — beside it on the same line in the stacked layout under 700 px." | fixed in review |
| 3 | minor | `docs/PRODUCT.md` | `FR-17` stated the header row `stays in place while a long table scrolls`. The stacked layout sets `position: static` (Step 8, deliberate: a two-line sticky block would cover the row below). Measured at 390 px `position` is `static`, at 900 px `sticky`. The requirement now carries the exception. | fixed in review |
| 4 | nit | `docs/PRODUCT.md` | `FR-17` remains a single very long bullet; this change makes it longer. Pre-existing shape of the document. | open |
| 5 | nit | `src/web/static/style.css` | `tr.fault.expanded td:first-child { padding-left: 4px }` still wins over the stacked layout's `padding: 0`, so the first cell of an open row shifts 4 px — the pair `3 px bar + 4 px pad` is what the wide layout uses to keep text aligned, and it happens to stay balanced there. No behavioural effect. | open |

### Documentation corrected in this round

- `docs/USER-GUIDE.md`, the fault-table paragraph: two sentences added after `… `cause` over `lasted`.` describing the two consequences of the stacked layout (no sticky header, `"and counting"` on the same line).
- `docs/PRODUCT.md`, `FR-17`: the sticky-header clause gained its stacked exception, and the `"and counting"` sentence gained the same one.
- Re-ran the quality suite after the edits: 471 passed, 0 failed. The profile defines no documentation check; the suite and a manual read of the three edited passages against `style.css` are the evidence.
- User guide checks: **accuracy** — every documented value cross-read against the renderers and the stylesheet, and against the measurements below; **completeness** — the change adds no configuration option, so `Configuration` is unchanged; **self-containment** — no link to an internal document (grep finds none), only the existing in-file anchors; **summary** — `What the tool does` is three lines.

### Verification matrix

| # | Check | Evidence |
|---|---|---|
| 1 | Every goal achieved | 13/13, see gap analysis |
| 2 | Scope | 8 files changed, all under `## Affected modules`: `style.css`, `app.js`, `page.rs`, `detail.rs`, `tests/web.rs`, `PRODUCT.md`, `ARCHITECTURE.md`, `USER-GUIDE.md` |
| 3 | Quality suite green | fmt ✓, clippy ✓, `cargo check` ✓, `cargo test` 471 ✓ |
| 4 | Tests exercise real paths | six new tests over the real renderers and the served assets: `the_tokens_of_a_cell_are_never_broken`, `an_instant_may_break_between_its_date_and_its_time`, `the_mean_and_the_day_are_never_broken`, `the_four_widgets_keep_their_order_and_take_the_three_stated_shapes`, `both_tables_state_their_roles_and_their_two_columns`, `no_cell_of_a_table_breaks_a_token` |
| 5 | No test weakened | every deleted assertion in the diff is replaced by the same assertion on the new markup; lib tests 332 → 334, integration 59 → 60; no `#[ignore]` added |
| 6 | Checklist and notes | 15/15 ticked, every open question resolved, `## Implementation Notes` present with decisions, deviations, discoveries and measurements |
| 7 | Documentation contracts | `PRODUCT` `FR-17`, `ARCHITECTURE` `web`/`Output` and `USER-GUIDE` updated; `ADR`, `GLOSSARY`, `DEVELOPER-GUIDE`, `README`, `NOTES` need no change (no decision, term, command or deployment step moved) |
| 8 | No contradiction with architecture or decisions | `ARCHITECTURE` states the same contract the code implements; no new decision record is required, as the plan justified |
| 9 | No scope creep | no feature outside the tables and the widget row; `assets/api/theme.css`, the chart, the API and the CSV are untouched |
| 10 | Edge cases | empty fault row and empty day row span the row (`grid-column: 1 / -1`, verified in the browser at 390 px with a fault-free history); a fault without a cause still lays out as a row; cell `title` tooltips preserved; the note row opens across the full 358 px |
| 11 | Security | no input path, query, credential or socket changed; roles are static attributes; the script still writes text nodes only |
| 12 | Not a `security` plan | n/a |
| 13 | User guide | corrected, see above |

Measurements taken in this session (`simulate`, seeds 1/3/7, `/` and `/target?…&days=30d`):

- 900 px: `.faults`/`.days` rows are `table-row`, the fault row is 52 px, `th` is `sticky`, `.ongoing` is `block`.
- 390 px: fault cells at x=16 (target over instant) and x=213 (cause over duration), header cells at exactly 16 and 213; day row `date`/`faults` at y=1023 and `down time`/`avg up` at y=1048, header mapped identically; `th` is `static`; `.ongoing` is `inline`; a closed note is hidden and an open one spans 358 px; `document.scrollingElement.scrollWidth` equals the viewport.
- 1200/760/700/520/470/460/430/414/401/400/375/360/344/320 px (implementation run): token line counts stay at 1, the widget row measures 4 → 2+2 → 1+3, and the page never scrolls sideways.

### Gap analysis

- **Goals**: 13/13 achieved. Each token goal is pinned by a test and was measured in a browser; the two stacked layouts are pinned by `both_tables_state_their_roles_and_their_two_columns` and measured at 390 px; the wide layout was measured unchanged at 900 px.
- **Scope**: in scope. No file outside `## Affected modules` was touched.
- **Steps**: 10/10 done.
- **Deviations**: all recorded in Implementation Notes — `label` helper dropped from `app.js`, `value_with_max` wrapping both branches, the Step 5 assertion split, the `stylesheet` import in `detail.rs` tests, `.stats { overflow-x: auto }` with the tighter narrow-step spacing, and `table.faults tr[hidden] { display: none; }`. All legitimate; each replaces a mechanism the plan described with the one the browser actually needed.
- **Additions**: the two rules above are the only ones beyond the plan text, and both are justified by a measurement (`display: grid` beating `[hidden]`; the document scrolling 5 px sideways at 380 px).
- **Deferred**: nothing.
- **Known limitations**: the box is checked in the two open nits.

### Known limitations

- Under 700 px the header of either table no longer stays in place while a long table scrolls; on a 360-day per-day table that is a real loss on a phone, accepted in Step 8 to avoid a two-line sticky block covering the row it belongs to.
- Below about 360 px the three tiles of the narrowest widget shape no longer fit and the row scrolls inside itself rather than moving the page.
- Two opens: the `PRODUCT` bullet keeps growing (#4) and the expanded-cause left bar leans on a compensation that now also applies in the stacked layout (#5).

### Recommendations for future work

- Consider `position: sticky` on the stacked header with a one-line variant (for example `target · started` on one line) so the per-day table keeps its labels while scrolling on a phone.
- Consider a second breakpoint at about 360 px for the widget row, where the three tiles stack one per row instead of scrolling.
