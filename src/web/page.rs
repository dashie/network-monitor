//! Server-side HTML and SVG for the read-only web page.
//!
//! The page is generated from the same segments the query path uses; the browser
//! only asks for another window. No chart library, no client-side data handling.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::history::{Fault, TargetState, TargetTimeline, TimelineSegment};
use crate::time::format_rfc3339;

use super::encode;
use super::stats::{self, RunStats};

const WIDTH: f64 = 1000.0;
const LABEL_WIDTH: f64 = 190.0;
const STATS_WIDTH: f64 = 248.0;
const ROW_HEIGHT: f64 = 24.0;
const ROW_GAP: f64 = 6.0;
const TOP_PAD: f64 = 6.0;
const AXIS_HEIGHT: f64 = 20.0;

/// The radius of the two outer ends of a lane. The bands inside the lane meet square,
/// so only the left of the first band and the right of the last one are rounded.
const CAP_RADIUS: f64 = 2.0;

/// The narrowest a fault is ever drawn. A few seconds inside a day-wide window are a
/// fraction of a pixel, and a fault that cannot be seen is not shown at all.
const MIN_BAND_WIDTH: f64 = 2.0;

/// The fill of a fault drawn wider than its true duration, so a band that lies about its
/// width does not lie about its severity: orange states "this is a short fault, widened".
const SHORT_FAULT_FILL: &str = "#ef6c00";
/// A reachable but slow target: light green, in the same family as `up`, so it reads as
/// "still up" while its lightness keeps it apart from the dark green of a healthy lane,
/// the orange of a widened short fault, the red of a fault and the grey of `unknown`.
pub const DEGRADED_FILL: &str = "#8bc34a";

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
const COLUMNS: [(f64, &str, &str); 4] = [
    (48.0, "DROPS", "faults in the window"),
    (122.0, "AVG UP", "average up run, with its spread"),
    (172.0, "MAX", "longest up run in the window"),
    (232.0, "UP FOR", "how long it has been up without a drop"),
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

/// The full page: the windows, the chart, the legend, the last disconnections.
pub fn page(
    timelines: &[TargetTimeline],
    downs: &[Fault],
    up_since: &HashMap<String, i64>,
    window: Window,
) -> String {
    let body = chart(timelines, up_since, window);
    let downs = faults_table(downs, window.to_ms, "");

    super::templates::load_template("page")
        .replace("{title}", "Uptime Monitor")
        .replace("{brand}", &super::templates::brand())
        .replace("{span}", &window_span(window))
        .replace("{windows}", &window_links(window))
        .replace("{body}", &body)
        .replace("{downs}", &downs)
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

/// The last faults, newest first, with why each one started and how long it lasted.
///
/// Shared with the detail page, which lists the same table for one target: `download` is
/// the query that narrows the export to that target, and is empty on the timeline, where
/// the export covers every target. A row with a cause is a control: clicking it opens the
/// standard message in a row of its own, and the `title` states it for a browser without
/// JavaScript.
pub(super) fn faults_table(faults: &[Fault], now_ms: i64, download: &str) -> String {
    let mut output = format!(
        "<h2>Last 10 faults\
         <a class=\"download\" href=\"/export/faults.csv.gz{download}\">Download History</a>\
         </h2>\n<div class=\"table-scroll\">\n<table class=\"faults\" role=\"table\">\n\
         <thead role=\"rowgroup\"><tr role=\"row\"><th role=\"columnheader\">target</th>\
         <th role=\"columnheader\">started</th>\
         <th role=\"columnheader\" class=\"cause-head\">cause</th>\
         <th role=\"columnheader\" class=\"num lasted-head\">lasted</th></tr></thead>\n\
         <tbody role=\"rowgroup\">\n"
    );

    if faults.is_empty() {
        output.push_str(
            "<tr role=\"row\"><td role=\"cell\" colspan=\"4\">No fault recorded yet.</td></tr>\n",
        );
    }

    for (index, fault) in faults.iter().enumerate() {
        let (lasted, ongoing) = match fault.ended_at_ms {
            Some(end) => (format_span(end - fault.started_at_ms), false),
            None => (format_span(now_ms - fault.started_at_ms), true),
        };
        let name = fault.label.clone().unwrap_or_else(|| fault.target.clone());
        // With a label the key sits under it, muted; without one the key is the name.
        let key = match fault.label {
            Some(_) => format!("<span class=\"fault-key\">{}</span>", escape(&fault.target)),
            None => String::new(),
        };
        let message = fault.cause_message.as_deref().unwrap_or_default();
        let cause = match fault.cause.as_deref() {
            Some(code) => format!(
                "<button type=\"button\" class=\"cause\" aria-expanded=\"false\" \
                 aria-controls=\"cause-{index}\" title=\"{}\">{}</button>",
                escape(message),
                escape(code)
            ),
            None => String::new(),
        };

        let _ = writeln!(
            output,
            "<tr role=\"row\"{expand}><td role=\"cell\">\
             <span class=\"fault-name\">{name}</span>{key}</td>\
             <td role=\"cell\">{shown}</td>\
             <td role=\"cell\" class=\"cause-cell\">{cause}</td>\
             <td role=\"cell\"{class}>{lasted}{tail}</td></tr>",
            expand = if fault.cause.is_some() {
                " class=\"fault\""
            } else {
                ""
            },
            name = escape(&name),
            key = key,
            cause = cause,
            shown = instant_text(fault.started_at_ms),
            class = if ongoing {
                " class=\"num down\""
            } else {
                " class=\"num\""
            },
            lasted = lasted,
            tail = if ongoing {
                " <span class=\"ongoing\">and counting</span>"
            } else {
                ""
            }
        );

        if fault.cause.is_some() {
            let _ = writeln!(
                output,
                "<tr role=\"row\" class=\"cause-row\" id=\"cause-{index}\" hidden>\
                 <td role=\"cell\" colspan=\"4\">{}</td></tr>",
                escape(message)
            );
        }
    }

    output.push_str("</tbody>\n</table>\n</div>");
    output
}

/// The two instants the chart covers, in UTC for the browser to localise.
fn window_span(window: Window) -> String {
    format!(
        "<p class=\"window\">{} → {}</p>\n",
        instant_text(window.from_ms),
        instant_text(window.to_ms)
    )
}

/// The SVG of the chart.
fn chart(timelines: &[TargetTimeline], up_since: &HashMap<String, i64>, window: Window) -> String {
    let mut output = String::new();

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
    let track_left = LABEL_WIDTH;
    let track_width = WIDTH - LABEL_WIDTH - STATS_WIDTH;
    for (index, timeline) in timelines.iter().enumerate() {
        let y = TOP_PAD + AXIS_HEIGHT + index as f64 * (ROW_HEIGHT + ROW_GAP);
        let name = timeline
            .label
            .clone()
            .unwrap_or_else(|| timeline.target.clone());
        let drops = window_drops(&timeline.segments, window.from_ms);

        let _ = write!(
            output,
            "<a href=\"/target?target={}\"><rect x=\"0\" y=\"{y:.1}\" width=\"{WIDTH}\" \
             height=\"{ROW_HEIGHT}\" fill=\"#000000\" fill-opacity=\"0\">\
             <title>Last 30 days for {}</title></rect>",
            encode(&timeline.target),
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

        // The bands butt together square and the lane is rounded as a whole, so a lane
        // reads as one bar instead of a row of lozenges: a clip path holds the two outer
        // ends, and the bands inside it stay square.
        let _ = write!(
            output,
            "<clipPath id=\"lane-{index}\"><rect x=\"{track_left:.1}\" y=\"{y:.1}\" \
             width=\"{track_width:.1}\" height=\"{ROW_HEIGHT}\" rx=\"{CAP_RADIUS}\"/></clipPath>\
             <g clip-path=\"url(#lane-{index})\">"
        );
        for band in bands(&timeline.segments, window) {
            let segment = band.segment;
            let _ = write!(
                output,
                "<rect x=\"{:.1}\" y=\"{y:.1}\" width=\"{:.1}\" height=\"{ROW_HEIGHT}\" \
                 fill=\"{}\"><title data-datetime=\"{}\" data-until=\"{}\" \
                 data-prefix=\"{} — \">{} — {} to {}</title></rect>",
                band.x,
                band.width,
                band.fill,
                format_rfc3339(segment.from_ms),
                format_rfc3339(segment.to_ms),
                state_label(segment.state),
                state_label(segment.state),
                utc_text(segment.from_ms),
                utc_text(segment.to_ms)
            );
        }
        output.push_str("</g>");

        let runs = runs_stats(&timeline.segments);
        let values = [
            (drops.to_string(), String::new()),
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

/// One band of a lane, placed on the track and ready to paint.
struct Band<'a> {
    segment: &'a TimelineSegment,
    x: f64,
    width: f64,
    fill: &'static str,
    /// Drawn wider than the time it covers, so it must be painted over its neighbours.
    exaggerated: bool,
}

/// The bands of one lane, in the order they are painted.
///
/// A fault too short to reach [`MIN_BAND_WIDTH`] is drawn that wide anyway, centred on the
/// instant it really covered, so it stays visible in a wide window; it is painted orange to
/// state that its width is not its duration. Such a band is painted last, because the
/// neighbour on its right would otherwise cover the width it gained.
fn bands<'a>(segments: &'a [TimelineSegment], window: Window) -> Vec<Band<'a>> {
    let track_left = LABEL_WIDTH;
    let track_right = WIDTH - STATS_WIDTH;

    let mut bands: Vec<Band<'a>> = segments
        .iter()
        .map(|segment| {
            let start = window.x(segment.from_ms);
            let true_width = (window.x(segment.to_ms) - start).max(0.0);
            // A brief fault and a brief degradation are both widened so they stay visible;
            // only the fault is painted orange, to say its width is not its duration.
            let exaggerated = matches!(segment.state, TargetState::Down | TargetState::Degraded)
                && true_width < MIN_BAND_WIDTH;
            let width = if exaggerated {
                MIN_BAND_WIDTH
            } else {
                true_width.max(0.4)
            };
            let x = if exaggerated {
                // Grown both ways from its true centre, kept inside the track so a fault at
                // either end of the window keeps its whole width.
                (start + true_width / 2.0 - width / 2.0).clamp(track_left, track_right - width)
            } else {
                start
            };
            Band {
                segment,
                x,
                width,
                fill: colour(segment.state, exaggerated),
                exaggerated,
            }
        })
        .collect();
    bands.sort_by_key(|band| band.exaggerated);
    bands
}

/// Disconnections of one lane over the shown window. A down segment that starts after
/// the window opened is a disconnection; a down segment that was already open when the
/// window opened is not, because it began before the period the number describes.
pub fn window_drops(segments: &[TimelineSegment], from_ms: i64) -> u32 {
    segments
        .iter()
        .filter(|segment| segment.state == TargetState::Down && segment.from_ms > from_ms)
        .count() as u32
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

/// A configured interval, exact and never truncated: `2m`, `45s`, `500ms`.
pub fn format_interval(ms: u64) -> String {
    if ms % 60_000 == 0 {
        format!("{}m", ms / 60_000)
    } else if ms % 1_000 == 0 {
        format!("{}s", ms / 1_000)
    } else {
        format!("{ms}ms")
    }
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
    let (date, time) = utc_halves(at_ms);
    format!("{date} {time}")
}

/// The two halves of an instant — the date and the time with its zone — which are the only
/// two pieces a page may keep apart: each one is unbreakable, and the line may fall between
/// them and nowhere else.
fn utc_halves(at_ms: i64) -> (String, String) {
    let stamp = format_rfc3339(at_ms);
    (stamp[0..10].to_string(), format!("{} UTC", &stamp[11..19]))
}

/// An instant as markup: the two halves in their own spans, so a narrow column stacks the
/// time under the date instead of splitting either of them.
fn instant_text(at_ms: i64) -> String {
    let (date, time) = utc_halves(at_ms);
    format!(
        "<time datetime=\"{}\"><span class=\"nowrap\">{date}</span> \
         <span class=\"nowrap\">{time}</span></time>",
        format_rfc3339(at_ms)
    )
}

fn short(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(limit.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
}

/// The fill of a band: green for reachable (`up` and the light-green `degraded`), red, or
/// grey. A fault widened to stay visible is orange, so a reader never mistakes a
/// minimum-width band for a long outage.
fn colour(state: TargetState, exaggerated: bool) -> &'static str {
    if exaggerated && state == TargetState::Down {
        return SHORT_FAULT_FILL;
    }
    match state {
        TargetState::Up => "#2e7d32",
        TargetState::Degraded => DEGRADED_FILL,
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
    use crate::web::templates::{script, stylesheet};

    /// No target is up in these pages unless a test says so.
    fn no_streaks() -> HashMap<String, i64> {
        HashMap::new()
    }
    use crate::history::TimelineSegment;

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
    fn a_lane_link_carries_the_encoded_key() {
        let mut timeline = timeline();
        timeline.target = "https://example.com/a?b=1".to_string();
        timeline.label = None;
        let rendered = chart(
            &[timeline],
            &no_streaks(),
            super::Window {
                from_ms: 0,
                to_ms: 3_600_000,
            },
        );

        assert!(
            rendered.contains("href=\"/target?target=https%3A%2F%2Fexample.com%2Fa%3Fb%3D1\""),
            "unexpected markup: {rendered}"
        );
    }

    #[test]
    fn the_chart_draws_one_segment_per_state() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = chart(&[timeline()], &no_streaks(), window);
        assert!(rendered.contains("#2e7d32"));
        assert!(rendered.contains("#c62828"));
        assert!(rendered.contains("cloudflare"));
    }

    /// A lane is one bar: rounded at its two outer ends and square in between, so two
    /// states never look like two separate objects.
    #[test]
    fn a_lane_is_rounded_at_its_ends_and_square_between_its_bands() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = chart(&[timeline()], &no_streaks(), window);

        // The radius lives in the clip path of the lane, not in its bands.
        assert_eq!(rendered.matches("rx=\"2\"").count(), 1);
        assert!(rendered.contains("<clipPath id=\"lane-0\">"));
        assert!(rendered.contains("clip-path=\"url(#lane-0)\""));

        // The bands cover the track end to end, each starting where the last one ended.
        let track_left = LABEL_WIDTH;
        let middle = (WIDTH - STATS_WIDTH + track_left) / 2.0;
        assert!(rendered.contains(&format!("<rect x=\"{track_left:.1}\"")));
        assert!(rendered.contains(&format!("<rect x=\"{middle:.1}\"")));
        for band in rendered.split("<rect ").skip(1) {
            let tag = band.split('>').next().unwrap_or_default();
            if tag.contains("fill=\"#2e7d32\"") || tag.contains("fill=\"#c62828\"") {
                assert!(!tag.contains("rx="), "a band carries no radius: {tag}");
            }
        }
    }

    /// A fault of a few seconds inside a day-wide window is a fraction of a pixel wide;
    /// it is drawn at the minimum width and painted orange to say its width is not its
    /// duration.
    #[test]
    fn a_fault_too_short_to_see_is_widened_and_painted_orange() {
        let timeline = TargetTimeline {
            target: "1.1.1.1".to_string(),
            label: None,
            segments: vec![
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 0,
                    to_ms: 43_199_000,
                },
                TimelineSegment {
                    state: TargetState::Down,
                    from_ms: 43_199_000,
                    to_ms: 43_200_000,
                },
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 43_200_000,
                    to_ms: 86_400_000,
                },
            ],
        };
        let window = Window {
            from_ms: 0,
            to_ms: 86_400_000,
        };
        let rendered = chart(&[timeline], &no_streaks(), window);

        let band = rendered
            .split("<rect ")
            .skip(1)
            .map(|band| band.split('>').next().unwrap_or_default())
            .find(|tag| tag.contains(SHORT_FAULT_FILL))
            .unwrap_or_else(|| panic!("the short fault is orange: {rendered}"));
        assert!(
            band.contains(&format!("width=\"{MIN_BAND_WIDTH:.1}\"")),
            "the short fault is widened to the minimum: {band}"
        );
        // Painted last, so the up band that follows it does not cover the width it gained.
        assert!(
            rendered.find(SHORT_FAULT_FILL) > rendered.rfind("#2e7d32"),
            "the widened band is painted over its neighbours"
        );
    }

    /// A brief degradation is widened like a fault, but stays light green: only a fault
    /// says its width is not its duration.
    #[test]
    fn a_short_degraded_stretch_is_widened_but_not_painted_orange() {
        let timeline = TargetTimeline {
            target: "1.1.1.1".to_string(),
            label: None,
            segments: vec![
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 0,
                    to_ms: 43_199_000,
                },
                TimelineSegment {
                    state: TargetState::Degraded,
                    from_ms: 43_199_000,
                    to_ms: 43_200_000,
                },
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 43_200_000,
                    to_ms: 86_400_000,
                },
            ],
        };
        let window = Window {
            from_ms: 0,
            to_ms: 86_400_000,
        };
        let rendered = chart(&[timeline], &no_streaks(), window);

        let band = rendered
            .split("<rect ")
            .skip(1)
            .map(|band| band.split('>').next().unwrap_or_default())
            .find(|tag| tag.contains(DEGRADED_FILL))
            .unwrap_or_else(|| panic!("the degraded band is light green: {rendered}"));
        assert!(
            band.contains(&format!("width=\"{MIN_BAND_WIDTH:.1}\"")),
            "the short degradation is widened to the minimum: {band}"
        );
        assert!(
            !rendered.contains(SHORT_FAULT_FILL),
            "a degradation is never painted as a short fault: {rendered}"
        );
    }

    /// The widened band stays inside the track, so a fault at the very end of the window
    /// keeps the whole minimum width instead of half of it.
    #[test]
    fn a_short_fault_at_the_end_of_the_window_keeps_its_width() {
        let timeline = TargetTimeline {
            target: "1.1.1.1".to_string(),
            label: None,
            segments: vec![
                TimelineSegment {
                    state: TargetState::Up,
                    from_ms: 0,
                    to_ms: 86_399_000,
                },
                TimelineSegment {
                    state: TargetState::Down,
                    from_ms: 86_399_000,
                    to_ms: 86_400_000,
                },
            ],
        };
        let window = Window {
            from_ms: 0,
            to_ms: 86_400_000,
        };
        let rendered = chart(&[timeline], &no_streaks(), window);

        let track_right = WIDTH - STATS_WIDTH;
        assert!(
            rendered.contains(&format!("<rect x=\"{:.1}\"", track_right - MIN_BAND_WIDTH)),
            "the last fault stops at the end of the track: {rendered}"
        );
    }

    #[test]
    fn the_lane_shows_its_drops_and_links_to_the_detail_page() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = chart(&[timeline()], &no_streaks(), window);
        assert!(rendered.contains("DROPS"));
        assert!(!rendered.contains("DOWN"), "the down-time column is gone");
        assert!(
            !rendered.contains("UP %"),
            "the availability column is gone"
        );
        assert!(rendered.contains("1h00"));
        assert!(rendered.contains("<a href=\"/target?target=1.1.1.1\">"));
    }

    #[test]
    fn the_last_faults_are_listed_with_their_duration() {
        let downs = [
            Fault {
                event_id: 2,
                target: "1.1.1.1".to_string(),
                label: None,
                interface: None,
                started_at_ms: 7_000_000,
                ended_at_ms: None,
                cause: None,
                cause_message: None,
            },
            Fault {
                event_id: 1,
                target: "8.8.8.8".to_string(),
                label: None,
                interface: None,
                started_at_ms: 3_600_000,
                ended_at_ms: Some(4_320_000),
                cause: None,
                cause_message: None,
            },
        ];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(rendered.contains("Last 10 faults"));
        assert!(rendered.contains("12m"), "720 s down");
        assert!(
            rendered.contains("and counting"),
            "still down at the window end"
        );
        assert!(rendered.contains("8.8.8.8"));
    }

    /// The list names a target the way its lane does: the label when there is one, the key
    /// otherwise, with the key always in the tooltip.
    #[test]
    fn a_disconnection_names_its_target_like_the_lane_above() {
        let downs = [
            Fault {
                event_id: 2,
                target: "tcp://1.1.1.1:443".to_string(),
                label: Some("cloudflare-https".to_string()),
                interface: None,
                started_at_ms: 7_000_000,
                ended_at_ms: None,
                cause: None,
                cause_message: None,
            },
            Fault {
                event_id: 1,
                target: "icmp://8.8.8.8".to_string(),
                label: None,
                interface: None,
                started_at_ms: 3_600_000,
                ended_at_ms: Some(4_320_000),
                cause: None,
                cause_message: None,
            },
        ];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);

        assert!(
            rendered.contains(
                "<td role=\"cell\"><span class=\"fault-name\">cloudflare-https</span>\
                 <span class=\"fault-key\">tcp://1.1.1.1:443</span></td>"
            ),
            "a labelled target states the key under its label"
        );
        assert!(
            rendered.contains(
                "<td role=\"cell\"><span class=\"fault-name\">icmp://8.8.8.8</span></td>"
            ),
            "an unlabelled target states the key alone, with nothing under it"
        );
    }

    #[test]
    fn a_fault_states_its_cause_and_opens_the_message_under_its_row() {
        let downs = [Fault {
            event_id: 1,
            target: "tcp://1.1.1.1:443".to_string(),
            label: Some("cloudflare-https".to_string()),
            interface: None,
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
            cause: Some("TCP.REFUSED".to_string()),
            cause_message: Some("the connection to 1.1.1.1:443 was refused".to_string()),
        }];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);

        assert!(
            rendered.contains("<th role=\"columnheader\" class=\"cause-head\">cause</th>"),
            "the cause closes the table"
        );
        let header = rendered
            .lines()
            .find(|line| line.contains("<thead role="))
            .unwrap_or_default();
        assert!(
            header.find("cause").unwrap_or(0) < header.find("lasted").unwrap_or(0),
            "the duration closes the table: {header}"
        );
        assert!(
            rendered.contains(
                "<button type=\"button\" class=\"cause\" aria-expanded=\"false\" \
                 aria-controls=\"cause-0\" \
                 title=\"the connection to 1.1.1.1:443 was refused\">TCP.REFUSED</button>"
            ),
            "the code is the control, the message the fallback: {rendered}"
        );
        assert!(
            rendered.contains(
                "<tr role=\"row\" class=\"cause-row\" id=\"cause-0\" hidden>\
                 <td role=\"cell\" colspan=\"4\">the connection to 1.1.1.1:443 was refused</td></tr>"
            ),
            "the message has a row of its own, closed until it is asked for"
        );
        assert!(
            rendered.contains("<tr role=\"row\" class=\"fault\">"),
            "the fault row is the one that toggles it"
        );
        assert!(
            script().contains("querySelectorAll('tr.fault')"),
            "the shared script opens it"
        );
    }

    #[test]
    fn a_table_announces_where_it_begins_and_keeps_its_header() {
        let style = stylesheet();
        assert!(
            style.contains("th {\n  position: sticky;"),
            "a long table keeps its header in place"
        );
        assert!(
            style.contains("border-top: 1px solid var(--rule);"),
            "a rule marks the first row of the header"
        );
        assert!(
            style.contains("table {\n  width: 100%;\n  margin: 22px 0 0;"),
            "room above the table separates it from what sits above it"
        );
        assert!(
            style.contains("background: Canvas;"),
            "the sticky header is painted with the page background"
        );
        assert!(
            !style.contains("border-collapse: separate"),
            "the rules stay the ones the theme already uses"
        );
    }

    /// A name, a key, a code, an instant half or a figure is one token: a narrow window or a
    /// long label must not split it, and the table widens and scrolls instead.
    #[test]
    fn the_tokens_of_a_cell_are_never_broken() {
        let style = stylesheet();
        assert!(
            style.contains(".nowrap { white-space: nowrap; }"),
            "the class the markup uses to pin a token"
        );
        for (selector, block) in [
            (".fault-name", ".fault-name {\n  display: block;"),
            (".fault-key", ".fault-key {\n  display: block;"),
            ("button.cause", "button.cause {\n  padding: 0;"),
        ] {
            let start = style
                .find(block)
                .unwrap_or_else(|| panic!("{selector} keeps its rule: {block}"));
            let rule = &style[start..];
            let rule = &rule[..rule.find('}').expect("the rule closes")];
            assert!(
                rule.contains("white-space: nowrap;"),
                "{selector} never breaks its token: {rule}"
            );
        }
        assert!(
            !style.contains("table-layout: fixed"),
            "a fixed layout clips the token it cannot fit"
        );
        assert!(
            style.contains(".table-scroll { overflow-x: auto; }"),
            "a table too wide for the window scrolls itself"
        );
    }

    #[test]
    fn a_fault_without_a_cause_opens_nothing() {
        let downs = [Fault {
            event_id: 1,
            target: "1.1.1.1".to_string(),
            label: None,
            interface: None,
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
            cause: None,
            cause_message: None,
        }];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);

        assert!(
            rendered.contains("<td role=\"cell\" class=\"cause-cell\"></td>"),
            "an empty cell"
        );
        assert!(
            !rendered.contains("<tr role=\"row\" class=\"cause-row\""),
            "a row written before causes has no message to open"
        );
        assert!(!rendered.contains("<tr class=\"fault\">"));
    }

    #[test]
    fn the_duration_column_uses_the_style_shared_with_the_detail_page() {
        let downs = [Fault {
            event_id: 1,
            target: "8.8.8.8".to_string(),
            label: None,
            interface: None,
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
            cause: None,
            cause_message: None,
        }];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(
            rendered.contains("<th role=\"columnheader\" class=\"num lasted-head\">lasted</th>")
        );
        assert!(rendered.contains("<td role=\"cell\" class=\"num\">12m</td>"));
        let style = stylesheet();
        assert!(
            !style.contains(".numeric"),
            "one table style for both pages"
        );
        assert!(style.contains(".num { text-align: right; }"));
    }

    #[test]
    fn an_ongoing_disconnection_keeps_both_the_numeric_and_the_down_style() {
        let downs = [Fault {
            event_id: 1,
            target: "1.1.1.1".to_string(),
            label: None,
            interface: None,
            started_at_ms: 7_000_000,
            ended_at_ms: None,
            cause: None,
            cause_message: None,
        }];
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(rendered.contains("<td role=\"cell\" class=\"num down\">"));
    }

    #[test]
    fn a_quiet_history_says_so() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains("No fault recorded yet."));
    }

    #[test]
    fn the_page_reloads_itself_and_loads_the_shared_script() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.starts_with("<!doctype html>"));
        assert!(
            rendered.contains("<noscript><meta http-equiv=\"refresh\" content=\"30\"></noscript>"),
            "the 30 s reload is the fallback of a browser without the script"
        );
        assert!(
            rendered.contains("<body data-refresh=\"30\">"),
            "with the script the interval is the one the script drives"
        );
        assert!(
            rendered.contains("<script src=\"/app.js\"></script>"),
            "the script is a file the browser can debug, not text in the page"
        );
        assert!(
            !rendered.contains("<script>\n"),
            "no script written into the page"
        );
        let script = script();
        assert!(!script.contains("wheel"), "no zoom");
        assert!(!script.contains("pointerdown"), "no pan");
        assert!(!script.contains("fetch("));
    }

    #[test]
    fn a_lane_can_be_opened() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(script().contains("closest('a')"));
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
    fn statistics_count_only_faults_that_start_inside_the_window() {
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

        let open_at_window_start = window_drops(&segments, 0);
        assert_eq!(open_at_window_start, 1);

        // The segment that began before the window opened stops counting as a drop.
        let late_window = window_drops(&segments, 2_000);
        assert_eq!(late_window, 0);
    }

    #[test]
    fn a_lane_states_its_average_up_run() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(rendered.contains("AVG UP"), "the average column");
        assert!(rendered.contains("MAX"), "the longest-run column");
        let order = ["DROPS", "AVG UP", "MAX", "UP FOR"].map(|label| {
            rendered
                .find(label)
                .unwrap_or_else(|| panic!("{label} is a column"))
        });
        assert!(
            order.windows(2).all(|pair| pair[0] < pair[1]),
            "the columns read left to right as requested"
        );
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
            "<time datetime=\"1970-01-01T00:00:00.000Z\"><span class=\"nowrap\">1970-01-01</span> \
             <span class=\"nowrap\">00:00:00 UTC</span></time>"
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
        let downs = [Fault {
            event_id: 1,
            target: "8.8.8.8".to_string(),
            label: None,
            interface: None,
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
            cause: None,
            cause_message: None,
        }];
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(DEFAULT_WINDOW_HOURS),
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        assert!(rendered.contains(
            "<td role=\"cell\"><time datetime=\"1970-01-01T01:00:00.000Z\"><span class=\"nowrap\">1970-01-01</span> \
             <span class=\"nowrap\">01:00:00 UTC</span></time></td>"
        ));
        assert!(rendered.contains("<script src=\"/app.js\"></script>"));
        assert!(
            script().contains("Intl.DateTimeFormat"),
            "the shared script localises the instants"
        );
    }

    /// The date and the time are the only two pieces a narrow column may keep apart: each
    /// half is its own span, so the time stacks under the date and neither half splits.
    #[test]
    fn an_instant_may_break_between_its_date_and_its_time() {
        let downs = [Fault {
            event_id: 1,
            target: "8.8.8.8".to_string(),
            label: None,
            interface: None,
            started_at_ms: 3_600_000,
            ended_at_ms: Some(4_320_000),
            cause: None,
            cause_message: None,
        }];
        let window = Window {
            from_ms: 0,
            to_ms: hours_span(DEFAULT_WINDOW_HOURS),
        };
        let rendered = page(&[timeline()], &downs, &no_streaks(), window);
        let cell = rendered
            .split("<td role=\"cell\"><time datetime=\"1970-01-01T01:00:00.000Z\">")
            .nth(1)
            .expect("the started column states the instant as markup");
        assert!(
            cell.starts_with(
                "<span class=\"nowrap\">1970-01-01</span> \
                 <span class=\"nowrap\">01:00:00 UTC</span></time>"
            ),
            "two unbreakable halves, side by side: {cell}"
        );
        assert!(
            !rendered.contains("1970-01-01T01:00:00.000Z\">1970-01-01 01:00:00 UTC"),
            "one loose string would let the browser break the date from the time anywhere"
        );
    }

    #[test]
    fn the_timeline_offers_no_print_control() {
        let window = Window {
            from_ms: 0,
            to_ms: 7_200_000,
        };
        let rendered = page(&[timeline()], &[], &no_streaks(), window);
        assert!(!rendered.contains("print-button"));
        assert!(!rendered.contains("print-head"));
        assert!(
            !rendered.contains("target-page"),
            "the print sheet names only the target page"
        );
    }

    #[test]
    fn spans_are_formatted_compactly() {
        assert_eq!(format_span(0), "0s");
        assert_eq!(format_span(45_000), "45s");
        assert_eq!(format_span(12 * 60_000), "12m");
        assert_eq!(format_span(3 * 3_600_000 + 7 * 60_000), "3h07");
        assert_eq!(format_span(2 * 86_400_000 + 4 * 3_600_000), "2d04h");
    }

    #[test]
    fn intervals_are_formatted_exactly() {
        assert_eq!(format_interval(500), "500ms");
        assert_eq!(format_interval(1_500), "1500ms");
        assert_eq!(format_interval(5_000), "5s");
        assert_eq!(format_interval(60_000), "1m");
        assert_eq!(format_interval(90_000), "90s");
        assert_eq!(format_interval(120_000), "2m");
    }
}
