//! Server-side HTML and SVG for the read-only web page.
//!
//! The page is generated from the same segments the query path uses; the browser
//! only asks for another window. No chart library, no client-side data handling.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::store::{DownEpisode, TargetState, TargetTimeline, TimelineSegment};
use crate::time::format_rfc3339;

use super::stats::{self, RunStats};

const WIDTH: f64 = 1000.0;
const LABEL_WIDTH: f64 = 190.0;
const STATS_WIDTH: f64 = 330.0;
const ROW_HEIGHT: f64 = 24.0;
const ROW_GAP: f64 = 6.0;
const TOP_PAD: f64 = 6.0;
const AXIS_HEIGHT: f64 = 20.0;

/// Shared stylesheet of both pages.
pub(crate) const STYLE: &str = r#"
  :root {
    color-scheme: light dark;
    --rule: rgba(127, 127, 127, 0.28);
    --muted: 0.65;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0;
    padding: 32px 24px 48px;
    font: 15px/1.5 system-ui, -apple-system, "Segoe UI", sans-serif;
  }
  main { max-width: 1040px; margin: 0 auto; }
  h1 { font-size: 22px; margin: 0; letter-spacing: -0.01em; }
  h2 {
    font-size: 12px;
    margin: 32px 0 10px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    opacity: var(--muted);
  }
  h2 .download {
    font-size: 12px;
    font-weight: 400;
    letter-spacing: 0;
    text-transform: none;
    margin-left: 12px;
  }
  .hint { margin: 4px 0 20px; font-size: 13px; opacity: var(--muted); }
  .windows { display: flex; flex-wrap: wrap; gap: 8px; margin: 0 0 20px; font-size: 13px; }
  .windows a {
    padding: 2px 11px;
    border: 1px solid var(--rule);
    border-radius: 999px;
    color: inherit;
    text-decoration: none;
    font-variant-numeric: tabular-nums;
  }
  .windows a:hover { border-color: currentColor; }
  .windows a[aria-current] {
    background: rgba(127, 127, 127, 0.16);
    border-color: transparent;
    font-weight: 600;
  }
  .breadcrumb {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    margin: 0 0 18px;
    font-size: 13px;
  }
  .breadcrumb a {
    color: inherit;
    text-decoration: none;
    border-bottom: 1px solid transparent;
    padding-bottom: 1px;
  }
  .breadcrumb a:hover { border-bottom-color: currentColor; }
  .breadcrumb .sep { opacity: 0.4; }
  .breadcrumb [aria-current] { opacity: var(--muted); }
  .title { display: flex; flex-wrap: wrap; align-items: center; gap: 10px; }
  .badge {
    padding: 2px 9px;
    border-radius: 999px;
    font-size: 11px;
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: #ffffff;
  }
  .badge.up { background: #2e7d32; }
  .badge.down { background: #c62828; }
  .badge.unknown { background: #9e9e9e; }
  .since { font-size: 13px; opacity: var(--muted); font-variant-numeric: tabular-nums; }
  .stats { display: flex; flex-wrap: wrap; gap: 12px; margin: 20px 0 0; }
  .stat {
    flex: 1 1 150px;
    padding: 10px 14px;
    border: 1px solid var(--rule);
    border-radius: 8px;
  }
  .stat b {
    display: block;
    font-size: 19px;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
  }
  .stat span { font-size: 12px; opacity: var(--muted); }
  .window {
    margin: 0 0 10px;
    font-size: 13px;
    font-variant-numeric: tabular-nums;
    opacity: var(--muted);
  }
  svg { width: 100%; height: auto; display: block; }
  a:hover text { text-decoration: underline; }
  .legend { display: flex; gap: 18px; margin-top: 14px; font-size: 13px; opacity: 0.8; }
  .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .swatch { width: 14px; height: 10px; border-radius: 2px; display: inline-block; }
  /* One table style for both pages: as wide as `main`, text left, numbers right.
     The numeric columns carry `.num`; no table is narrower than the chart above it. */
  table { width: 100%; border-collapse: collapse; font-variant-numeric: tabular-nums; }
  th, td { text-align: left; padding: 7px 18px 7px 0; }
  th {
    font-size: 12px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    opacity: var(--muted);
    border-bottom: 1px solid var(--rule);
  }
  tbody td { border-bottom: 1px solid var(--rule); }
  tbody tr:last-child td { border-bottom: 0; }
  tbody tr:hover { background: rgba(127, 127, 127, 0.07); }
  /* The row of the day still running states a rolling window, not a calendar one. */
  td.current { background: rgba(127, 127, 127, 0.1); }
  .tag {
    display: inline-block;
    margin-right: 7px;
    padding: 0 7px;
    border: 1px solid var(--rule);
    border-radius: 999px;
    font-size: 10px;
    font-weight: 600;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    opacity: 0.75;
    vertical-align: 1px;
  }
  .num { text-align: right; }
  .sub { font-size: 11px; opacity: 0.65; font-variant-numeric: tabular-nums; }
  td.down { color: #c62828; }
  @media (max-width: 620px) { body { padding: 20px 16px 40px; } }
"#;

/// How often the browser reloads a page by itself.
pub(crate) const REFRESH_SECONDS: u32 = 30;

const HOUR_MS: i64 = 3_600_000;

/// The windows the timeline offers, smallest first: the links read left to right from
/// the narrowest to the widest.
pub(crate) const WINDOWS: [u8; 6] = [1, 2, 4, 6, 12, 24];

/// The window shown when the query names none: the shortest one, because a reader opens
/// the page to see what is happening now.
pub(crate) const DEFAULT_WINDOW_HOURS: u8 = 1;

/// The length of a window, in milliseconds.
pub(crate) fn hours_span(hours: u8) -> i64 {
    i64::from(hours) * HOUR_MS
}

const TICK_STEPS_MS: [i64; 10] = [
    60_000,
    300_000,
    900_000,
    1_800_000,
    3_600_000,
    10_800_000,
    21_600_000,
    43_200_000,
    86_400_000,
    604_800_000,
];

/// Right edge of every statistics column of a lane, relative to the start of the block,
/// with its header and what the column means.
const COLUMNS: [(f64, &str, &str); 6] = [
    (26.0, "DROPS", "disconnections in the window"),
    (86.0, "DOWN", "time spent down in the window"),
    (160.0, "AVG UP", "average up run, with its spread"),
    (208.0, "MAX", "longest up run in the window"),
    (258.0, "UP FOR", "how long it has been up without a drop"),
    (318.0, "UP %", "up share of the time the target was probed"),
];

/// The time window shown by the chart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from_ms: i64,
    pub to_ms: i64,
}

impl Window {
    fn span(&self) -> i64 {
        (self.to_ms - self.from_ms).max(1)
    }

    fn x(&self, at_ms: i64) -> f64 {
        let track = WIDTH - LABEL_WIDTH - STATS_WIDTH;
        let offset = (at_ms - self.from_ms) as f64 / self.span() as f64;
        LABEL_WIDTH + offset * track
    }
}

/// Follow a lane link from script: some engines ignore an `<a href>` that wraps SVG
/// shapes.
const LANE_SCRIPT: &str = r#"document.getElementById('chart').addEventListener('click', event => {
  const link = event.target instanceof Element ? event.target.closest('a') : null;
  if (link === null) return;
  event.preventDefault();
  window.location.assign(link.getAttribute('href'));
}, true);"#;

/// Replace every instant the server wrote in UTC with the locale and the time zone of
/// the visitor. The server stays on UTC; only the presentation moves.
pub(crate) const LOCALIZE_SCRIPT: &str = r#"const FORMATS = {
  datetime: { dateStyle: 'medium', timeStyle: 'medium' },
  time: { hour: '2-digit', minute: '2-digit' },
  daytime: { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' },
  date: { dateStyle: 'medium', timeZone: 'UTC' },
  day: { month: 'short', day: 'numeric', timeZone: 'UTC' },
};
const label = (instant, kind) =>
  new Intl.DateTimeFormat(undefined, FORMATS[kind] ?? FORMATS.datetime).format(instant);
for (const element of document.querySelectorAll('time[datetime], [data-datetime]')) {
  const from = new Date(element.getAttribute('data-datetime') ?? element.getAttribute('datetime'));
  if (Number.isNaN(from.getTime())) continue;
  const until = element.getAttribute('data-until');
  const prefix = element.getAttribute('data-prefix') ?? '';
  const suffix = element.getAttribute('data-suffix') ?? '';
  element.textContent = prefix + (until === null
    ? label(from, element.getAttribute('data-format'))
    : `${label(from, 'datetime')} → ${label(new Date(until), 'datetime')}`) + suffix;
}"#;

/// The full page: the windows, the chart, the legend, the last disconnections, and the
/// scripts that follow a lane link and localise the instants.
pub fn page(
    timelines: &[TargetTimeline],
    downs: &[DownEpisode],
    up_since: &HashMap<String, i64>,
    window: Window,
) -> String {
    let body = chart(timelines, up_since, window);
    let downs = downs_table(downs, window.to_ms);

    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="refresh" content="{REFRESH_SECONDS}">
<title>network-monitor</title>
<style>{STYLE}</style>
</head>
<body>
<main>
<h1>network-monitor</h1>
<p class="hint">Times follow the time zone of your browser. The page reloads every
{REFRESH_SECONDS} s; select a lane for its last 30 days.</p>
{windows}
<div id="chart">{body}</div>
<div class="legend">
  <span><i class="swatch" style="background:#2e7d32"></i>up</span>
  <span><i class="swatch" style="background:#c62828"></i>down</span>
  <span><i class="swatch" style="background:#bdbdbd"></i>unknown</span>
</div>
{downs}
</main>
<script>
{lane}
{localize}
</script>
</body>
</html>
"#,
        windows = window_links(window),
        body = body,
        downs = downs,
        lane = LANE_SCRIPT,
        localize = LOCALIZE_SCRIPT,
    )
}

/// The links that change the window of the chart.
fn window_links(window: Window) -> String {
    let mut output = String::from("<nav class=\"windows\" aria-label=\"Time window\">");
    for hours in WINDOWS {
        let current = if window.span() == hours_span(hours) {
            " aria-current=\"page\""
        } else {
            ""
        };
        let _ = write!(
            output,
            "<a href=\"/?window={hours}h\"{current}>{hours}h</a>"
        );
    }
    output.push_str("</nav>\n");
    output
}

/// The last disconnections, newest first, with how long each one lasted.
fn downs_table(downs: &[DownEpisode], now_ms: i64) -> String {
    let mut output = String::from(
        "<h2>Last 10 disconnections\
         <a class=\"download\" href=\"/export/disconnections.csv.gz\">download all (csv.gz)</a>\
         </h2>\n<table>\n\
         <thead><tr><th>target</th><th>started</th><th class=\"num\">lasted</th></tr></thead>\n\
         <tbody>\n",
    );

    if downs.is_empty() {
        output.push_str("<tr><td colspan=\"3\">No disconnection recorded yet.</td></tr>\n");
    }

    for episode in downs {
        let (lasted, ongoing) = match episode.ended_at_ms {
            Some(end) => (format_span(end - episode.started_at_ms), false),
            None => (format_span(now_ms - episode.started_at_ms), true),
        };
        let _ = writeln!(
            output,
            "<tr><td>{}</td><td><time datetime=\"{}\">{}</time></td><td{}>{}{}</td></tr>",
            escape(&episode.target),
            format_rfc3339(episode.started_at_ms),
            utc_text(episode.started_at_ms),
            if ongoing {
                " class=\"num down\""
            } else {
                " class=\"num\""
            },
            lasted,
            if ongoing { " and counting" } else { "" }
        );
    }

    output.push_str("</tbody>\n</table>");
    output
}

/// The window label plus the SVG.
fn chart(timelines: &[TargetTimeline], up_since: &HashMap<String, i64>, window: Window) -> String {
    let mut output = String::new();
    let _ = write!(
        output,
        "<p class=\"window\"><time datetime=\"{}\">{}</time> → <time datetime=\"{}\">{}</time></p>",
        format_rfc3339(window.from_ms),
        utc_text(window.from_ms),
        format_rfc3339(window.to_ms),
        utc_text(window.to_ms)
    );

    if timelines.is_empty() {
        output.push_str("<p>No target has been probed yet. Start the monitor first.</p>");
        return output;
    }

    let rows = timelines.len() as f64;
    let height = TOP_PAD + AXIS_HEIGHT + rows * (ROW_HEIGHT + ROW_GAP) + 8.0;

    let _ = write!(
        output,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {WIDTH} {height:.0}\" \
         role=\"img\" aria-label=\"State timeline per target\">"
    );

    // Time grid and labels.
    let span = window.span();
    let step = tick_step(span);
    let mut tick = window.from_ms.div_euclid(step) * step;
    if tick < window.from_ms {
        tick += step;
    }
    while tick <= window.to_ms {
        let x = window.x(tick);
        let _ = write!(
            output,
            "<line x1=\"{x:.1}\" y1=\"{}\" x2=\"{x:.1}\" y2=\"{:.0}\" stroke=\"rgba(127,127,127,0.35)\"/>",
            TOP_PAD + AXIS_HEIGHT - 4.0,
            height - 4.0
        );
        let _ = write!(
            output,
            "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"10\" \
             fill=\"#666666\" data-datetime=\"{}\" data-format=\"{}\">{}</text>",
            TOP_PAD + 8.0,
            format_rfc3339(tick),
            tick_format(span),
            escape(&tick_label(tick, span))
        );
        tick += step;
    }

    // Column headers of the per-lane statistics. The numbers are right-aligned, like the
    // tables of the detail page, so a reader compares magnitudes by column edge.
    let stats_x = WIDTH - STATS_WIDTH + 8.0;
    for (offset, label, hint) in COLUMNS {
        let _ = write!(
            output,
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"10\" fill=\"#666666\" \
             text-anchor=\"end\"><title>{hint}</title>{label}</text>",
            stats_x + offset,
            TOP_PAD + 8.0
        );
    }

    // One lane per target; the whole lane links to the 30-day detail page.
    for (index, timeline) in timelines.iter().enumerate() {
        let y = TOP_PAD + AXIS_HEIGHT + index as f64 * (ROW_HEIGHT + ROW_GAP);
        let name = timeline
            .label
            .clone()
            .unwrap_or_else(|| timeline.target.clone());
        let stats = window_stats(&timeline.segments, window.from_ms);

        let _ = write!(
            output,
            "<a href=\"/target?target={}\"><rect x=\"0\" y=\"{y:.1}\" width=\"{WIDTH}\" \
             height=\"{ROW_HEIGHT}\" fill=\"#000000\" fill-opacity=\"0\">\
             <title>Last 30 days for {}</title></rect>",
            escape(&timeline.target),
            escape(&timeline.target),
        );

        let _ = write!(
            output,
            "<text x=\"0\" y=\"{:.1}\" font-size=\"12\" fill=\"currentColor\">\
             <title>{}</title>{}</text>",
            y + ROW_HEIGHT * 0.7,
            escape(&timeline.target),
            escape(&short(&name, 22))
        );

        for segment in &timeline.segments {
            let x = window.x(segment.from_ms);
            let width = (window.x(segment.to_ms) - x).max(0.4);
            let _ = write!(
                output,
                "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{width:.1}\" height=\"{ROW_HEIGHT}\" \
                 rx=\"2\" fill=\"{}\"><title data-datetime=\"{}\" data-until=\"{}\" \
                 data-prefix=\"{} — \">{} — {} to {}</title></rect>",
                colour(segment.state),
                format_rfc3339(segment.from_ms),
                format_rfc3339(segment.to_ms),
                state_label(segment.state),
                state_label(segment.state),
                utc_text(segment.from_ms),
                utc_text(segment.to_ms)
            );
        }

        let runs = runs_stats(&timeline.segments);
        let values = [
            (stats.drops.to_string(), String::new()),
            (format_span(stats.down_ms), String::new()),
            (format_runs(runs), run_count_hint(runs)),
            (
                match stats::longest_run(&timeline.segments) {
                    Some(longest_ms) => format_span(longest_ms),
                    None => "—".to_string(),
                },
                "the longest up run of the window".to_string(),
            ),
            (
                match up_since.get(&timeline.target) {
                    Some(up_for_ms) => format_span(*up_for_ms),
                    None => "—".to_string(),
                },
                String::new(),
            ),
            (
                format_share(availability(&timeline.segments)),
                String::new(),
            ),
        ];
        for ((offset, _, _), (value, hint)) in COLUMNS.iter().zip(&values) {
            let title = if hint.is_empty() {
                String::new()
            } else {
                format!("<title>{hint}</title>")
            };
            let _ = write!(
                output,
                "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"currentColor\" \
                 text-anchor=\"end\">{title}{}</text>",
                stats_x + offset,
                y + ROW_HEIGHT * 0.7,
                escape(value)
            );
        }
        output.push_str("</a>");
    }

    output.push_str("</svg>");
    output
}

/// Disconnections and total down time of one lane, counted over the shown window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowStats {
    pub drops: u32,
    pub down_ms: i64,
}

/// Statistics of a lane. A down segment that starts after the window opened is a
/// disconnection; a down segment that was already open when the window opened is not,
/// because it began before the period the numbers describe.
pub fn window_stats(segments: &[TimelineSegment], from_ms: i64) -> WindowStats {
    let mut stats = WindowStats {
        drops: 0,
        down_ms: 0,
    };
    for segment in segments {
        if segment.state != TargetState::Down {
            continue;
        }
        stats.down_ms += (segment.to_ms - segment.from_ms).max(0);
        if segment.from_ms > from_ms {
            stats.drops += 1;
        }
    }
    stats
}

/// A compact duration: `45s`, `12m`, `3h07`, `2d04h`.
pub fn format_span(ms: i64) -> String {
    if ms <= 0 {
        return "0s".to_string();
    }
    let seconds = ms / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h{:02}", minutes % 60);
    }
    format!("{}d{:02}h", hours / 24, hours % 24)
}

/// Mean and spread of the `up` runs of a lane, over the window it shows.
pub fn runs_stats(segments: &[TimelineSegment]) -> RunStats {
    stats::summarize(&stats::up_runs(segments))
}

/// How long a target has been continuously `up`: the distance from the transition that
/// turned it up to `now_ms`. `None` while it is down or unknown, because there is no
/// running `up` stretch to measure.
pub fn up_for(state: TargetState, last_change_at_ms: i64, now_ms: i64) -> Option<i64> {
    (state == TargetState::Up).then(|| (now_ms - last_change_at_ms).max(0))
}

/// Share of the probed time spent `up`, in permille; `None` when nothing was probed.
pub fn availability(segments: &[TimelineSegment]) -> Option<i64> {
    stats::up_permille(segments)
}

/// `1h12±45m`, `1h12` when a single run gives no spread to state, or `—` when the
/// window holds no `up` run at all.
pub fn format_runs(runs: RunStats) -> String {
    match runs.count {
        0 => "—".to_string(),
        1 => format_span(runs.mean_ms),
        _ => format!(
            "{}±{}",
            format_span(runs.mean_ms),
            format_span(runs.sigma_ms)
        ),
    }
}

/// `99.8%`, from a share in permille; `—` when there is no probed time to share.
pub fn format_share(permille: Option<i64>) -> String {
    match permille {
        Some(permille) => format!("{}.{}%", permille / 10, permille % 10),
        None => "—".to_string(),
    }
}

/// The sample behind an average, for the tooltip: an average of one run has no
/// spread, and an average of none does not exist.
fn run_count_hint(runs: RunStats) -> String {
    match runs.count {
        0 => "no up run in the window".to_string(),
        1 => "1 up run in the window: no spread to speak of".to_string(),
        count => format!("mean of {count} up runs in the window"),
    }
}

fn tick_step(span_ms: i64) -> i64 {
    for step in TICK_STEPS_MS {
        if span_ms / step <= 8 {
            return step;
        }
    }
    (span_ms / 6).max(1)
}

/// A tick wider than a day and a half also names the day, so a long window is not a
/// row of identical clock times.
fn shows_date(span_ms: i64) -> bool {
    span_ms > 36 * HOUR_MS
}

/// How the browser formats a tick, mirroring [`tick_label`].
fn tick_format(span_ms: i64) -> &'static str {
    if shows_date(span_ms) {
        "daytime"
    } else {
        "time"
    }
}

fn tick_label(at_ms: i64, span_ms: i64) -> String {
    let stamp = format_rfc3339(at_ms);
    if shows_date(span_ms) {
        format!("{} {}", &stamp[5..10], &stamp[11..16])
    } else {
        stamp[11..16].to_string()
    }
}

/// The fallback text of an instant: readable, explicitly UTC, and replaced in the
/// browser by the time zone and the locale of the visitor.
fn utc_text(at_ms: i64) -> String {
    let stamp = format_rfc3339(at_ms);
    format!("{} {} UTC", &stamp[0..10], &stamp[11..19])
}

fn short(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

fn colour(state: TargetState) -> &'static str {
    match state {
        TargetState::Up => "#2e7d32",
        TargetState::Down => "#c62828",
        TargetState::Unknown => "#bdbdbd",
    }
}

fn state_label(state: TargetState) -> &'static str {
    state.as_str()
}

pub(crate) fn escape(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No target is up in these pages unless a test says so.
    fn no_streaks() -> HashMap<String, i64> {
        HashMap::new()
    }
    use crate::store::TimelineSegment;

    fn timeline() -> TargetTimeline {
        TargetTimeline {
            target: "1.1.1.1".to_string(),
            label: Some("cloudflare".to_string()),
            segments: vec![
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 0,
                    to_ms: 3_600_000,
                },
                TimelineSegment {
                    state: TargetState::Down,
                    from_ms: 3_600_000,
                    to_ms: 7_200_000,
                },
            ],
        }
    }

    #[test]
    fn the_chart_draws_one_segment_per_state() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = chart(&[timeline()], &no_streaks(), window);
        assert_eq!(rendered.matches("rx=\"2\"").count(), 2);
        assert!(rendered.contains("#2e7d32"));
        assert!(rendered.contains("#c62828"));
        assert!(rendered.contains("cloudflare"));
    }

    #[test]
    fn the_lane_shows_its_drops_and_down_time_and_links_to_the_detail_page() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = chart(&[timeline()], &no_streaks(), window);
        assert!(rendered.contains("DROPS"));
        assert!(rendered.contains("DOWN"));
        assert!(rendered.contains("1h00"));
        assert!(rendered.contains("<a href=\"/target?target=1.1.1.1\">"));
    }

    #[test]
    fn the_last_disconnections_are_listed_with_their_duration() {
        let downs = [
            DownEpisode {
                event_id: 2,
                target: "1.1.1.1".to_string(),
                started_at_ms: 7_000_000,
                ended_at_ms: None,
            },
            DownEpisode {
                event_id: 1,
                target: "8.8.8.8".to_string(),
                started_at_ms: 3_600_000,
                ended_at_ms: Some(4_320_000),
            },
        ];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(rendered.contains("Last 10 disconnections"));
        assert!(rendered.contains("12m"), "720 s down");
        assert!(
            rendered.contains("and counting"),
            "still down at the window end"
        );
        assert!(rendered.contains("8.8.8.8"));
    }

    #[test]
    fn the_duration_column_uses_the_style_shared_with_the_detail_page() {
        let downs = [DownEpisode {
            event_id: 1,
            target: "8.8.8.8".to_string(),
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
        }];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(rendered.contains("<th class=\"num\">lasted</th>"));
        assert!(rendered.contains("<td class=\"num\">12m</td>"));
        assert!(
            !STYLE.contains(".numeric"),
            "one table style for both pages"
        );
        assert!(STYLE.contains(".num { text-align: right; }"));
    }

    #[test]
    fn an_ongoing_disconnection_keeps_both_the_numeric_and_the_down_style() {
        let downs = [DownEpisode {
            event_id: 1,
            target: "1.1.1.1".to_string(),
            started_at_ms: 7_000_000,
            ended_at_ms: None,
        }];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(rendered.contains("<td class=\"num down\">"));
    }

    #[test]
    fn a_quiet_history_says_so() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains("No disconnection recorded yet."));
    }

    #[test]
    fn the_page_reloads_itself_and_keeps_only_the_lane_script() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.starts_with("<!doctype html>"));
        assert!(rendered.contains("<meta http-equiv=\"refresh\" content=\"30\">"));
        assert!(!rendered.contains("wheel"), "no zoom");
        assert!(!rendered.contains("pointerdown"), "no pan");
        assert!(!rendered.contains("fetch("));
    }

    #[test]
    fn a_lane_can_be_opened() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains("closest('a')"));
        assert!(rendered.contains("href=\"/target?target=1.1.1.1\""));
    }

    #[test]
    fn an_empty_database_renders_a_hint_instead_of_an_empty_chart() {
        let window = Window {
            from_ms: 0,
            to_ms: 1_000,
        };
        let rendered = chart(&[], &no_streaks(), window);
        assert!(rendered.contains("Start the monitor first"));
        assert!(!rendered.contains("<svg"));
    }

    #[test]
    fn labels_are_escaped() {
        let mut hostile = timeline();
        hostile.label = Some("<script>alert(1)</script>".to_string());
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = chart(&[hostile], &no_streaks(), window);
        assert!(!rendered.contains("<script>"));
        assert!(rendered.contains("&lt;script&gt;"));
    }

    #[test]
    fn long_labels_are_shortened() {
        assert_eq!(short("short", 10), "short");
        assert_eq!(short("0123456789abcdef", 10).chars().count(), 10);
    }

    #[test]
    fn tick_labels_depend_on_the_span() {
        // 2026-11-08T00:00:00Z
        let at = 1_794_096_000_000;
        assert_eq!(tick_label(at, 3_600_000), "00:00");
        assert_eq!(tick_label(at, 7 * 86_400_000), "11-08 00:00");
    }

    #[test]
    fn statistics_count_only_disconnections_that_start_inside_the_window() {
        let segments = [
            TimelineSegment {
                state: TargetState::Down,
                from_ms: 0,
                to_ms: 1_000,
            },
            TimelineSegment {
                state: TargetState::Up,
                from_ms: 1_000,
                to_ms: 1_500,
            },
            TimelineSegment {
                state: TargetState::Down,
                from_ms: 1_500,
                to_ms: 3_000,
            },
        ];

        let open_at_window_start = window_stats(&segments, 0);
        assert_eq!(open_at_window_start.drops, 1);
        assert_eq!(open_at_window_start.down_ms, 2_500);

        // The segments already cover the window, so the down time is the same; only
        // the segment that began before the window opened stops counting as a drop.
        let late_window = window_stats(&segments, 2_000);
        assert_eq!(late_window.drops, 0);
        assert_eq!(late_window.down_ms, 2_500);
    }

    #[test]
    fn a_lane_states_its_availability_and_its_average_up_run() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains("UP %"), "the availability column");
        assert!(rendered.contains("AVG UP"), "the average column");
        assert!(rendered.contains("MAX"), "the longest-run column");
        let order = ["DROPS", "DOWN", "AVG UP", "MAX", "UP FOR", "UP %"].map(|label| {
            rendered
                .find(label)
                .unwrap_or_else(|| panic!("{label} is a column"))
        });
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "the columns read left to right as requested"
        );
        assert!(rendered.contains(">50.0%<"), "half of the window was up");
        assert!(
            rendered.contains(">1h00<"),
            "the only up run lasted an hour, and is also the longest one"
        );
        assert!(
            rendered.contains("1 up run in the window"),
            "the sample behind the average goes in the tooltip"
        );
    }

    #[test]
    fn a_lane_with_several_runs_states_its_spread() {
        let timeline = TargetTimeline {
            target: "1.1.1.1".to_string(),
            label: None,
            segments: vec![
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 0,
                    to_ms: 3_600_000,
                },
                TimelineSegment {
                    state: TargetState::Down,
                    from_ms: 3_600_000,
                    to_ms: 7_200_000,
                },
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 7_200_000,
                    to_ms: 14_400_000,
                },
            ],
        };
        let window = Window {
            from_ms: 0,
            to_ms: 14_400_000,
        };
        let rendered = page(&[timeline], &[], &no_streaks(), window);
        assert!(rendered.contains(">1h30±30m<"));
        assert!(rendered.contains("mean of 2 up runs in the window"));
    }

    #[test]
    fn a_lane_states_how_long_its_target_has_been_up() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let mut up_since = HashMap::new();
        up_since.insert("1.1.1.1".to_string(), 11_220_000);
        let rendered = page(&[timeline()], &[], &up_since, window);
        assert!(rendered.contains("UP FOR"), "the streak column");
        assert!(rendered.contains(">3h07<"));
    }

    #[test]
    fn a_target_that_is_not_up_states_no_streak() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains(">—<"), "no running up stretch");
        assert!(
            rendered.contains(">1h00<"),
            "the average run is still there"
        );
    }

    #[test]
    fn the_streak_is_the_age_of_the_current_up_state() {
        assert_eq!(
            up_for(TargetState::Up, 9_000_000, 10_000_000),
            Some(1_000_000)
        );
        assert_eq!(up_for(TargetState::Down, 9_000_000, 10_000_000), None);
        assert_eq!(up_for(TargetState::Unknown, 0, 10_000_000), None);
        assert_eq!(
            up_for(TargetState::Up, 20_000_000, 10_000_000),
            Some(0),
            "a transition ahead of the clock never yields a negative age"
        );
    }

    #[test]
    fn an_average_of_one_run_states_no_spread() {
        let single = RunStats {
            count: 1,
            mean_ms: 3_600_000,
            sigma_ms: 0,
        };
        assert_eq!(format_runs(single), "1h00");
        assert_eq!(format_runs(RunStats::default()), "—");
    }

    #[test]
    fn a_share_is_a_percentage_with_one_decimal() {
        assert_eq!(format_share(Some(1_000)), "100.0%");
        assert_eq!(format_share(Some(998)), "99.8%");
        assert_eq!(format_share(Some(0)), "0.0%");
        assert_eq!(format_share(None), "—", "nothing was probed");
    }

    #[test]
    fn the_timeline_offers_every_window_and_marks_the_current_one() {
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(6),
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        for hours in WINDOWS {
            assert!(
                rendered.contains(&format!("href=\"/?window={hours}h\"")),
                "{hours}h is offered"
            );
        }
        assert!(rendered.contains("<a href=\"/?window=6h\" aria-current=\"page\">6h</a>"));
        assert!(!rendered.contains("<a href=\"/?window=24h\" aria-current"));
    }

    #[test]
    fn the_default_window_is_the_one_marked_current() {
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(DEFAULT_WINDOW_HOURS),
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains("<a href=\"/?window=1h\" aria-current=\"page\">1h</a>"));
    }

    #[test]
    fn the_windows_are_offered_from_the_narrowest_to_the_widest() {
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(DEFAULT_WINDOW_HOURS),
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        let order: Vec<usize> = WINDOWS
            .iter()
            .map(|hours| {
                rendered
                    .find(&format!("href=\"/?window={hours}h\""))
                    .expect("every window is offered")
            })
            .collect();
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "1h first, 24h last"
        );
    }

    #[test]
    fn the_window_label_and_the_axis_carry_the_instant_for_the_browser() {
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(DEFAULT_WINDOW_HOURS),
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains(
            "<time datetime=\"1970-01-01T00:00:00.000Z\">1970-01-01 00:00:00 UTC</time>"
        ));
        assert!(
            rendered.contains("data-format=\"time\""),
            "a short window labels a clock time"
        );
        assert!(
            !rendered.contains("(UTC)"),
            "the server no longer names the zone"
        );
    }

    #[test]
    fn a_long_window_labels_the_day_before_the_clock_time() {
        let window = Window {
            from_ms: 0,
            to_ms: 3 * 86_400_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains("data-format=\"daytime\""));
    }

    #[test]
    fn the_segment_tooltip_carries_both_instants_and_the_state() {
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(DEFAULT_WINDOW_HOURS),
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains(
            "data-datetime=\"1970-01-01T01:00:00.000Z\" \
             data-until=\"1970-01-01T02:00:00.000Z\" data-prefix=\"down — \""
        ));
    }

    #[test]
    fn the_start_of_a_disconnection_is_localised_like_every_other_instant() {
        let downs = [DownEpisode {
            event_id: 1,
            target: "8.8.8.8".to_string(),
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
        }];
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(DEFAULT_WINDOW_HOURS),
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(rendered.contains(
            "<td><time datetime=\"1970-01-01T01:00:00.000Z\">1970-01-01 01:00:00 UTC</time></td>"
        ));
        assert!(rendered.contains("Intl.DateTimeFormat"));
    }

    #[test]
    fn spans_are_formatted_compactly() {
        assert_eq!(format_span(0), "0s");
        assert_eq!(format_span(45_000), "45s");
        assert_eq!(format_span(12 * 60_000), "12m");
        assert_eq!(format_span(3 * 3_600_000 + 7 * 60_000), "3h07");
        assert_eq!(format_span(2 * 86_400_000 + 4 * 3_600_000), "2d04h");
    }
}
