//! Detail page of one target: one bar per day for the last 30 days, the last
//! faults of the target, then the same numbers in a table, day by day.
//!
//! The numbers come from the faults stored in the database, bucketed by UTC
//! day. No chart library and no client-side data handling, like the timeline.

use std::fmt::Write as _;

use crate::history::{DailyDrops, Fault, TargetState, TargetStateRow, TimelineSegment};
use crate::time::format_rfc3339;

use super::encode;
use super::page::{escape, faults_table, format_runs, format_span, runs_stats};
use super::stats;

const WIDTH: f64 = 1000.0;
const HEIGHT: f64 = 300.0;
const LEFT: f64 = 44.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 26.0;
const BOTTOM: f64 = 34.0;
const DAY_MS: i64 = 86_400_000;

/// The periods the page offers, in days, shortest first: the same reading direction the
/// timeline uses for its windows.
pub const PERIODS: [u32; 5] = [30, 60, 90, 180, 360];

/// The period shown when the query names none.
pub const DEFAULT_PERIOD_DAYS: u32 = 30;

/// `90d` or `90` selects one of the offered periods; anything else is ignored, so a
/// hand-edited query falls back to the default instead of failing.
pub fn parse_period(value: &str) -> Option<u32> {
    let days: u32 = value.strip_suffix('d').unwrap_or(value).parse().ok()?;
    PERIODS.contains(&days).then_some(days)
}

/// The whole page for one target. `segments` is the same target over the same 30 days:
/// the run boundaries are not in the aggregated day buckets, so the per-day averages of
/// the `up` runs are computed from the segments. `recent` is the last faults of
/// the target, newest first.
pub fn target_page(
    target: &TargetStateRow,
    days: &[DailyDrops],
    segments: &[TimelineSegment],
    recent: &[Fault],
    now_ms: i64,
    period_days: u32,
) -> String {
    let name = target
        .label
        .clone()
        .unwrap_or_else(|| target.target.clone());
    let address = escape(&target.target);
    // The toolbar states what the page shows, on one line: the way back to the list, the
    // target — its label with the address in parentheses, or the address alone — and the
    // state with how long it has held.
    let identity = match &target.label {
        Some(label) => format!(
            "<span class=\"target-label\">{}</span> (<span class=\"target-address\">{address}</span>)",
            escape(label)
        ),
        None => format!("<span class=\"target-address\">{address}</span>"),
    };
    let period = format!("{period_days} days");
    let total: u32 = days.iter().map(|day| day.drops).sum();
    let down_ms: i64 = days.iter().map(|day| day.down_ms).sum();
    let runs = runs_stats(segments);
    let longest = match stats::longest_run(segments) {
        Some(longest_ms) => format_span(longest_ms),
        None => "—".to_string(),
    };
    // The state is the first widget of the page, ahead of the figures of the period: its
    // value is the state, and its caption how long that state has held. A target that has
    // never answered has no stretch to measure, so it states the state alone.
    let held = match target.state {
        TargetState::Unknown => "current state".to_string(),
        _ => format!(
            "for {}",
            format_span((now_ms - target.last_change_at_ms).max(0))
        ),
    };

    let title = format!(
        "{name} — last {period}",
        name = escape(&name),
        period = period
    );
    let avg_up_hint = match runs.count {
        0 => format!("no up run in the last {period}"),
        1 => format!("1 up run in the last {period}: no spread to state"),
        count => format!("mean of {count} up runs in the last {period}"),
    };

    super::templates::load_template("detail")
        .replace("{title}", &title)
        .replace("{brand}", &super::templates::brand())
        .replace("{identity}", &identity)
        .replace("{state}", target.state.as_str())
        .replace("{held}", &held)
        .replace("{periods}", &period_links(&target.target, period_days))
        .replace("{total}", &total.to_string())
        .replace("{period}", &period)
        .replace("{down_ms}", &format_span(down_ms))
        .replace("{avg_up}", &format_runs(runs))
        .replace("{longest}", &longest)
        .replace("{avg_up_hint}", &avg_up_hint)
        .replace("{bars}", &bars(days))
        .replace(
            "{recent}",
            &faults_table(
                recent,
                now_ms,
                &format!("?target={}", encode(&target.target)),
            ),
        )
        .replace("{table}", &days_table(days, segments, now_ms))
}

/// The links that change the period of the page, the selected one marked.
fn period_links(target: &str, period_days: u32) -> String {
    let mut output = String::from("<nav class=\"windows\" aria-label=\"Period\">");
    for days in PERIODS {
        let current = if days == period_days {
            " aria-current=\"page\""
        } else {
            ""
        };
        let _ = write!(
            output,
            "<a href=\"/target?target={encoded}&days={days}d\"{current}>{days}d</a>",
            encoded = encode(target),
        );
    }
    output.push_str("</nav>");
    output
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
    let label_step = (days.len() / 6).max(1);
    // A count needs room above its bar. Past about seven weeks the columns are a few
    // pixels apart and the numbers would overlap into an unreadable row of digits; the
    // table below keeps stating every day exactly.
    let show_counts = slot >= 24.0;

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
        if show_counts {
            let colour = if day.drops > 0 { "#c62828" } else { "#666666" };
            let _ = write!(
                svg,
                "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"10\" \
                 fill=\"{colour}\">{}</text>",
                x + bar_width / 2.0,
                y_bar - 5.0,
                day.drops
            );
        }
        // A longer period keeps about six dated labels on the axis instead of one per
        // five days, which would overlap once the bars get thin.
        if index % label_step == 0 {
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
        "<h2>Day by day</h2>\n<table class=\"days\" role=\"table\">\n\
         <thead role=\"rowgroup\"><tr role=\"row\">\
         <th role=\"columnheader\">day (UTC)</th>\
         <th role=\"columnheader\" class=\"num\">faults</th>\
         <th role=\"columnheader\" class=\"num\">down time</th>\
         <th role=\"columnheader\" class=\"num\" \
         title=\"average up run of each day; the current day states the last 24 hours\">\
         avg up</th></tr></thead>\n\
         <tbody role=\"rowgroup\">\n",
    );

    if days.is_empty() {
        output.push_str(
            "<tr role=\"row\"><td role=\"cell\" colspan=\"4\">No day in the window.</td></tr>\n",
        );
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
                "<td role=\"cell\" class=\"num current\" \
                 title=\"average up run over the last 24 hours: {}\">\
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
                "<td role=\"cell\" class=\"num\" title=\"{}\">{}</td>",
                run_hint(summary.count),
                value_with_max(summary, pieces.iter().copied().max())
            )
        };
        let _ = writeln!(
            output,
            "<tr role=\"row\"><td role=\"cell\">\
             <time datetime=\"{date}\" data-format=\"date\" class=\"nowrap\">{}</time></td>\
             <td role=\"cell\" class=\"num\">{}</td>\
             <td role=\"cell\" class=\"num\">{}</td>{}</tr>",
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
/// and a storm of short ones has a mean that describes neither. The mean and its spread stay
/// on one line — `7h54±6h12` is one figure — and only the longest run starts a line of its own.
fn value_with_max(runs: stats::RunStats, longest: Option<i64>) -> String {
    match longest {
        Some(longest_ms) => format!(
            "<span class=\"nowrap\">{}</span><br><span class=\"sub\">max {}</span>",
            format_runs(runs),
            format_span(longest_ms)
        ),
        None => format!("<span class=\"nowrap\">{}</span>", format_runs(runs)),
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
    use crate::history::TargetState;
    use crate::web::templates::stylesheet;

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
        let rendered = target_page(&target(), &days, &[], &[], NOW, DEFAULT_PERIOD_DAYS);
        assert_eq!(rendered.matches("<rect ").count(), 2);
        assert!(rendered.contains("data-format=\"day\" data-suffix=\": 2\">01-01: 2</title>"));
        assert!(rendered.contains("data-suffix=\": 5\">01-03: 5</title>"));
        assert!(rendered.contains("cloudflare"));
        assert!(rendered.contains("7 faults"));
        assert!(rendered.contains("class=\"back-button\""));
    }

    #[test]
    fn the_table_lists_every_day_newest_first_with_its_down_time() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days, &[], &[], NOW, DEFAULT_PERIOD_DAYS);
        assert_eq!(
            rendered
                .matches("<tr role=\"row\"><td role=\"cell\">")
                .count(),
            3
        );
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
            "<td role=\"cell\"><time datetime=\"1970-01-03T00:00:00.000Z\" data-format=\"date\" class=\"nowrap\">1970-01-03</time></td>\
             <td role=\"cell\" class=\"num\">5</td><td role=\"cell\" class=\"num\">10m</td>"
        ));
        assert!(rendered.contains(
            "<td role=\"cell\"><time datetime=\"1970-01-02T00:00:00.000Z\" data-format=\"date\" class=\"nowrap\">1970-01-02</time></td>\
             <td role=\"cell\" class=\"num\">0</td><td role=\"cell\" class=\"num\">0s</td>"
        ));
        assert!(rendered.contains(
            "<td role=\"cell\"><time datetime=\"1970-01-01T00:00:00.000Z\" data-format=\"date\" class=\"nowrap\">1970-01-01</time></td>\
             <td role=\"cell\" class=\"num\">2</td><td role=\"cell\" class=\"num\">1m</td>"
        ));
    }

    #[test]
    fn the_last_faults_sit_above_the_day_by_day_table() {
        let recent = [Fault {
            event_id: 1,
            target: "1.1.1.1".to_string(),
            label: Some("cloudflare".to_string()),
            interface: None,
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
            cause: None,
            cause_message: None,
        }];
        let rendered = target_page(
            &target(),
            &[day(0, 1, 720_000)],
            &[],
            &recent,
            NOW,
            DEFAULT_PERIOD_DAYS,
        );

        let list = rendered
            .find("Last 10 faults")
            .expect("the page lists the last faults of the target");
        let table = rendered.find("Day by day").expect("the day-by-day table");
        assert!(
            list < table,
            "the list comes before the table, right under the chart"
        );
        assert!(
            rendered.contains(
                "<td role=\"cell\"><span class=\"fault-name\">cloudflare</span>\
                 <span class=\"fault-key\">1.1.1.1</span></td>\
                 <td role=\"cell\"><time datetime=\"1970-01-01T01:00:00.000Z\">\
                 <span class=\"nowrap\">1970-01-01</span> \
                 <span class=\"nowrap\">01:00:00 UTC</span></time></td>\
                 <td role=\"cell\" class=\"cause-cell\"></td><td role=\"cell\" class=\"num\">12m</td>"
            ),
            "the row names the target and states how long it was down"
        );
    }

    #[test]
    fn a_target_without_a_drop_still_states_the_empty_list() {
        let rendered = target_page(&target(), &[day(0, 0, 0)], &[], &[], NOW, 30);
        assert!(rendered.contains("No fault recorded yet."));
    }

    #[test]
    fn the_table_uses_the_style_shared_with_the_timeline_page() {
        let rendered = target_page(
            &target(),
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(
            rendered.contains("<table class=\"days\" role=\"table\">"),
            "no class: the table takes the width of main, like the chart above it"
        );
        assert!(
            rendered.contains("<th role=\"columnheader\" class=\"num\">faults</th>"),
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
        let rendered = target_page(&target(), &days, &[], &[], NOW, DEFAULT_PERIOD_DAYS);
        assert!(!rendered.contains("<rect "), "no day draws a bar");
        assert_eq!(
            rendered
                .matches("text-anchor=\"middle\" font-size=\"10\" fill=\"#666666\">0</text>")
                .count(),
            2,
            "every quiet day states its zero at the baseline, next to the axis zero"
        );
        assert!(rendered.contains("0 faults"));
    }

    #[test]
    fn a_busiest_day_of_one_labels_the_axis_without_repeating_a_value() {
        let rendered = target_page(
            &target(),
            &[day(0, 1, 1_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
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
    fn the_period_is_one_of_the_offered_spans() {
        for days in PERIODS {
            assert_eq!(parse_period(&format!("{days}d")), Some(days));
            assert_eq!(parse_period(&days.to_string()), Some(days));
        }
        assert_eq!(parse_period("45d"), None, "not offered");
        assert_eq!(parse_period("30dd"), None);
        assert_eq!(parse_period(""), None);
        assert_eq!(parse_period("abc"), None);
    }

    #[test]
    fn the_page_offers_every_period_and_marks_the_one_shown() {
        let rendered = target_page(&target(), &[day(0, 1, 60_000)], &[], &[], NOW, 30);
        for days in PERIODS {
            assert!(
                rendered.contains(&format!("href=\"/target?target=1.1.1.1&days={days}d\"")),
                "{days} is offered"
            );
        }
        assert!(rendered
            .contains("<a href=\"/target?target=1.1.1.1&days=30d\" aria-current=\"page\">30d</a>"));
        assert_eq!(
            rendered.matches("aria-current=\"page\"").count(),
            1,
            "only the selected period (breadcrumb removed)"
        );
    }

    #[test]
    fn a_longer_period_restates_every_figure_it_covers() {
        let days: Vec<DailyDrops> = (0..90).map(|index| day(index, 1, 60_000)).collect();
        let rendered = target_page(&target(), &days, &[], &[], NOW, 90);
        assert!(rendered.contains("last 90 days"));
        assert!(rendered.contains("90 faults"));
        assert!(rendered.contains("down time in the last 90 days"));
        assert!(rendered.contains("avg up run in 90 days"));
        assert!(
            rendered.contains("last 90 days</title>"),
            "the title follows"
        );
        assert!(rendered
            .contains("<a href=\"/target?target=1.1.1.1&days=90d\" aria-current=\"page\">90d</a>"));
        assert_eq!(
            rendered
                .matches("<tr role=\"row\"><td role=\"cell\">")
                .count(),
            90,
            "one row per day of the chosen period"
        );
    }

    #[test]
    fn a_year_of_days_keeps_the_axis_readable() {
        let days: Vec<DailyDrops> = (0..360).map(|index| day(index, 1, 60_000)).collect();
        let rendered = target_page(&target(), &days, &[], &[], NOW, 360);
        let labels = rendered.matches("data-format=\"day\">").count();
        assert!(
            (6..=7).contains(&labels),
            "about six dated labels, not one per five days: {labels}"
        );
        assert_eq!(rendered.matches("<rect ").count(), 360);
        assert!(
            !rendered.contains("text-anchor=\"middle\" font-size=\"10\" fill=\"#c62828\">1</text>"),
            "the counts are dropped when the columns are too close to carry them"
        );
    }

    #[test]
    fn the_counts_keep_their_place_while_the_columns_have_room() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days, &[], &[], NOW, 30);
        assert!(rendered.contains("fill=\"#c62828\">2</text>"));
        assert!(rendered.contains("fill=\"#666666\">0</text>"));
    }

    #[test]
    fn the_way_back_sits_at_the_top_of_the_page() {
        let rendered = target_page(
            &target(),
            &[day(0, 1, 1_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        let back = rendered
            .find("class=\"back-button\"")
            .expect("the back button in the toolbar");
        // The banner of a held reload opens with a small `svg` of its own, so the chart is
        // the first `svg` the generator draws, not the first one in the document.
        let chart = rendered
            .find("<svg xmlns=\"http://www.w3.org/2000/svg\"")
            .expect("the chart");
        assert!(
            back < chart,
            "the back button comes before the chart, so it needs no scrolling"
        );
        assert_eq!(rendered.matches("class=\"back-button\"").count(), 1);
        assert!(
            rendered.contains("<a class=\"back-button\" href=\"/\">HOME</a>"),
            "the way back is a link to the timeline, not a step into the history"
        );
    }

    #[test]
    fn the_page_states_the_state_and_the_total_down_time() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days, &[], &[], NOW, DEFAULT_PERIOD_DAYS);
        assert!(
            rendered.contains("<div class=\"stat state up\"><b>up</b>"),
            "the state opens the row of widgets, on a tile of its own colour"
        );
        assert!(rendered.contains("<b>7 faults</b>"));
        assert!(
            rendered.contains("<b>11m</b>"),
            "90 s and 10 min summed into one figure"
        );
    }

    #[test]
    fn the_target_label_is_escaped() {
        let mut hostile = target();
        hostile.label = Some("<script>alert(1)</script>".to_string());
        let rendered = target_page(
            &hostile,
            &[day(0, 1, 1_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
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
        let rendered = target_page(
            &target(),
            &[day(0, 1, 300_000)],
            &runs(),
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
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
        let rendered = target_page(
            &target(),
            &[day(0, 1, 60_000)],
            &down,
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
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
        let rendered = target_page(&target(), &days, &crossing, &[], NOW, DEFAULT_PERIOD_DAYS);
        assert!(
            rendered.contains(
                "<td role=\"cell\" class=\"num\" title=\"1 up run that day\">\
                 <span class=\"nowrap\">3h00</span>\
                 <br><span class=\"sub\">max 3h00</span></td>"
            ),
            "three hours before midnight"
        );
        assert!(
            rendered.contains(
                "<td role=\"cell\" class=\"num\" title=\"1 up run that day\">\
                 <span class=\"nowrap\">3h00</span>\
                 <br><span class=\"sub\">max 3h00</span></td>"
            ),
            "and three after it"
        );
    }

    #[test]
    fn the_longest_run_sits_next_to_the_mean_it_outweighs() {
        // One hour and one hour and a half: the mean is neither, and the longest run is
        // what says the link was not uniformly bad.
        let rendered = target_page(
            &target(),
            &[day(0, 1, 300_000)],
            &runs(),
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(
            rendered.contains(
                "<span class=\"nowrap\">1h15±15m</span><br><span class=\"sub\">max 1h30</span>"
            ),
            "the 24-hour cell states the longest run of the same window"
        );
    }

    /// Under the width where four columns stop fitting, each table states its values as two
    /// columns of two lines, and its header takes the same mapping. The roles in the markup are
    /// what keep the cells cells for a screen reader while the display moves.
    #[test]
    fn both_tables_state_their_roles_and_their_two_columns() {
        let style = stylesheet();
        let block = style
            .find("@media (max-width: 700px) {")
            .expect("one width, used by both tables");
        let block = &style[block..];
        for rule in [
            "table.faults tr > *:nth-child(1) { grid-area: 1 / 1; }",
            "table.faults tr > *:nth-child(2) { grid-area: 2 / 1; }",
            "table.faults tr > *:nth-child(3) { grid-area: 1 / 2; }",
            "table.faults tr > *:nth-child(4) { grid-area: 2 / 2; }",
            "table.days tr > *:nth-child(1) { grid-area: 1 / 1; text-align: left; }",
            "table.days tr > *:nth-child(2) { grid-area: 1 / 2; }",
            "table.days tr > *:nth-child(3) { grid-area: 2 / 1; text-align: left; }",
            "table.days tr > *:nth-child(4) { grid-area: 2 / 2; }",
            "table.faults td[colspan], table.days td[colspan] { grid-column: 1 / -1; }",
            "tr.fault { height: auto; }",
        ] {
            assert!(block.contains(rule), "the collapsed layout states {rule}");
        }

        let rendered = target_page(
            &target(),
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(
            rendered.contains("<table class=\"days\" role=\"table\">"),
            "the per-day table names itself and keeps its role"
        );
        assert!(
            rendered.contains("<tr role=\"row\"><td role=\"cell\">"),
            "a day row keeps its row and its cell"
        );
        assert!(
            rendered.contains("<td role=\"cell\" class=\"num\">"),
            "a figure cell keeps its cell"
        );
        assert!(
            rendered.contains("<table class=\"faults\" role=\"table\">"),
            "the fault table the page shares keeps its role too"
        );
    }

    /// A mean and its spread are one figure — `7h54±6h12` never breaks after the `±` — and a
    /// day is a date that never breaks; only the longest run starts a line of its own.
    #[test]
    fn the_mean_and_the_day_are_never_broken() {
        let rendered = target_page(
            &target(),
            &[day(0, 1, 300_000)],
            &runs(),
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(
            rendered.contains("<span class=\"nowrap\">1h15±15m</span>"),
            "the mean and its spread stay on one line"
        );
        assert!(
            rendered.contains("data-format=\"date\" class=\"nowrap\""),
            "the day of a row is one unbreakable date"
        );
        assert!(
            !rendered.contains(">1h15±15m<br>"),
            "a bare mean would let the browser break it after the ±"
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
        let rendered = target_page(
            &target(),
            &[day(0, 0, 0)],
            &running,
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(rendered.contains(
            "<td role=\"cell\" class=\"num current\" title=\"average up run over the last 24 hours: \
             1 up run in the last 24 hours\"><span class=\"tag\">last 24 h</span>\
             <span class=\"nowrap\">4h10</span><br><span class=\"sub\">max 4h10</span></td>"
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
        let rendered = target_page(&target(), &days, &[], &[], NOW, DEFAULT_PERIOD_DAYS);
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

    /// The first widget is the state itself: its value is the state, its caption how long
    /// that state has held. It is the first of the four, ahead of the figures of the period.
    #[test]
    fn the_first_widget_states_the_state_and_how_long_it_has_held() {
        let rendered = target_page(
            &target(),
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        let stats = rendered
            .find("<div class=\"stats\">")
            .expect("the row of widgets");
        let first = &rendered[stats..];
        let first = &first[..first.find("</div>").expect("the first widget closes")];
        assert!(
            first.contains("<b>up</b><span>for 16m</span>"),
            "the state and its streak share the first widget: {first}"
        );
        assert_eq!(
            rendered.matches("<div class=\"stat").count()
                - rendered.matches("<div class=\"stats\">").count(),
            4,
            "the state, the faults, the down time, and the average up run"
        );
    }

    /// A narrow window must not decide the shape of the widget row: the four stand four
    /// across, then two and two, then the state alone above the other three, in that order at
    /// every width, and no figure inside a widget is ever split.
    #[test]
    fn the_four_widgets_keep_their_order_and_take_the_three_stated_shapes() {
        let style = stylesheet();
        assert!(
            style.contains("grid-template-columns: repeat(4, 1fr);"),
            "four widgets share the row of a wide page"
        );
        assert!(
            style.contains(
                "@media (max-width: 700px) {\n  .stats { grid-template-columns: repeat(2, 1fr); }"
            ),
            "they halve when four stop fitting"
        );
        assert!(
            style.contains("@media (max-width: 460px) {")
                && style.contains("grid-template-columns: repeat(3, 1fr); gap: 8px;"),
            "the three figures share the narrowest row"
        );
        assert!(
            style.contains(".stat.state { grid-column: 1 / -1; }"),
            "the state owns a row of its own rather than one tile of three"
        );
        assert!(
            !style.contains("flex: 1 1 150px"),
            "a flex basis let the browser choose how many widgets share a line"
        );
        assert_eq!(
            style.matches("grid-template-columns: repeat(").count(),
            3,
            "three shapes of widget row, and no fourth"
        );

        let rendered = target_page(
            &target(),
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        let order: Vec<&str> = [
            "class=\"stat state up\"",
            "faults</b>",
            "down time in the",
            "avg up run in ",
        ]
        .into_iter()
        .collect();
        let mut at = 0;
        for marker in order {
            let found = rendered[at..]
                .find(marker)
                .unwrap_or_else(|| panic!("{marker} follows the widget before it"));
            at += found + marker.len();
        }
    }

    /// A target that is down states that, and how long it has been down: the widget
    /// measures the current state, it does not claim an `up` streak.
    #[test]
    fn a_target_that_is_not_up_states_its_own_state() {
        let mut down = target();
        down.state = TargetState::Down;
        let rendered = target_page(
            &down,
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(rendered
            .contains("<div class=\"stat state down\"><b>down</b><span>for 16m</span></div>"));
    }

    /// A target that has never answered has no stretch to measure, so its widget states
    /// the state and nothing it cannot know.
    #[test]
    fn an_unknown_state_states_no_stretch() {
        let mut unknown = target();
        unknown.state = TargetState::Unknown;
        let rendered = target_page(
            &unknown,
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(rendered.contains(
            "<div class=\"stat state unknown\"><b>unknown</b><span>current state</span></div>"
        ));
    }

    /// The toolbar states what the page shows, on the one line above the chart: the way
    /// back to the list, the target in bold, and its state. The masthead stays the identity
    /// block of every page, with nothing of the target in it.
    #[test]
    fn the_toolbar_states_the_target_the_way_back_and_the_state() {
        let rendered = target_page(
            &target(),
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        let crumbs = rendered
            .find("<p class=\"crumbs\">")
            .expect("one line under the masthead");
        let line = &rendered[crumbs..];
        let line = &line[..line.find("</p>").expect("the line closes")];
        assert!(
            line.contains("<a class=\"back-button\" href=\"/\">HOME</a>"),
            "the way back opens the line: {line}"
        );
        assert!(
            line.contains(
                "<span class=\"target-identity\"><span class=\"target-label\">cloudflare</span> \
                 (<span class=\"target-address\">1.1.1.1</span>)</span>"
            ),
            "the label with the address it names, in parentheses: {line}"
        );
        assert!(
            !line.contains("badge") && !line.contains("for 16m"),
            "the state is a widget of its own, not a third part of the line: {line}"
        );
        let masthead_end = rendered.find("</header>").expect("a closed masthead");
        assert!(
            !rendered[..masthead_end].contains("target-info")
                && !rendered[..masthead_end].contains("1.1.1.1"),
            "the masthead of the detail page is the identity block of every page"
        );
    }

    #[test]
    fn a_target_without_a_label_states_its_address_alone() {
        let mut unnamed = target();
        unnamed.label = None;
        let rendered = target_page(
            &unnamed,
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert_eq!(
            rendered.matches(">1.1.1.1<").count(),
            1,
            "one address, with no empty label beside it"
        );
        assert!(!rendered.contains("class=\"target-label\""));
        assert!(
            rendered.contains(
                "<span class=\"target-identity\"><span class=\"target-address\">1.1.1.1</span></span>"
            ),
            "the address alone, with no empty parentheses"
        );
        assert!(!rendered.contains("()"));
    }

    #[test]
    fn every_instant_is_machine_readable_and_localised_by_the_browser() {
        let rendered = target_page(
            &target(),
            &[day(0, 1, 60_000)],
            &[],
            &[],
            NOW,
            DEFAULT_PERIOD_DAYS,
        );
        assert!(rendered.contains(
            "<time datetime=\"1970-01-01T00:00:00.000Z\" data-format=\"date\" class=\"nowrap\">1970-01-01</time>"
        ));
        assert!(
            rendered.contains("data-datetime=\"1970-01-01T00:00:00.000Z\" data-format=\"day\""),
            "the axis labels carry the instant the browser reformats"
        );
        assert!(rendered.contains("<script src=\"/app.js\"></script>"));
        assert!(
            crate::web::templates::script().contains("Intl.DateTimeFormat"),
            "the shared script localises the instants of the page"
        );
        assert!(!rendered.contains("times are UTC"));
    }
}
