//! Detail page of one target: one bar per day for the last 30 days, then the same
//! numbers in a table, day by day.
//!
//! The numbers come from the disconnections stored in the database, bucketed by UTC
//! day. No chart library and no client-side data handling, like the timeline.

use std::fmt::Write as _;

use crate::store::{DailyDrops, TargetStateRow};
use crate::time::format_rfc3339;

use super::page::{escape, format_span, REFRESH_SECONDS, STYLE};

const WIDTH: f64 = 1000.0;
const HEIGHT: f64 = 300.0;
const LEFT: f64 = 44.0;
const RIGHT: f64 = 12.0;
const TOP: f64 = 16.0;
const BOTTOM: f64 = 34.0;

/// The whole page for one target.
pub fn target_page(target: &TargetStateRow, days: &[DailyDrops]) -> String {
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

    let body = format!(
        "<nav class=\"breadcrumb\" aria-label=\"Breadcrumb\">\
         <a href=\"/\">All targets</a>\
         <span class=\"sep\" aria-hidden=\"true\">/</span>\
         <span aria-current=\"page\">{name}</span>\
         </nav>\n\
         <div class=\"title\">\
         <h1>{name}</h1>\
         <span class=\"badge {state}\">{state}</span>\
         </div>\n\
         <p class=\"hint\">{address}{interface} · times are UTC</p>\n\
         <div class=\"stats\">\
         <div class=\"stat\"><b>{total} disconnections</b><span>last 30 days</span></div>\
         <div class=\"stat\"><b>{down_ms}</b><span>down time in the last 30 days</span></div>\
         </div>\n\
         <h2>Disconnections per day</h2>\n\
         {bars}\n\
         {table}",
        name = escape(&name),
        address = escape(&target.target),
        interface = interface,
        state = target.state.as_str(),
        total = total,
        down_ms = format_span(down_ms),
        bars = bars(days),
        table = days_table(days),
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
</body>
</html>
"#,
        title = escape(title),
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

        if day.drops > 0 {
            let height = (day.drops as f64 / max as f64) * plot;
            let _ = write!(
                svg,
                "<rect x=\"{x:.1}\" y=\"{:.1}\" width=\"{bar_width:.1}\" height=\"{height:.1}\" \
                 rx=\"1\" fill=\"#c62828\"><title>{day_label}: {}</title></rect>",
                TOP + plot - height,
                day.drops
            );
        }
        if index % 5 == 0 {
            let _ = write!(
                svg,
                "<text x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"10\" \
                 fill=\"#666666\">{day_label}</text>",
                x + bar_width / 2.0,
                HEIGHT - 12.0
            );
        }
    }

    svg.push_str("</svg>");
    svg
}

/// The same numbers as the chart, day by day, newest first.
fn days_table(days: &[DailyDrops]) -> String {
    let mut output = String::from(
        "<h2>Day by day</h2>\n<table>\n\
         <thead><tr><th>day (UTC)</th><th class=\"num\">disconnections</th>\
         <th class=\"num\">down time</th></tr></thead>\n\
         <tbody>\n",
    );

    if days.is_empty() {
        output.push_str("<tr><td colspan=\"3\">No day in the window.</td></tr>\n");
    }

    for day in days.iter().rev() {
        let date = format_rfc3339(day.day_start_ms);
        let _ = writeln!(
            output,
            "<tr><td>{}</td><td class=\"num\">{}</td><td class=\"num\">{}</td></tr>",
            &date[..10],
            day.drops,
            format_span(day.down_ms)
        );
    }

    output.push_str("</tbody>\n</table>");
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::TargetState;

    fn target() -> TargetStateRow {
        TargetStateRow {
            target: "1.1.1.1".to_string(),
            interface: Some("eth0".to_string()),
            label: Some("cloudflare".to_string()),
            state: TargetState::Up,
            first_seen_at_ms: 0,
            last_change_at_ms: 0,
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
        let rendered = target_page(&target(), &days);
        assert_eq!(rendered.matches("<rect ").count(), 2);
        assert!(rendered.contains("<title>01-01: 2</title>"));
        assert!(rendered.contains("<title>01-03: 5</title>"));
        assert!(rendered.contains("cloudflare"));
        assert!(rendered.contains("7 disconnections"));
        assert!(rendered.contains("href=\"/\""));
    }

    #[test]
    fn the_table_lists_every_day_newest_first_with_its_down_time() {
        let days = [day(0, 2, 90_000), day(1, 0, 0), day(2, 5, 600_000)];
        let rendered = target_page(&target(), &days);
        assert_eq!(rendered.matches("<tr><td>").count(), 3);
        let first = rendered
            .find("1970-01-03")
            .expect("the newest day comes first");
        let second = rendered.find("1970-01-02").expect("then the one before it");
        let last = rendered.find("1970-01-01").expect("then the oldest");
        assert!(first < second && second < last);
        assert!(rendered
            .contains("<td>1970-01-03</td><td class=\"num\">5</td><td class=\"num\">10m</td>"));
        assert!(rendered
            .contains("<td>1970-01-02</td><td class=\"num\">0</td><td class=\"num\">0s</td>"));
        assert!(rendered
            .contains("<td>1970-01-01</td><td class=\"num\">2</td><td class=\"num\">1m</td>"));
    }

    #[test]
    fn the_table_uses_the_style_shared_with_the_timeline_page() {
        let rendered = target_page(&target(), &[day(0, 1, 60_000)]);
        assert!(
            rendered.contains("<table>"),
            "no class: the table takes the width of main, like the chart above it"
        );
        assert!(
            rendered.contains("<th class=\"num\">disconnections</th>"),
            "the numbers are right-aligned by the shared rule"
        );
        assert!(!rendered.contains("class=\"numeric\""));
    }

    #[test]
    fn a_quiet_month_states_zero_disconnections() {
        let days = [day(0, 0, 0), day(1, 0, 0)];
        let rendered = target_page(&target(), &days);
        assert!(!rendered.contains("<rect "));
        assert!(rendered.contains("0 disconnections"));
    }

    #[test]
    fn a_busiest_day_of_one_labels_the_axis_without_repeating_a_value() {
        let rendered = target_page(&target(), &[day(0, 1, 1_000)]);
        assert_eq!(rendered.matches(">1</text>").count(), 1);
        assert_eq!(rendered.matches(">0</text>").count(), 1);
    }

    #[test]
    fn the_way_back_sits_at_the_top_of_the_page() {
        let rendered = target_page(&target(), &[day(0, 1, 1_000)]);
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
        let rendered = target_page(&target(), &days);
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
        let rendered = target_page(&hostile, &[day(0, 1, 1_000)]);
        assert!(!rendered.contains("<script>"));
        assert!(rendered.contains("&lt;script&gt;"));
    }
}
