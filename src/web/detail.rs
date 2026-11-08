//! Detail page of one target: one bar per day for the last 30 days.
//!
//! The numbers come from the down transitions stored in the database, bucketed by
//! UTC day. No chart library and no client-side data handling, like the timeline.

use std::fmt::Write as _;

use crate::store::{DailyDrops, TargetStateRow};
use crate::time::format_rfc3339;

use super::page::{escape, REFRESH_SECONDS, STYLE};

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

    let body = format!(
        "<h1>{name}</h1>\n\
         <p class=\"hint\">{address}{interface} · state {state} · {total} disconnections in 30 \
         days · days are UTC</p>\n\
         {bars}\n\
         <p><a href=\"/\">← back to all targets</a></p>",
        name = escape(&name),
        address = escape(&target.target),
        interface = interface,
        state = target.state.as_str(),
        total = total,
        bars = bars(days),
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

    for step in 0..=2 {
        let value = max as f64 * (step as f64 / 2.0);
        let y = TOP + plot - (value / max as f64) * plot;
        let _ = write!(
            svg,
            "<line x1=\"{LEFT}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\" stroke=\"#e6e6e6\"/>",
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

    fn day(index: i64, drops: u32) -> DailyDrops {
        DailyDrops {
            day_start_ms: index * 86_400_000,
            drops,
        }
    }

    #[test]
    fn the_page_draws_one_bar_per_day_with_drops() {
        let days = [day(0, 2), day(1, 0), day(2, 5)];
        let rendered = target_page(&target(), &days);
        assert_eq!(rendered.matches("<rect ").count(), 2);
        assert!(rendered.contains("<title>01-01: 2</title>"));
        assert!(rendered.contains("<title>01-03: 5</title>"));
        assert!(rendered.contains("cloudflare"));
        assert!(rendered.contains("7 disconnections"));
        assert!(rendered.contains("href=\"/\""));
    }

    #[test]
    fn a_quiet_month_states_zero_disconnections() {
        let days = [day(0, 0), day(1, 0)];
        let rendered = target_page(&target(), &days);
        assert!(!rendered.contains("<rect "));
        assert!(rendered.contains("0 disconnections"));
    }

    #[test]
    fn the_target_label_is_escaped() {
        let mut hostile = target();
        hostile.label = Some("<script>alert(1)</script>".to_string());
        let rendered = target_page(&hostile, &[day(0, 1)]);
        assert!(!rendered.contains("<script>"));
        assert!(rendered.contains("&lt;script&gt;"));
    }
}
