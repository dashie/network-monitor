//! Server-side HTML and SVG for the read-only web page.
//!
//! The page is generated from the same segments the query path uses; the browser
//! only asks for another window. No chart library, no client-side data handling.

use std::fmt::Write as _;

use crate::store::{DownEpisode, TargetState, TargetTimeline, TimelineSegment};
use crate::time::format_rfc3339;

const WIDTH: f64 = 1000.0;
const LABEL_WIDTH: f64 = 190.0;
const STATS_WIDTH: f64 = 150.0;
const ROW_HEIGHT: f64 = 24.0;
const ROW_GAP: f64 = 6.0;
const TOP_PAD: f64 = 6.0;
const AXIS_HEIGHT: f64 = 20.0;

/// Shared stylesheet of both pages.
pub(crate) const STYLE: &str = r#"
  :root { color-scheme: light dark; }
  body { margin: 0; padding: 24px; font: 14px/1.4 system-ui, -apple-system, sans-serif; }
  main { max-width: 1060px; margin: 0 auto; }
  h1 { font-size: 20px; margin: 0 0 4px; }
  .hint { margin: 0 0 16px; opacity: 0.7; }
  .window { margin: 0 0 8px; font-variant-numeric: tabular-nums; }
  svg { width: 100%; height: auto; display: block; }
  a:hover text { text-decoration: underline; }
  .legend { display: flex; gap: 16px; margin-top: 12px; opacity: 0.8; }
  .legend span { display: inline-flex; align-items: center; gap: 6px; }
  .swatch { width: 14px; height: 10px; border-radius: 2px; display: inline-block; }
  h2 { font-size: 15px; margin: 28px 0 8px; }
  table { border-collapse: collapse; font-variant-numeric: tabular-nums; }
  th, td { text-align: left; padding: 3px 20px 3px 0; }
  th { font-weight: 600; opacity: 0.7; border-bottom: 1px solid #e6e6e6; }
  td.down { color: #c62828; }
"#;

/// How often the browser reloads a page by itself.
pub(crate) const REFRESH_SECONDS: u32 = 30;

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

/// The full page: the chart, the legend, the last disconnections, and the script that
/// follows a lane link.
pub fn page(timelines: &[TargetTimeline], downs: &[DownEpisode], window: Window) -> String {
    let body = chart(timelines, window);
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
<p class="hint">Times are UTC. The page reloads every {REFRESH_SECONDS} s; select a lane for
its last 30 days.</p>
<div id="chart">{body}</div>
<div class="legend">
  <span><i class="swatch" style="background:#2e7d32"></i>up</span>
  <span><i class="swatch" style="background:#c62828"></i>down</span>
  <span><i class="swatch" style="background:#bdbdbd"></i>unknown</span>
</div>
{downs}
</main>
<script>
// Navigate from script: some engines ignore an <a href> that wraps SVG shapes.
document.getElementById('chart').addEventListener('click', event => {{
  const link = event.target instanceof Element ? event.target.closest('a') : null;
  if (link === null) return;
  event.preventDefault();
  window.location.assign(link.getAttribute('href'));
}}, true);
</script>
</body>
</html>
"#,
        body = body,
        downs = downs,
    )
}

/// The last disconnections, newest first, with how long each one lasted.
fn downs_table(downs: &[DownEpisode], now_ms: i64) -> String {
    let mut output = String::from(
        "<h2>Last 10 disconnections</h2>\n<table>\n\
         <thead><tr><th>target</th><th>started (UTC)</th><th>lasted</th></tr></thead>\n\
         <tbody>\n",
    );

    if downs.is_empty() {
        output.push_str("<tr><td colspan=\"3\">No disconnection recorded yet.</td></tr>\n");
    }

    for episode in downs {
        let started = format_rfc3339(episode.started_at_ms);
        let (lasted, ongoing) = match episode.ended_at_ms {
            Some(end) => (format_span(end - episode.started_at_ms), false),
            None => (format_span(now_ms - episode.started_at_ms), true),
        };
        let _ = writeln!(
            output,
            "<tr><td>{}</td><td>{} {}</td><td{}>{}{}</td></tr>",
            escape(&episode.target),
            &started[5..10],
            &started[11..19],
            if ongoing { " class=\"down\"" } else { "" },
            lasted,
            if ongoing { " and counting" } else { "" }
        );
    }

    output.push_str("</tbody>\n</table>");
    output
}

/// The window label plus the SVG.
fn chart(timelines: &[TargetTimeline], window: Window) -> String {
    let mut output = String::new();
    let _ = write!(
        output,
        "<p class=\"window\">{} → {} (UTC)</p>",
        format_rfc3339(window.from_ms),
        format_rfc3339(window.to_ms)
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
            "<line x1=\"{x:.1}\" y1=\"{}\" x2=\"{x:.1}\" y2=\"{:.0}\" stroke=\"#e6e6e6\"/>",
            TOP_PAD + AXIS_HEIGHT - 4.0,
            height - 4.0
        );
        let _ = write!(
            output,
            "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"10\" \
             fill=\"#666666\">{}</text>",
            TOP_PAD + 8.0,
            escape(&tick_label(tick, span))
        );
        tick += step;
    }

    // Column headers of the per-lane statistics.
    let stats_x = WIDTH - STATS_WIDTH + 8.0;
    let _ = write!(
        output,
        "<text x=\"{stats_x:.1}\" y=\"{:.1}\" font-size=\"10\" fill=\"#666666\">DROPS</text>",
        TOP_PAD + 8.0
    );
    let _ = write!(
        output,
        "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"10\" fill=\"#666666\">DOWN</text>",
        stats_x + 62.0,
        TOP_PAD + 8.0
    );

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
                 rx=\"2\" fill=\"{}\"><title>{} — {} to {}</title></rect>",
                colour(segment.state),
                state_label(segment.state),
                format_rfc3339(segment.from_ms),
                format_rfc3339(segment.to_ms)
            );
        }

        let _ = write!(
            output,
            "<text x=\"{stats_x:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"currentColor\">\
             {}</text>",
            y + ROW_HEIGHT * 0.7,
            stats.drops
        );
        let _ = write!(
            output,
            "<text x=\"{:.1}\" y=\"{:.1}\" font-size=\"11\" fill=\"currentColor\">\
             {}</text>",
            stats_x + 62.0,
            y + ROW_HEIGHT * 0.7,
            format_span(stats.down_ms)
        );
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

fn tick_step(span_ms: i64) -> i64 {
    for step in TICK_STEPS_MS {
        if span_ms / step <= 8 {
            return step;
        }
    }
    (span_ms / 6).max(1)
}

fn tick_label(at_ms: i64, span_ms: i64) -> String {
    let stamp = format_rfc3339(at_ms);
    if span_ms > 36 * 3_600_000 {
        format!("{} {}", &stamp[5..10], &stamp[11..16])
    } else {
        stamp[11..16].to_string()
    }
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
        let rendered = chart(&[timeline()], window);
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
        let rendered = chart(&[timeline()], window);
        assert!(rendered.contains("DROPS"));
        assert!(rendered.contains("DOWN"));
        assert!(rendered.contains("1h00"));
        assert!(rendered.contains("<a href=\"/target?target=1.1.1.1\">"));
    }

    #[test]
    fn the_last_disconnections_are_listed_with_their_duration() {
        let downs = [
            DownEpisode {
                target: "1.1.1.1".to_string(),
                started_at_ms: 7_000_000,
                ended_at_ms: None,
            },
            DownEpisode {
                target: "8.8.8.8".to_string(),
                started_at_ms: 3_600_000,
                ended_at_ms: Some(4_320_000),
            },
        ];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, window);
        assert!(rendered.contains("Last 10 disconnections"));
        assert!(rendered.contains("12m"), "720 s down");
        assert!(
            rendered.contains("and counting"),
            "still down at the window end"
        );
        assert!(rendered.contains("8.8.8.8"));
    }

    #[test]
    fn a_quiet_history_says_so() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], window);
        assert!(rendered.contains("No disconnection recorded yet."));
    }

    #[test]
    fn the_page_reloads_itself_and_keeps_only_the_lane_script() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], window);
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
        let rendered = page(&[timeline()], &[], window);
        assert!(rendered.contains("closest('a')"));
        assert!(rendered.contains("href=\"/target?target=1.1.1.1\""));
    }

    #[test]
    fn an_empty_database_renders_a_hint_instead_of_an_empty_chart() {
        let window = Window {
            from_ms: 0,
            to_ms: 1_000,
        };
        let rendered = chart(&[], window);
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
        let rendered = chart(&[hostile], window);
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
    fn spans_are_formatted_compactly() {
        assert_eq!(format_span(0), "0s");
        assert_eq!(format_span(45_000), "45s");
        assert_eq!(format_span(12 * 60_000), "12m");
        assert_eq!(format_span(3 * 3_600_000 + 7 * 60_000), "3h07");
        assert_eq!(format_span(2 * 86_400_000 + 4 * 3_600_000), "2d04h");
    }
}
