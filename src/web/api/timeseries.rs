//! `GET /api/v1/timeseries`: one target's state on a regular grid of instants.
//!
//! The history stores transitions, not observations. This route projects the timeline onto a
//! grid of `step` seconds: at every instant it states whether the target was `up`, `down`, or
//! `unknown` at that moment. Nothing is stored and nothing is invented after `now`.

use std::net::TcpStream;

use crate::history::{HistorySource, TargetState, TimelineSegment};
use crate::json;
use crate::time::now_ms;

use super::{
    close, open, reject, resolve, truncated, Envelope, Lines, Meta, Params, Rejection, Reply,
    StreamError, DOMAIN_MIN_MS,
};

/// The window the route covers when the request names no `from`.
const DEFAULT_WINDOW_MS: i64 = 3_600_000;
/// How many samples a page holds when the request states no `limit`.
const DEFAULT_LIMIT: u32 = 1_000;
/// The largest page this route accepts; the answer is streamed one record per chunk.
const MAX_LIMIT: u32 = 100_000;

/// Everything both renderings of the route answer: the window, the page, and its samples.
struct Plan {
    params: Params,
    target: String,
    now: i64,
    from: i64,
    to: i64,
    total: i64,
    samples: Vec<(i64, TargetState)>,
}

impl Plan {
    fn meta(&self) -> Meta<'_> {
        Meta {
            limit: self.params.limit,
            offset: self.params.offset,
            total: self.total,
            target: Some(&self.target),
            from: Some(self.from),
            to: Some(self.to),
            generated_at: self.now,
            step: Some(self.params.step),
            tz: self.params.tz.as_deref(),
        }
    }

    fn sample(&self, at: i64, state: TargetState) -> String {
        let mut record = format!("{{\"t\": {}, \"v\": {}", at, json::string(state.as_str()));
        if self.params.ts {
            record.push_str(&format!(
                ", \"ts\": {}",
                json::string(&self.params.zone.format(at))
            ));
        }
        record.push('}');
        record
    }
}

/// Resolve the request into the page both routes write, answering a refusal itself.
fn plan(
    stream: &mut TcpStream,
    query: &str,
    source: &dyn HistorySource,
    reply: Reply,
) -> Option<Plan> {
    let now = now_ms();
    let params = match Params::parse_with(
        query,
        &[
            "target", "from", "to", "step", "limit", "offset", "tz", "ts",
        ],
        now,
        DEFAULT_LIMIT,
        MAX_LIMIT,
    ) {
        Ok(params) => params,
        Err(rejection) => {
            reject(stream, rejection, reply);
            return None;
        }
    };

    let Some(needle) = params.target.clone().filter(|target| !target.is_empty()) else {
        reject(stream, Rejection::missing_required("target"), reply);
        return None;
    };
    let store = open(stream, source, reply)?;
    let target = match resolve(store.as_ref(), Some(&needle)) {
        Ok(Some(target)) => target,
        Ok(None) => unreachable!("a non-empty needle resolves or is refused"),
        Err(rejection) => {
            reject(stream, rejection, reply);
            return None;
        }
    };

    let to = params.to.unwrap_or(now);
    let from = params
        .from
        .unwrap_or(to.saturating_sub(DEFAULT_WINDOW_MS).max(DOMAIN_MIN_MS));
    if from >= to {
        reject(stream, Rejection::range(), reply);
        return None;
    }

    let step_ms = i64::from(params.step) * 1_000;
    let total = sample_count(from, to, step_ms);
    let (skip, take) = page(total, params.offset, params.limit);
    let first = first_sample(from, step_ms);

    let samples = if take > 0 {
        let page_first = first + skip * step_ms;
        let page_last = page_first + (take - 1) * step_ms;
        // Only the timeline the page needs is read: the cost follows `limit`, not the window.
        match store.timeline_of(&target, page_first, page_last + 1) {
            Ok(timeline) => {
                sample_segments(&timeline.segments, page_first, step_ms, take as usize, now)
            }
            Err(error) => {
                reject(stream, Rejection::failed(&error), reply);
                return None;
            }
        }
    } else {
        Vec::new()
    };

    Some(Plan {
        params,
        target,
        now,
        from,
        to,
        total,
        samples,
    })
}

/// The samples of one page as the usual `data`/`meta` JSON object.
pub(super) fn route(stream: &mut TcpStream, query: &str, source: &dyn HistorySource, reply: Reply) {
    let Some(plan) = plan(stream, query, source, reply) else {
        return;
    };

    let mut envelope = Envelope::begin(stream, reply);
    for (at, state) in &plan.samples {
        let record = format!("  {}", plan.sample(*at, *state));
        if let Err(error) = envelope.record(&record) {
            return truncated(StreamError::Write(error));
        }
    }
    if let Err(error) = envelope.finish(&plan.meta()) {
        return truncated(StreamError::Write(error));
    }
    close(stream, reply);
}

/// The same page as JSON Lines: the meta object first, then one sample per line.
pub(super) fn stream_route(
    stream: &mut TcpStream,
    query: &str,
    source: &dyn HistorySource,
    reply: Reply,
) {
    let Some(plan) = plan(stream, query, source, reply) else {
        return;
    };

    let mut lines = Lines::begin(stream, reply);
    if let Err(error) = lines.line(&plan.meta().line()) {
        return truncated(StreamError::Write(error));
    }
    for (at, state) in &plan.samples {
        if let Err(error) = lines.line(&plan.sample(*at, *state)) {
            return truncated(StreamError::Write(error));
        }
    }
    if let Err(error) = lines.finish() {
        return truncated(StreamError::Write(error));
    }
    close(stream, reply);
}

/// The smallest instant of the grid that is not earlier than `from_ms`.
pub(super) fn first_sample(from_ms: i64, step_ms: i64) -> i64 {
    let remainder = from_ms.rem_euclid(step_ms);
    if remainder == 0 {
        from_ms
    } else {
        from_ms + (step_ms - remainder)
    }
}

/// How many grid instants lie in `[from_ms, to_ms)`.
pub(super) fn sample_count(from_ms: i64, to_ms: i64, step_ms: i64) -> i64 {
    let first = first_sample(from_ms, step_ms);
    if first >= to_ms {
        0
    } else {
        let span = to_ms - first;
        (span + step_ms - 1) / step_ms
    }
}

/// The contiguous slice of samples one request answers.
pub(super) fn page(total: i64, offset: u32, limit: u32) -> (i64, i64) {
    let skip = i64::from(offset).min(total);
    let take = (total - skip).min(i64::from(limit));
    (skip, take)
}

/// Project `count` grid instants, starting at `first_ms`, onto the segments.
///
/// A sample states the segment that contains its instant; an instant after `now` is `unknown`
/// because unelapsed time is not measured.
pub(super) fn sample_segments(
    segments: &[TimelineSegment],
    first_ms: i64,
    step_ms: i64,
    count: usize,
    now_ms: i64,
) -> Vec<(i64, TargetState)> {
    let mut samples = Vec::with_capacity(count);
    let mut index = 0;
    for position in 0..count {
        let at = first_ms + position as i64 * step_ms;
        while index < segments.len() && segments[index].to_ms <= at {
            index += 1;
        }
        let state = if at > now_ms {
            TargetState::Unknown
        } else {
            match segments.get(index) {
                Some(segment) if segment.from_ms <= at && at < segment.to_ms => segment.state,
                _ => TargetState::Unknown,
            }
        };
        samples.push((at, state));
    }
    samples
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: i64 = 5_000;

    fn segment(state: TargetState, from_ms: i64, to_ms: i64) -> TimelineSegment {
        TimelineSegment {
            state,
            from_ms,
            to_ms,
        }
    }

    #[test]
    fn the_grid_starts_at_the_first_multiple_not_earlier_than_from() {
        assert_eq!(first_sample(0, STEP), 0);
        assert_eq!(first_sample(1, STEP), STEP);
        assert_eq!(first_sample(5_000, STEP), 5_000);
        assert_eq!(first_sample(5_001, STEP), 10_000);
    }

    #[test]
    fn the_count_uses_a_half_open_window() {
        assert_eq!(sample_count(0, 0, STEP), 0);
        assert_eq!(sample_count(0, 5_000, STEP), 1);
        assert_eq!(sample_count(0, 5_001, STEP), 2);
        assert_eq!(sample_count(1, 10_000, STEP), 1);
    }

    #[test]
    fn paging_is_clamped_to_the_total() {
        assert_eq!(page(10, 0, 4), (0, 4));
        assert_eq!(page(10, 8, 4), (8, 2));
        assert_eq!(page(10, 10, 4), (10, 0));
        assert_eq!(page(10, 99, 4), (10, 0));
    }

    #[test]
    fn a_sample_states_the_segment_that_contains_it() {
        let segments = [
            segment(TargetState::Unknown, 0, 10_000),
            segment(TargetState::Up, 10_000, 20_000),
            segment(TargetState::Down, 20_000, 30_000),
        ];
        let samples = sample_segments(&segments, 0, STEP, 6, 30_000);
        assert_eq!(
            samples,
            vec![
                (0, TargetState::Unknown),
                (5_000, TargetState::Unknown),
                (10_000, TargetState::Up),
                (15_000, TargetState::Up),
                (20_000, TargetState::Down),
                (25_000, TargetState::Down),
            ]
        );
    }

    #[test]
    fn an_instant_after_now_is_unknown() {
        let segments = [segment(TargetState::Up, 0, 60_000)];
        let samples = sample_segments(&segments, 0, STEP, 4, 7_500);
        assert_eq!(
            samples,
            vec![
                (0, TargetState::Up),
                (5_000, TargetState::Up),
                (10_000, TargetState::Unknown),
                (15_000, TargetState::Unknown),
            ]
        );
    }

    #[test]
    fn an_empty_timeline_answers_unknown() {
        let samples = sample_segments(&[], 0, STEP, 2, STEP);
        assert_eq!(
            samples,
            vec![(0, TargetState::Unknown), (STEP, TargetState::Unknown)]
        );
    }
}
