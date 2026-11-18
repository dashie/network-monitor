//! `GET /api/v1/faults/daily`: faults, down time, and `up` runs per UTC day.

use std::collections::BTreeMap;
use std::net::TcpStream;

use crate::history::{DailyDrops, HistorySource};
use crate::json;
use crate::time::{format_rfc3339, now_ms};
use crate::web::{stats, DAY_MS};

use super::{
    close, open, reject, resolve, truncated, Envelope, Meta, Params, Rejection, Reply, StreamError,
    DEFAULT_DAYS, DOMAIN_MAX_MS, DOMAIN_MIN_MS,
};

/// Faults and down time per UTC day, oldest first, with the `up` runs of each day.
pub(super) fn route(stream: &mut TcpStream, query: &str, source: &dyn HistorySource, reply: Reply) {
    let now = now_ms();
    let params = match Params::parse(
        query,
        &["target", "from", "to", "limit", "offset", "tz"],
        now,
    ) {
        Ok(params) => params,
        Err(rejection) => return reject(stream, rejection, reply),
    };
    let now = now.clamp(DOMAIN_MIN_MS, DOMAIN_MAX_MS);
    let to = params.to.unwrap_or(now);
    // The default window stops at the oldest accepted instant instead of running off the end of
    // the domain, so a `to` at the lower bound states an empty window rather than an underflow.
    let from = params
        .from
        .unwrap_or(to.saturating_sub(DEFAULT_DAYS * DAY_MS).max(DOMAIN_MIN_MS));
    if from >= to {
        return reject(stream, Rejection::range(), reply);
    }

    let Some(store) = open(stream, source, reply) else {
        return;
    };
    let target = match resolve(store.as_ref(), params.target.as_deref()) {
        Ok(target) => target,
        Err(rejection) => return reject(stream, rejection, reply),
    };

    // Only the requested page of days is read: the window may be any length, but the work
    // and the memory of one answer follow `limit`, not the span it covers.
    let page = Page::of(from, to, params.offset, params.limit).unwrap_or(Page {
        from_ms: to,
        to_ms: to,
        total: 0,
        take: 0,
    });

    // The same rule the detail page applies: a run that crosses midnight is split, and the
    // current UTC day states the runs of the last 24 hours.
    let mut runs_by_day: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    let mut rolling: Vec<i64> = Vec::new();
    let mut days = Vec::new();
    if page.take > 0 {
        days = match store.daily_drops(target.as_deref(), page.from_ms, page.to_ms, now) {
            Ok(days) => days,
            Err(error) => return reject(stream, Rejection::failed(&error), reply),
        };
        // The statistics stop at the present: a day that has not begun has no `up` run, and
        // reading to the end of a window that reaches into the future would invent one.
        let timelines = match store.timeline(page.timeline_from_ms(now), page.to_ms.min(now)) {
            Ok(timelines) => timelines,
            Err(error) => return reject(stream, Rejection::failed(&error), reply),
        };
        for timeline in &timelines {
            if let Some(only) = target.as_deref() {
                if timeline.target != only {
                    continue;
                }
            }
            for (day, runs) in stats::up_runs_per_day(&timeline.segments, DAY_MS) {
                runs_by_day.entry(day).or_default().extend(runs);
            }
            rolling.extend(stats::up_runs_between(
                &timeline.segments,
                now - DAY_MS,
                now,
            ));
        }
    }

    let current_day = now.div_euclid(DAY_MS) * DAY_MS;
    let meta = Meta {
        limit: params.limit,
        offset: params.offset,
        total: page.total,
        target: target.as_deref(),
        from: Some(from),
        to: Some(to),
        generated_at: now,
        step: None,
        tz: params.tz.as_deref(),
    };

    let mut envelope = Envelope::begin(stream, reply);
    for day in &days {
        let partial = day.day_start_ms == current_day;
        let pieces: &[i64] = if partial {
            &rolling
        } else {
            runs_by_day
                .get(&day.day_start_ms)
                .map_or(&[][..], Vec::as_slice)
        };
        let summary = stats::summarize(pieces);
        let record = daily_record(
            day,
            target.as_deref(),
            (summary.count > 0).then_some(summary.mean_ms),
            pieces.iter().copied().max(),
            partial,
        );
        if let Err(error) = envelope.record(&record) {
            return truncated(StreamError::Write(error));
        }
    }
    if let Err(error) = envelope.finish(&meta) {
        return truncated(StreamError::Write(error));
    }
    close(stream, reply);
}

/// The days of the window, and the slice of them one request answers.
///
/// One rule decides every boundary, and every read is clipped by it:
///
/// 1. The days of a window are the UTC days that contain `[from, to)`: the first contains
///    `from`, the last contains `to - 1 ms`, and there are `day_count(floor(from), to)` of them.
/// 2. `offset` and `limit` select the contiguous slice `[skip, skip + take)` of that list.
/// 3. Read `[max(day_start(skip), from), min(day_start(skip + take), to))`, so a day at either
///    edge counts only the part that falls inside the window.
/// 4. The timeline starts at `min(read from, now - 1 day)` when the page reaches the
///    present, so the UTC day still running states the last 24 hours, and at `read from`
///    otherwise: a page wholly in the future holds no running day, and reaching back to
///    `now` would spread the timeline over the whole calendar.
/// 5. A page past the end reads nothing and still states the day count of the whole window.
struct Page {
    /// Start of the first day of the page.
    from_ms: i64,
    /// End of the last day of the page, clipped to the window.
    to_ms: i64,
    /// Days in the whole window, whatever the page holds.
    total: i64,
    /// Days this page holds; zero for an `offset` past the end, which still states `total`.
    take: i64,
}

impl Page {
    /// `None` when the window holds no day at all.
    fn of(window_from_ms: i64, window_to_ms: i64, offset: u32, limit: u32) -> Option<Page> {
        let first_day = window_from_ms.div_euclid(DAY_MS).saturating_mul(DAY_MS);
        let total = day_count(first_day, window_to_ms);
        if total <= 0 {
            return None;
        }

        let skip = i64::from(offset).min(total);
        let take = (total - skip).min(i64::from(limit));

        let page_start = first_day.saturating_add(skip.saturating_mul(DAY_MS));
        let page_end = first_day.saturating_add(skip.saturating_add(take).saturating_mul(DAY_MS));
        Some(Page {
            from_ms: page_start.max(window_from_ms),
            to_ms: page_end.min(window_to_ms),
            total,
            take,
        })
    }

    /// Where the timeline must start: the page, or the last 24 hours when the page holds
    /// the UTC day still running, whose average covers that rolling window.
    fn timeline_from_ms(&self, now_ms: i64) -> i64 {
        if self.from_ms > now_ms {
            return self.from_ms;
        }
        self.from_ms.min(now_ms.saturating_sub(DAY_MS))
    }
}

/// How many UTC days `[first_day, to_ms)` covers, the last one possibly partial.
fn day_count(first_day: i64, to_ms: i64) -> i64 {
    if to_ms <= first_day {
        return 0;
    }
    let span = i128::from(to_ms) - i128::from(first_day);
    let days = (span + i128::from(DAY_MS) - 1) / i128::from(DAY_MS);
    i64::try_from(days).unwrap_or(i64::MAX)
}

fn daily_record(
    day: &DailyDrops,
    target: Option<&str>,
    average_up_run_ms: Option<i64>,
    longest_up_run_ms: Option<i64>,
    partial: bool,
) -> String {
    let start = format_rfc3339(day.day_start_ms);
    format!(
        "  {{\"target\": {}, \"day\": {}, \"dayStart\": {}, \"dayStartMs\": {}, \
         \"faults\": {}, \"downMs\": {}, \"avgUpRunMs\": {}, \"longestUpRunMs\": {}, \
         \"partial\": {}}}",
        json::option_string(&target.map(str::to_string)),
        json::string(&start[..10]),
        json::string(&start),
        day.day_start_ms,
        day.drops,
        day.down_ms,
        json::option_i64(average_up_run_ms),
        json::option_i64(longest_up_run_ms),
        partial,
    )
}
