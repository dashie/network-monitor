//! Detail page of one target: one bar per day for the last 30 days, then the same
//! numbers in a table, day by day.
//!
//! The numbers come from the disconnections stored in the database, bucketed by UTC
//! day. No chart library and no client-side data handling, like the timeline.

use std::fmt::Write as _;

use crate::store::{DailyDrops, TargetStateRow, TimelineSegment};
use crate::time::format_rfc3339;

use super::page::{
    escape, format_runs, format_span, runs_stats, up_for, LOCALIZE_SCRIPT, REFRESH_SECONDS, STYLE,
};
use super::stats;

const WIDTH: f64 = 1000.0;
const HEIGHT: f64 = 300.0;
const LEFT: f64 = 44.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 26.0;
const BOTTOM: f64 = 34.0;
const DAY_MS: i64 = 86_400_000;

/// The whole page for one target. `segments` is the same target over the same 30 days:
/// the run boundaries are not in the aggregated day buckets, so the per-day averages of
/// the `up` runs are computed from the segments.
pub fn target_page(
    target: &TargetStateRow,
    days: &[DailyDrops],
    segments: &[TimelineSegment],
    now_ms: i64,
) -> String {
    let name = target
        .label
        .clone()
        .unwrap_or_else(|| target.target.clone());
    let interface = match &target.interface {
        Some(name) => format!(" on {}", escape(name)),
        None => String::new(),
    };
    let total: u32 = days.iter().map(|day| day.drops).sum();
    let down_ms: i64 = days.iter().map(|day| day.down_ms).sum();
    let runs = runs_stats(segments);
    let longest = match stats::longest_run(segments) {
        Some(longest_ms) => format_span(longest_ms),
        None => "—".to_string(),
    };
    let since = match up_for(target.state, target.last_change_at_ms, now_ms) {
        Some(up_for_ms) => format!(
            "<span class=\"since\">for {}</span>",
            format_span(up_for_ms)
        ),
        None => String::new(),
    };

    let body = format!(
        "<nav class=\"breadcrumb\" aria-label=\"Breadcrumb\">\
         <a href=\"/\">All targets</a>\
         <span class=\"sep\" aria-hidden=\"true\">/</span>\
         <span aria-current=\"page\">{name}</span>\
         </nav>\n\
         <div class=\"title\">\
         <h1>{name}</h1>\
         <span class=\"badge {state}\">{state}</span>\
         {since}\
         </div>\n\
         <p class=\"hint\">{address}{interface} · times in your time zone</p>\n\
         <div class=\"stats\">\
         <div class=\"stat\"><b>{total} disconnections</b><span>last 30 days</span></div>\
         <div class=\"stat\"><b>{down_ms}</b><span>down time in the last 30 days</span></div>\
         <div class=\"stat\" title=\"{avg_up_hint}\"><b>{avg_up}</b>\
         <span>avg up run in 30 days · max {longest}</span></div>\
         </div>\n\
         <h2>Disconnections per day</h2>\n\
         {bars}\n\
         {table}",
        name = escape(&name),
        address = escape(&target.target),
        interface = interface,
        state = target.state.as_str(),
        since = since,
        total = total,
        down_ms = format_span(down_ms),
        avg_up = format_runs(runs),
        longest = longest,
        avg_up_hint = match runs.count {
            0 => "no up run in the last 30 days".to_string(),
            1 => "1 up run in the last 30 days: no spread to state".to_string(),
            count => format!("mean of {count} up runs in the last 30 days"),
        },
        bars = bars(days),
        table = days_table(days, segments, now_ms),
    );

    layout(&format!("{name} — last 30 days"), &body)
}

fn layout(title: &str, body: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta http-equiv="refresh" content="{REFRESH_SECONDS}">
<title>{title}</title>
<style>{STYLE}</style>
</head>
<body>
<main>
{body}
</main>
<script>
{localize}
</script>
</body>
</html>
"#,
        title = escape(title),
        localize = LOCALIZE_SCRIPT,
    )
}

/// The bar chart: one bar per day, scaled to the busiest day of the period.
fn bars(days: &[DailyDrops]) -> String {
    let max = days.iter().map(|day| day.drops).max().unwrap_or(0).max(1);
    let track = WIDTH - LEFT - RIGHT;
    let plot = HEIGHT - TOP - BOTTOM;
    let slot = if days.is_empty() {
        track
    } else {
        track / days.len() as f64
    };
    let bar_width = (slot * 0.72).max(1.0);

    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 {WIDTH} {HEIGHT}\" role=\"img\" \
         aria-label=\"Disconnections per day\">"
    );

    // Two grid lines are enough, but a busiest day of one would label both of them "1".
    let steps = if max < 2 { 1 } else { 2 };
    for step in 0..=steps {
        let value = max as f64 * (step as f64 / steps as f64);
        let y = TOP + plot - (value / max as f64) * plot;
        let _ = write!(
            svg,
            "<line x1=\"{LEFT}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"rgba(127,127,127,0.35)\"/>",
            WIDTH - RIGHT
        );
        let _ = write!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" font-size=\"10\" \
             fill=\"#666666\">{}</text>",
            LEFT - 6.0,
            y + 3.0,
            value.round() as i64
        );
    }

    for (index, day) in days.iter().enumerate() {
        let x = LEFT + index as f64 * slot + (slot - bar_width) / 2.0;
        let date = format_rfc3339(day.day_start_ms);
        let day_label = &date[5..10];

        let height = if day.drops > 0 {
            (day.drops as f64 / max as f64) * plot
        } else {
            0.0
        };
        let y_bar = TOP + plot - height;

        if day.drops > 0 {
            let _ = write!(
                svg,
                "<rect x=\"{x:.1}\" y=\"{y_bar:.1}\" width=\"{bar_width:.1}\" \
                 height=\"{height:.1}\" rx=\"1\" fill=\"#c62828\"><title data-datetime=\"{date}\" \
                 data-format=\"day\" data-suffix=\": {}\">{day_label}: {}</title></rect>",
                day.drops, day.drops
            );
        }

        // Every day states its count above where its bar would end, a quiet day included:
        // a missing label reads as missing data, a muted zero as a measured nothing.
        let colour = if day.drops > 0 { "#c62828" } else { "#666666" };
        let _ = write!(
            svg,
            "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"10\" \
             fill=\"{colour}\">{}</text>",
            x + bar_width / 2.0,
            y_bar - 5.0,
            day.drops
        );
        if index % 5 == 0 {
            let _ = write!(
                svg,
                "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"10\" \
                 fill=\"#666666\" data-datetime=\"{date}\" data-format=\"day\">{day_label}</text>",
                x + bar_width / 2.0,
                HEIGHT - 12.0
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

/// The same numbers as the chart, day by day, newest first. A day is a UTC day, so the
/// browser localises the format of the date but keeps the day it belongs to.
///
/// The first row is the current UTC day, which is still running: its average `up` run
/// covers the last 24 hours instead, tagged so nobody reads it as a full day.
fn days_table(days: &[DailyDrops], segments: &[TimelineSegment], now_ms: i64) -> String {
    let mut output = String::from(
        "<h2>Day by day</h2>\n<table>\n\
         <thead><tr><th>day (UTC)</th><th class=\"num\">disconnections</th>\
         <th class=\"num\">down time</th><th class=\"num\" \
         title=\"average up run of each day; the current day states the last 24 hours\">\
         avg up</th></tr></thead>\n\
         <tbody>\n",
    );

    if days.is_empty() {
        output.push_str("<tr><td colspan=\"4\">No day in the window.</td></tr>\n");
    }

    // A run that crosses midnight is split, so each day states the average of the `up`
    // stretches that fall inside it.
    let runs = stats::up_runs_per_day(segments, DAY_MS);
    let current_day_ms = now_ms.div_euclid(DAY_MS) * DAY_MS;

    for day in days.iter().rev() {
        let date = format_rfc3339(day.day_start_ms);
        let average = if day.day_start_ms == current_day_ms {
            let rolling = stats::up_runs_between(segments, now_ms - DAY_MS, now_ms);
            let summary = stats::summarize(&rolling);
            format!(
                "<td class=\"num current\" title=\"average up run over the last 24 hours: {}\">\
                 <span class=\"tag\">last 24 h</span>{}</td>",
                run_hint_24h(summary.count),
                value_with_max(summary, rolling.iter().copied().max())
            )
        } else {
            let pieces = runs
                .get(&day.day_start_ms)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let summary = stats::summarize(pieces);
            format!(
                "<td class=\"num\" title=\"{}\">{}</td>",
                run_hint(summary.count),
                value_with_max(summary, pieces.iter().copied().max())
            )
        };
        let _ = writeln!(
            output,
            "<tr><td><time datetime=\"{date}\" data-format=\"date\">{}</time></td>\
             <td class=\"num\">{}</td><td class=\"num\">{}</td>{}</tr>",
            &date[..10],
            day.drops,
            format_span(day.down_ms),
            average
        );
    }

    output.push_str("</tbody>\n</table>");
    output
}

/// The mean of the runs and, under it, the longest one: a day that holds one long run
/// and a storm of short ones has a mean that describes neither.
fn value_with_max(runs: stats::RunStats, longest: Option<i64>) -> String {
    match longest {
        Some(longest_ms) => format!(
            "{}<br><span class=\"sub\">max {}</span>",
            format_runs(runs),
            format_span(longest_ms)
        ),
        None => format_runs(runs),
    }
}

/// How many runs an average is built on, for the tooltip of a cell.
fn run_hint(count: u32) -> String {
    match count {
        0 => "no up run that day".to_string(),
        1 => "1 up run that day".to_string(),
        count => format!("{count} up runs that day"),
    }
}

/// The same sample, stated over a rolling day instead of a calendar day.
fn run_hint_24h(count: u32) -> String {
    match count {
        0 => "no up run in the last 24 hours".to_string(),
        1 => "1 up run in the last 24 hours".to_string(),
        count => format!("{count} up runs in the last 24 hours"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TargetState;

    /// A fixed instant, so the age of the current `up` state is deterministic.
    const NOW: i64 = 10_000_000;

    fn target() -> TargetStateRow {
        TargetStateRow {
            target: "1.1.1.1".to_string(),
            interface: Some("eth0".to_string()),
            label: Some("cloudflare".to_string()),
            state: TargetState::Up,
            first_seen_at_ms: 0,
            last_change_at_ms: 9_000_000,
            last_rtt_us: None,
        }
    }

    fn day(index: i64, drops: u32, down_ms: i64) -> DailyDrops {
        DailyDrops {
            day_start_ms: index * 86_400_000,
            drops,
            down_ms,
        }
    }

    #[test]
    fn the_page_draws_one_bar_per_day_with_drops() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days, &[], NOW);
        assert_eq!(rendered.matches("<rect ").count(), 2);
        assert!(rendered.contains("data-format=\"day\" data-suffix=\": 2\">01-01: 2</title>"));
        assert!(rendered.contains("data-suffix=\": 5\">01-03: 5</title>"));
        assert!(rendered.contains("cloudflare"));
        assert!(rendered.contains("7 disconnections"));
        assert!(rendered.contains("href=\"/\""));
    }

    #[test]
    fn the_table_lists_every_day_newest_first_with_its_down_time() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days, &[], NOW);
        assert_eq!(rendered.matches("<tr><td>").count(), 3);
        let first = rendered
            .find("<time datetime=\"1970-01-03")
            .expect("the newest day comes first");
        let second = rendered
            .find("<time datetime=\"1970-01-02")
            .expect("then the one before it");
        let last = rendered
            .find("<time datetime=\"1970-01-01")
            .expect("then the oldest");
        assert!(first < second && second < last);
        assert!(rendered.contains(
            "<td><time datetime=\"1970-01-03T00:00:00.000Z\" data-format=\"date\">1970-01-03</time></td>\
             <td class=\"num\">5</td><td class=\"num\">10m</td>"
        ));
        assert!(rendered.contains(
            "<td><time datetime=\"1970-01-02T00:00:00.000Z\" data-format=\"date\">1970-01-02</time></td>\
             <td class=\"num\">0</td><td class=\"num\">0s</td>"
        ));
        assert!(rendered.contains(
            "<td><time datetime=\"1970-01-01T00:00:00.000Z\" data-format=\"date\">1970-01-01</time></td>\
             <td class=\"num\">2</td><td class=\"num\">1m</td>"
        ));
    }

    #[test]
    fn the_table_uses_the_style_shared_with_the_timeline_page() {
        let rendered = target_page(&target(), &[day(0, 1, 60_000)], &[], NOW);
        assert!(
            rendered.contains("<table>"),
            "no class: the table takes the width of main, like the chart above it"
        );
        assert!(
            rendered.contains("<th class=\"num\">disconnections</th>"),
            "the numbers are right-aligned by the shared rule"
        );
        assert!(
            rendered.contains(">avg up</th>"),
            "the column keeps the name the timeline uses"
        );
        assert!(!rendered.contains("class=\"numeric\""));
    }

    #[test]
    fn a_quiet_month_states_zero_disconnections() {
        let days = [day(0, 0, 0), day(1, 0, 0)];
        let rendered = target_page(&target(), &days, &[], NOW);
        assert!(!rendered.contains("<rect "), "no day draws a bar");
        assert_eq!(
            rendered
                .matches("text-anchor=\"middle\" font-size=\"10\" fill=\"#666666\">0</text>")
                .count(),
            2,
            "every quiet day states its zero at the baseline, next to the axis zero"
        );
        assert!(rendered.contains("0 disconnections"));
    }

    #[test]
    fn a_busiest_day_of_one_labels_the_axis_without_repeating_a_value() {
        let rendered = target_page(&target(), &[day(0, 1, 1_000)], &[], NOW);
        assert_eq!(
            rendered
                .matches("text-anchor=\"end\" font-size=\"10\" fill=\"#666666\">1</text>")
                .count(),
            1,
            "the axis names the top value once"
        );
        assert_eq!(rendered.matches("fill=\"#c62828\">1</text>").count(), 1);
    }

    #[test]
    fn the_way_back_sits_at_the_top_of_the_page() {
        let rendered = target_page(&target(), &[day(0, 1, 1_000)], &[], NOW);
        let crumb = rendered
            .find("href=\"/\"")
            .expect("the breadcrumb links to the timeline");
        let chart = rendered.find("<svg").expect("the chart");
        assert!(
            crumb < chart,
            "the way back comes before the chart, so it needs no scrolling"
        );
        assert_eq!(rendered.matches("href=\"/\"").count(), 1);
    }

    #[test]
    fn the_page_states_the_state_and_the_total_down_time() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days, &[], NOW);
        assert!(rendered.contains("<span class=\"badge up\">up</span>"));
        assert!(rendered.contains("<b>7 disconnections</b>"));
        assert!(
            rendered.contains("<b>11m</b>"),
            "90 s and 10 min summed into one figure"
        );
    }

    #[test]
    fn the_target_label_is_escaped() {
        let mut hostile = target();
        hostile.label = Some("<script>alert(1)</script>".to_string());
        let rendered = target_page(&hostile, &[day(0, 1, 1_000)], &[], NOW);
        assert!(!rendered.contains("<script>alert(1)</script>"));
        assert!(rendered.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    }

    fn runs() -> Vec<TimelineSegment> {
        vec![
            TimelineSegment {
                state: TargetState::Up,
                from_ms: 0,
                to_ms: 3_600_000,
            },
            TimelineSegment {
                state: TargetState::Down,
                from_ms: 3_600_000,
                to_ms: 3_900_000,
            },
            TimelineSegment {
                state: TargetState::Up,
                from_ms: 3_900_000,
                to_ms: 9_300_000,
            },
        ]
    }

    #[test]
    fn the_header_states_the_average_up_run_of_the_period() {
        let rendered = target_page(&target(), &[day(0, 1, 300_000)], &runs(), NOW);
        assert!(
            rendered.contains("<b>1h15±15m</b>"),
            "1 h and 1h30 average into 1h15 with a spread of 15 min"
        );
        assert!(rendered.contains("mean of 2 up runs in the last 30 days"));
        assert!(
            rendered.contains("avg up run in 30 days · max 1h30"),
            "the longest run of the period travels with the mean"
        );
    }

    #[test]
    fn a_period_without_any_up_run_states_no_average() {
        let down = [TimelineSegment {
            state: TargetState::Down,
            from_ms: 0,
            to_ms: 60_000,
        }];
        let rendered = target_page(&target(), &[day(0, 1, 60_000)], &down, NOW);
        assert!(rendered.contains("<b>—</b>"));
        assert!(rendered.contains("no up run in the last 30 days"));
    }

    #[test]
    fn each_day_states_the_average_of_the_up_runs_inside_it() {
        // One run that crosses midnight: each day states the piece it holds.
        let crossing = [TimelineSegment {
            state: TargetState::Up,
            from_ms: 45 * 3_600_000,
            to_ms: 51 * 3_600_000,
        }];
        let days = [day(1, 0, 0), day(2, 0, 0)];
        let rendered = target_page(&target(), &days, &crossing, NOW);
        assert!(
            rendered.contains(
                "<td class=\"num\" title=\"1 up run that day\">3h00\
                 <br><span class=\"sub\">max 3h00</span></td>"
            ),
            "three hours before midnight"
        );
        assert!(
            rendered.contains(
                "<td class=\"num\" title=\"1 up run that day\">3h00\
                 <br><span class=\"sub\">max 3h00</span></td>"
            ),
            "and three after it"
        );
    }

    #[test]
    fn the_longest_run_sits_next_to_the_mean_it_outweighs() {
        // One hour and one hour and a half: the mean is neither, and the longest run is
        // what says the link was not uniformly bad.
        let rendered = target_page(&target(), &[day(0, 1, 300_000)], &runs(), NOW);
        assert!(
            rendered.contains("1h15±15m<br><span class=\"sub\">max 1h30</span>"),
            "the 24-hour cell states the longest run of the same window"
        );
    }

    #[test]
    fn the_day_still_running_states_the_average_of_the_last_24_hours() {
        // One run that began before midnight: the day so far holds 2h46 of it, the
        // rolling last 24 hours hold 4h10.
        let running = [TimelineSegment {
            state: TargetState::Up,
            from_ms: -5_000_000,
            to_ms: NOW,
        }];
        let rendered = target_page(&target(), &[day(0, 0, 0)], &running, NOW);
        assert!(rendered.contains(
            "<td class=\"num current\" title=\"average up run over the last 24 hours: \
             1 up run in the last 24 hours\"><span class=\"tag\">last 24 h</span>\
             4h10<br><span class=\"sub\">max 4h10</span></td>"
        ));
        assert_eq!(
            rendered.matches("class=\"tag\"").count(),
            1,
            "only the current day states a rolling window"
        );
    }

    #[test]
    fn each_bar_states_its_own_count() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days, &[], NOW);
        assert!(rendered.contains("fill=\"#c62828\">2</text>"));
        assert!(rendered.contains("fill=\"#c62828\">5</text>"));
        assert!(
            rendered.contains("text-anchor=\"middle\" font-size=\"10\" fill=\"#666666\">0</text>"),
            "a quiet day still states its zero, in a muted colour: no label would read \
             as missing data"
        );
        assert!(
            !rendered.contains("fill=\"#c62828\">0</text>"),
            "and never paints the zero like a count of drops"
        );
    }

    #[test]
    fn the_badge_says_how_long_the_target_has_been_up() {
        let rendered = target_page(&target(), &[day(0, 1, 60_000)], &[], NOW);
        assert!(rendered.contains("<span class=\"since\">for 16m</span>"));
    }

    #[test]
    fn a_target_that_is_not_up_states_no_streak() {
        let mut down = target();
        down.state = TargetState::Down;
        let rendered = target_page(&down, &[day(0, 1, 60_000)], &[], NOW);
        assert!(!rendered.contains("class=\"since\""));
    }

    #[test]
    fn every_instant_is_machine_readable_and_localised_by_the_browser() {
        let rendered = target_page(&target(), &[day(0, 1, 60_000)], &[], NOW);
        assert!(rendered.contains(
            "<time datetime=\"1970-01-01T00:00:00.000Z\" data-format=\"date\">1970-01-01</time>"
        ));
        assert!(
            rendered.contains("data-datetime=\"1970-01-01T00:00:00.000Z\" data-format=\"day\""),
            "the axis labels carry the instant the browser reformats"
        );
        assert!(
            rendered.contains("Intl.DateTimeFormat"),
            "the page localises its own instants"
        );
        assert!(!rendered.contains("times are UTC"));
    }
}
