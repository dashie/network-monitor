---
title: Add a print control and a printable form of the target page
date: 2026-11-18
status: done
type: feature
impact: low
area: web
version_bump: minor
tags: [web, print, css, accessibility]
depends_on: []
branch_recommended: false
---

## Summary

The target page gains a `Print` control at the right end of its masthead. Selecting it calls
the browser's print dialog on the page already open. A new `@media print` block turns that
same page into the printable form: it keeps only the target name, the selected period stated
as a subtitle, the bar chart of the faults per day, and the day-by-day table. The identity
block, the paused banner, the toolbar, the four widgets, the last-faults list, the navigation
and the footer stay on the screen and off the paper.

No new route, no server-side rendering path, no configuration key, and no client-side data
handling are added: the printed sheet is the page the browser already holds, with the dates
already localised by `app.js`, restyled by CSS. The timeline page does not change.

## Why

An operator who needs to attach the history of one target to a ticket or hand it to a
colleague today has to print the whole screen page: the navigation, the widgets, the last
faults and the footer come along, and the identity block pushes the chart down. The page
already holds exactly the three things the report needs — the name, the chart and the table —
so the missing piece is a print stylesheet, not a second page.

A second route would duplicate the detail rendering, need its own tests and documentation, and
lose the browser localisation of the instants unless the script ran there too. A CSS-only form
reuses everything and stays inside the KISS and modularity rules of
[AGENTS.md](../AGENTS.md#engineering-principles): one page, one stylesheet, one script.

## Goals

- The target page shows a `Print` control inside its masthead, after the identity block and at
  the right end of the same centre line; selecting it opens the browser's print dialog.
- The control has a thin border and a small radius (`border: 1px solid var(--rule)`,
  `border-radius: 6px`), so it reads as a quiet toolbar action beside the brand.
- Printing the target page produces a sheet that, from top to bottom, holds only: the target
  name as a heading; a subtitle `Last <period> · Disconnections per day`; the bar chart; and
  the `Day by day` table. The masthead, the paused banner, the toolbar, the `.stats` widgets,
  the last-faults section, the period navigation and the footer are absent from the paper.
- The printed sheet uses black text on white paper under a dark operating-system preference,
  and keeps the red bar fill and the grey axis and count labels by asking for exact colour
  adjustment on the chart.
- A printed table never scrolls and never splits a row across two sheets: the header loses its
  sticky position and each row avoids a page break inside itself.
- The timeline page, the JSON API, the CSV export and every other page keep their current
  markup, behaviour and bytes.
- Every new behaviour is covered by a test that fails without the change.

## Non-goals

- A dedicated `/target/print` route, a server-rendered print document, a PDF generator, or a
  download of the print form. The browser's own print-to-PDF is the only output.
- Printing the timeline page, the JSON API reference, the history import or any other page.
- Choosing a paper size, orientation, margins per target, headers and footers of the print
  engine, or a server-side logo. `@page` states one margin for every sheet.
- A configurable print button, a new configuration key, a feature flag or a user preference.
- Changing the chart data, the buckets, the table columns or the period selector.
- Rendering the print form without JavaScript. The page already depends on `app.js` for the
  cause disclosure and the localised instants; a browser without the script prints the same
  page through its own Print command, and the Print control does nothing.

## Affected modules

**Source**
- `src/web/templates/detail.html` — the `Print` button in the masthead, the print-only header,
  a `<section class="recent">` wrapper around `{recent}`, and a class on the chart heading.
- `src/web/detail.rs` — replace `{print_name}` with the escaped target name and give the
  day-by-day heading a class; add the unit tests listed in the steps.
- `src/web/static/style.css` — the `.print-button` rule, the screen-hidden `.print-head`, and
  the `@media print` block that isolates the three printed parts.
- `src/web/static/app.js` — one guarded listener that calls `window.print()`.

**Tests**
- `src/web/detail.rs` — print control placement, print header content and escaping, the
  recent-faults wrapper, the chart and table headings, the button style and the print sheet
  rules; the existing `a_target_without_a_label_states_its_address_alone` is scoped to the
  toolbar so the new print heading does not change what it counts.
- `src/web/page.rs` — the timeline page offers no print control.
- `tests/web.rs` — *(maybe)* one HTTP-level assertion that `GET /target?target=…` carries the
  control and the print header.

**Documentation / configuration**
- `docs/USER-GUIDE.md` — **The web page**, the `/target` bullet: one paragraph on the `Print`
  control and what the sheet carries. No configuration option is added, so the
  **Configuration** section does not change.
- `docs/PRODUCT.md` — `FR-17`, the target-page description: one sentence on the control and
  the three printed parts.
- `docs/ARCHITECTURE.md` — no change: no route, contract, invariant or dependency is added.
- `docs/ADR.md` — no new decision record (see **Decision-record impact**).
- `site/index.html` — *(maybe)* one short mention only if the feature list is touched for
  another reason; the print control does not justify a screenshot change.

## Architecture impact

No architectural change. The web module keeps the same routes and the same rendering
functions; the change adds markup to an existing template, one CSS block and one event
listener. The page stays server-rendered, read-only, and free of client-side data handling:
`window.print()` reads no data and writes none.

## Decision-record impact

No new decision record: the change adds no route, no configuration key, no dependency and no
new contract. It is a presentation rule inside an existing page, reversible by deleting one
template block and one CSS block. The decisions about the read-only, server-rendered page
(`ADR-0008`, `ADR-0010`) remain the same.

## Proposed solution

### Step 1 — Add the control and the print-only header to the detail template

In `src/web/templates/detail.html`, after `{brand}` and before `</header>` of the masthead, add
the button. After the masthead, add the print-only header. Wrap `{recent}` in a section, and
class the chart heading:

```html
<header class="masthead">
{brand}
<button type="button" class="print-button" id="print-page">Print</button>
</header>
<header class="print-head">
<h1>{print_name}</h1>
<p>Last {period} · Disconnections per day</p>
</header>
```

```html
<h2 class="chart-title">Disconnections per day</h2>
{bars}
<section class="recent">{recent}</section>
{table}
```

The button opens no URL and carries no inline handler, so the page keeps the rule that no
script is written into the HTML. `{print_name}` is filled in the next step; `{period}` is the
existing placeholder, already replaced for the widgets and the title.

**Verify:** `cargo test --lib web::detail` compiles; after step 2 the new assertions pass, and
`rg -n "print-button|print-head|section class=\"recent\"" src/web/templates/detail.html`
finds each block once.

### Step 2 — Fill the print header from the target

In `src/web/detail.rs`, in `target_page`, add
`.replace("{print_name}", &escape(&name))` to the replacement chain, and change the day-by-day
heading in `days_table` to `<h2 class="days-title">Day by day</h2>`. The value is the same
`name` the title uses — the label when the target has one, the address otherwise — and it is
escaped by the same function every other user value uses.

Add the unit tests:

- `the_masthead_offers_a_print_control` — the button is inside `<header class="masthead">`
  (`</header>`), appears after `class="brand"`, and the timeline markup is not involved.
- `the_printed_header_states_the_target_and_the_period` — `<h1>cloudflare</h1>` and
  `Last 30 days · Disconnections per day`; a second render at `90` states `Last 90 days`.
- `the_printed_header_escapes_the_target_name` — a label of `<script>alert(1)</script>` is
  printed as `&lt;script&gt;alert(1)&lt;/script&gt;` inside the print heading.
- `the_recent_faults_are_a_section_the_print_sheet_can_drop` — `<section class="recent">`
  opens before `Last 10 faults` and closes after the fault table.
- `the_chart_and_the_table_keep_their_titles` — `<h2 class="chart-title">Disconnections per
  day</h2>` and `<h2 class="days-title">Day by day</h2>`.

Update the existing `a_target_without_a_label_states_its_address_alone`: the print header now
states the address a second time, so the assertion that counts `>1.1.1.1<` must be scoped to
the toolbar (`<p class="crumbs">` to `</p>`) and the new `<h1>1.1.1.1</h1>` asserted
separately. The same test keeps asserting that no `class="target-label"` and no `()` appear.

**Verify:** `cargo test --lib web::detail::tests`.

### Step 3 — Style the control and write the print sheet

In `src/web/static/style.css`, next to the masthead rules, add the button and hide the print
header on screen:

```css
.print-button {
  padding: 4px 12px;
  border: 1px solid var(--rule);
  border-radius: 6px;
  background: none;
  color: inherit;
  font: inherit;
  font-size: 12px;
  font-weight: 600;
  letter-spacing: 0.04em;
  cursor: pointer;
}
.print-button:hover { border-color: currentColor; }
/* The printed header exists only on paper: the screen already states the target in the
   toolbar and the identity block, which a print does not carry. */
.print-head { display: none; }
```

At the end of the stylesheet, add:

```css
/* The printed sheet is the target alone: its name and period, the bar chart, and the
   day-by-day table. Every control, the widgets, the fault list and the manual are of the
   screen, not of the paper. A page printed under a dark preference stays black on white. */
@media print {
  @page { margin: 16mm; }
  :root { color-scheme: light; }
  body { padding: 0; color: #000000; background: #ffffff; }
  main { max-width: none; }
  .masthead, .paused, .toolbar, .stats, .recent, footer, .print-button, h2.chart-title {
    display: none;
  }
  .print-head { display: block; margin: 0 0 12px; }
  .print-head h1 { font-size: 20px; }
  .print-head p { margin: 2px 0 0; font-size: 12px; opacity: 0.65; }
  /* A printed table does not scroll, and no row is split across two sheets. */
  th { position: static; background: none; }
  tr { break-inside: avoid; }
  /* The bars and the axis labels are data, not decoration: print them as drawn. */
  svg {
    -webkit-print-color-adjust: exact;
    print-color-adjust: exact;
  }
}
```

The `.days-title` heading stays visible: it labels the table under the print header. The
`.chart-title` heading is hidden because the print header already states the same phrase.

Add the unit tests:

- `the_print_control_has_a_thin_border_and_a_small_radius` — the `.print-button` rule contains
  `border: 1px solid var(--rule);` and `border-radius: 6px;`.
- `the_print_sheet_keeps_only_the_name_the_chart_and_the_table` — the `@media print` block
  names `.masthead`, `.paused`, `.toolbar`, `.stats`, `.recent`, `footer`, `.print-button` and
  `h2.chart-title` in its hidden rule, shows `.print-head`, and contains
  `print-color-adjust: exact`, `break-inside: avoid` and `position: static`.
- `the_print_header_is_hidden_on_screen` — outside the print block the stylesheet contains
  `.print-head { display: none; }`.

**Verify:** `cargo test --lib web::detail::tests`.

### Step 4 — Open the print dialog from the shared script

In `src/web/static/app.js`, beside the existing chart listener, add:

```js
// The print control of the target page opens the browser dialog on the page already open,
// so the sheet is the page the browser holds, with the instants already localised.
const printPage = document.getElementById('print-page');
if (printPage !== null) printPage.addEventListener('click', () => window.print());
```

The guard keeps the one script valid for the timeline, which has no control. The id is
`print-page`, not `print`, so the element never shadows `window.print` through the global named
access of an id.

Add the unit test `the_print_control_opens_the_browser_dialog` asserting that `script()`
contains `getElementById('print-page')` and `window.print()`.

**Verify:** `cargo test --lib web::detail::tests`.

### Step 5 — Cover the timeline and the HTTP contract

In `src/web/page.rs`, add `the_timeline_offers_no_print_control`: render `page(...)` and assert
it contains no `print-button` and no `print-head`.

In `tests/web.rs`, add one integration test in the shape of
`every_row_shows_its_statistics_and_links_to_its_detail_page`: start a server, seed one target,
`GET /target?target=1.1.1.1`, and assert the body contains
`<button type="button" class="print-button" id="print-page">Print</button>`,
`<header class="print-head">` and `Last 30 days · Disconnections per day`. The timeline page
`GET /` in the same test does not contain `print-button`.

**Verify:** `cargo test --lib web::page` and `cargo test --test web`.

### Step 6 — Update the user guide and the product requirement

`docs/USER-GUIDE.md`, **The web page**, after the `/target` bullet: add one paragraph stating
that the target page carries a `Print` control at the right end of the masthead, that it opens
the browser print dialog, and that the sheet holds the target name and period, the bar chart
and the day-by-day table only, while the identity block, the widgets, the last faults, the
navigation and the footer stay on screen. No configuration key is added, so the
**Configuration** section and its tables do not change.

`docs/PRODUCT.md`, `FR-17`, in the target-page description: append one sentence with the same
facts, so the requirement and the guide agree with the code.

**Verify:** `rg -n "Print control|print control|print dialog" docs/USER-GUIDE.md docs/PRODUCT.md`
finds one entry in each, and neither document names a key, route or option that the code does
not have.

### Step 7 — Run the quality gate and review the diff

Run the whole gate, then read `git diff` end to end: no route, no configuration key, no new
asset, no unrelated file.

**Verify:** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`.

## Security considerations

The change adds no surface: no route, no parameter, no file, no network call and no data
handling. The printed page is the same read-only HTML the browser already received, and
`window.print()` neither reads nor writes the history. The target name in the print header is
escaped by the existing `escape` function, so a hostile label cannot inject markup. The
`@media print` rules hide the same history the screen already shows; they expose nothing a
shoulder-surfer could not read on the screen.

## Verification plan

- **Automated.** The unit tests of steps 2–5: control placement, print header content and
  escaping, recent-faults section, headings, button style, print sheet rules, script listener,
  the timeline's absence of the control, and the HTTP body of `/target`.
- **Quality gate.** `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  (from `.spec-workflow.yml`).
- **Manual.** Start `serve` against a seeded database, open one target page, select `Print`:
  the preview shows only the name and period, the bar chart and the `Day by day` table; the
  bars are red, the text is black, and no masthead, widget, fault list, period link or footer
  appears. Repeat with `days=360d` and confirm the table paginates with its header and without
  splitting a row. Repeat with the operating system in dark mode. The timeline page shows no
  `Print` control.
- **Rollback.** Revert the commit. No schema, no route and no configuration key changes, so the
  previous binary and the previous pages behave exactly as before.

## Open questions

- [x] Confirm the `Print` control belongs to the target page only and not to the timeline.
      Resolved: target page only, as requested; the timeline is not a report of one target.
      `src/web/page.rs` asserts the timeline carries no control.
- [x] Confirm the printed sheet keeps the `Day by day` heading under the print header.
      Resolved: keep it, because it labels the table; the chart heading is hidden because the
      print header already states `Disconnections per day`.
- [x] Confirm that a browser without JavaScript prints the page through its own Print command,
      the `Print` control being inert. Resolved: accept; the page already requires `app.js` for
      its interactive parts, and the print stylesheet works without the script.

## Implementation checklist

- [x] Step 1 — Add the control and the print-only header to the detail template
- [x] Step 2 — Fill the print header from the target
- [x] Step 3 — Style the control and write the print sheet
- [x] Step 4 — Open the print dialog from the shared script
- [x] Step 5 — Cover the timeline and the HTTP contract
- [x] Step 6 — Update the user guide and the product requirement
- [x] Step 7 — Run the quality gate and review the diff
- [x] All open questions resolved
- [x] Quality suite passes (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- [x] Documentation listed under Affected modules is updated
- [x] User guide (`docs/USER-GUIDE.md`) reflects user-visible behaviour and every configuration option, or the plan records why it needs no change
- [x] `## Implementation Notes` added or updated in this plan

## Implementation Notes

- **Files changed**: `src/web/templates/detail.html`, `src/web/detail.rs`,
  `src/web/static/style.css`, `src/web/static/app.js`, `src/web/page.rs`, `tests/web.rs`,
  `docs/USER-GUIDE.md`, `docs/PRODUCT.md`.
- **Decisions**: the three open questions were resolved as recorded in the plan — target page
  only, `Day by day` kept as the table label, and a browser without JavaScript prints through
  its own Print command. The id is `print-page`, not `print`, so the element never shadows
  `window.print` through the global named access of an id.
- **Deviation (user-approved)**: `tests/web.rs::every_page_opens_with_the_same_brand_block`
  ended the brand block at `</header>`, so the new sibling button joined the extracted block.
  With the user's explicit approval, the helper now ends at `</a>`, which is the identity block
  its own doc comment describes; the equality assertion and every other assertion of the test
  are unchanged, so the test is not weakened. The test is inside the `tests/web.rs` entry the
  plan already listed.
- **Discovery**: `tests/simulate.rs::the_simulated_mode_writes_nothing` failed once with a
  connection error on an untouched module during the first full run; it passes when rerun
  alone and in the final full suite. Recorded as an environmental flake, not fixed.
- **Manual print preview**: no browser is available in this environment, so the visual print
  preview of the plan's **Verification plan** was not executed; the markup and the stylesheet
  rules are asserted by the unit and HTTP tests instead.
- **Deviation (fix round)**: review round 1 found that the first `@media print` block was
  global, so printing the timeline would also hide its masthead, toolbar and footer and gain
  the sheet's page margin. The block is now scoped with `class="target-page"` on the detail
  document's `<html>`, every rule names `html.target-page`, and the global `@page` rule was
  removed: the printable sheet uses the browser's default page margins, and the timeline keeps
  the print form it had before. `the_print_sheet_keeps_only_the_name_the_chart_and_the_table`,
  `the_print_sheet_is_scoped_to_the_target_page` and `the_timeline_offers_no_print_control`
  assert the scoping.

## Review Notes

- **Round**: 1
- **Review date**: 2026-11-18
- **Verdict**: NEEDS WORK
- **Suggested commit**: `feat(web): add a printable form of the target page`
- **Quality gate**: tests ✓ | lint ✓ | format ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `b8edafcde0df211450e9ac3357920f4db45eb2c38218d57074da4af71ca8a48d`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/web/static/style.css` | The print sheet is shared by both pages. Printing the timeline now hides its masthead, toolbar and footer and applies the target sheet's `@page` margin, so `FR-17`'s timeline does not keep its current print behaviour, contrary to the plan's sixth goal. The rules must be scoped to the target page. | open |

### Gap analysis
- Goals: 6/7 achieved. The timeline/API/export goal is partial: the markup and the screen behaviour are unchanged, but the shared print sheet changes what the timeline prints.
- Scope: in scope. `tests/web.rs` carried the user-approved helper change recorded in Implementation Notes.
- Deviations: the `every_page_opens_with_the_same_brand_block` helper now ends at `</a>`; approved by the user and recorded.
- Additions: none beyond the approved test-helper change.
- Deferred: the manual print preview, because no browser is available in this environment.

### Known limitations
- The print preview was verified through markup and stylesheet tests, not through a browser.

### Recommendations for future work
- Scope the print rules with a class on the target page's `<body>` (or a print-only stylesheet linked by the detail page alone), and keep the timeline out of the print sheet.

## Review Notes

- **Round**: 2 — self-review performed in the same invocation as the implementation and the fix round
- **Review date**: 2026-11-18
- **Verdict**: PASS
- **Suggested commit**: `feat(web): add a printable form of the target page`
- **Quality gate**: tests ✓ | lint ✓ | format ✓ (`cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`)
- **Reviewed state**: `b9622daaef31d4204208942a6b289be3269c353f30b715624aacee5e3fc12363`

### Findings
| # | Severity | Area | Finding | Status |
|---|---|---|---|---|
| 1 | major | `src/web/static/style.css` | The print sheet was shared by both pages, so printing the timeline lost its masthead, toolbar and footer. | fixed in fix round: every print rule now names `html.target-page`, and the global `@page` rule is gone; the full suite passes and `the_timeline_offers_no_print_control` asserts the timeline carries no `target-page` class. |

### Gap analysis
- Goals: 7/7 achieved. The timeline keeps its markup and its print form; the print sheet is scoped to the target document.
- Scope: in scope. All changed files are in **Affected modules**; `tests/web.rs` also carries the user-approved brand-helper change recorded in Implementation Notes.
- Deviations: the body class became a class on `<html>`, so the existing `<body data-refresh="30">` assertion on both pages stays true; the `@page` margin was dropped rather than scoped, because CSS cannot scope it to one document.
- Additions: none beyond the approved test-helper change.
- Deferred: the manual print preview, because no browser is available in this environment.

### Known limitations
- The print preview was verified through markup and stylesheet tests, not through a browser; the printable sheet uses the browser's default page margins.

### Recommendations for future work
- If a fixed paper margin is wanted later, link a print-only stylesheet from the detail page alone rather than adding a global `@page` rule to the shared sheet.

## Closure

- **Closed**: 2026-11-18
- **Version**: 0.16.0 → 0.17.0
- **Executed by**: opencode (`deepseek/deepseek-flash`)
- **Tokens**: unknown
- **Cost**: unknown
