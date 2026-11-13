//! How long a target stays `up` before it drops.
//!
//! The page draws the same segments that feed these numbers, so a reader can check
//! every figure against the bands above it. A run that the window opens or closes in
//! the middle of is counted as it appears: the first and the last run of a window are
//! therefore lower bounds, never overestimates.

use std::collections::BTreeMap;

use crate::store::{TargetState, TimelineSegment};

/// Every continuous `up` stretch of the segments, as `(from_ms, to_ms)`.
///
/// Adjacent `up` segments are merged: a state that never changed is one run even when
/// the database holds several transition rows for it.
pub fn up_ranges(segments: &[TimelineSegment]) -> Vec<(i64, i64)> {
    let mut ranges = Vec::new();
    let mut open: Option<i64> = None;
    for segment in segments {
        if segment.state == TargetState::Up {
            open.get_or_insert(segment.from_ms);
        } else if let Some(from_ms) = open.take() {
            ranges.push((from_ms, segment.from_ms));
        }
    }
    if let (Some(from_ms), Some(last)) = (open, segments.last()) {
        ranges.push((from_ms, last.to_ms));
    }
    ranges.retain(|(from_ms, to_ms)| to_ms > from_ms);
    ranges
}

/// The length of every `up` run, in milliseconds.
pub fn up_runs(segments: &[TimelineSegment]) -> Vec<i64> {
    up_ranges(segments)
        .into_iter()
        .map(|(from_ms, to_ms)| to_ms - from_ms)
        .collect()
}

/// The longest `up` run of the segments, if any.
///
/// The mean of a window that holds one long run and a storm of short ones sits far
/// below both, so the longest run travels with it.
pub fn longest_run(segments: &[TimelineSegment]) -> Option<i64> {
    up_runs(segments).into_iter().max()
}

/// The `up` runs clipped to `[from_ms, to_ms]`, in milliseconds.
///
/// A run that the window opens or closes in the middle of counts as it appears, like
/// every other figure of the page: the edge runs are lower bounds, never overestimates.
pub fn up_runs_between(segments: &[TimelineSegment], from_ms: i64, to_ms: i64) -> Vec<i64> {
    up_ranges(segments)
        .into_iter()
        .filter_map(|(start, end)| {
            let start = start.max(from_ms);
            let end = end.min(to_ms);
            (end > start).then_some(end - start)
        })
        .collect()
}

/// The `up` runs split at UTC day boundaries: one entry per day, oldest first, with the
/// length of every piece of a run that falls inside that day.
pub fn up_runs_per_day(segments: &[TimelineSegment], day_ms: i64) -> BTreeMap<i64, Vec<i64>> {
    let mut days: BTreeMap<i64, Vec<i64>> = BTreeMap::new();
    for (from_ms, to_ms) in up_ranges(segments) {
        let mut cursor = from_ms;
        while cursor < to_ms {
            let day = cursor.div_euclid(day_ms) * day_ms;
            let end = (day + day_ms).min(to_ms);
            days.entry(day).or_default().push(end - cursor);
            cursor = end;
        }
    }
    days
}

/// Mean and population standard deviation of a sample, in whole milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RunStats {
    pub count: u32,
    pub mean_ms: i64,
    pub sigma_ms: i64,
}

/// Summarise the runs of a window. An empty sample has a count of zero and no mean.
pub fn summarize(runs: &[i64]) -> RunStats {
    if runs.is_empty() {
        return RunStats::default();
    }
    let count = runs.len() as u32;
    let total: i64 = runs.iter().sum();
    let mean_ms = total / i64::from(count);
    let spread = runs
        .iter()
        .map(|run| {
            let deviation = (*run - mean_ms) as f64;
            deviation * deviation
        })
        .sum::<f64>()
        / f64::from(count);
    RunStats {
        count,
        mean_ms,
        sigma_ms: spread.sqrt().round() as i64,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(state: TargetState, from_s: i64, to_s: i64) -> TimelineSegment {
        TimelineSegment {
            state,
            from_ms: from_s * 1_000,
            to_ms: to_s * 1_000,
        }
    }

    #[test]
    fn adjacent_up_segments_are_one_run() {
        let segments = [
            segment(TargetState::Up, 0, 10),
            segment(TargetState::Up, 10, 20),
            segment(TargetState::Down, 20, 30),
            segment(TargetState::Up, 30, 40),
        ];
        assert_eq!(up_runs(&segments), vec![20_000, 10_000]);
    }

    #[test]
    fn a_window_that_opens_and_closes_on_up_counts_both_edges() {
        let segments = [
            segment(TargetState::Up, 0, 10),
            segment(TargetState::Down, 10, 20),
            segment(TargetState::Up, 20, 30),
        ];
        assert_eq!(
            up_runs(&segments),
            vec![10_000, 10_000],
            "the first and the last run are clipped by the window, not dropped"
        );
    }

    #[test]
    fn a_target_down_for_the_whole_window_has_no_run() {
        let segments = [segment(TargetState::Down, 0, 30)];
        assert!(up_runs(&segments).is_empty());
        assert_eq!(summarize(&[]), RunStats::default());
    }

    #[test]
    fn the_mean_and_the_spread_are_those_of_the_sample() {
        let stats = summarize(&[1_000, 1_000, 1_000, 1_000, 10_000, 10_000, 10_000]);
        assert_eq!(stats.count, 7);
        assert_eq!(stats.mean_ms, 4_857, "34 s over 7 runs");
        assert_eq!(
            stats.sigma_ms, 4_454,
            "the mean is truncated to whole milliseconds"
        );
    }

    #[test]
    fn a_single_run_has_no_spread() {
        let stats = summarize(&[7_200_000]);
        assert_eq!(stats.count, 1);
        assert_eq!(stats.mean_ms, 7_200_000);
        assert_eq!(stats.sigma_ms, 0);
    }

    #[test]
    fn the_longest_run_is_the_widest_up_stretch() {
        let segments = [
            segment(TargetState::Up, 0, 10),
            segment(TargetState::Down, 10, 20),
            segment(TargetState::Up, 20, 45),
            segment(TargetState::Down, 45, 50),
        ];
        assert_eq!(longest_run(&segments), Some(25_000));
        assert_eq!(
            longest_run(&[segment(TargetState::Down, 0, 10)]),
            None,
            "no up run, no longest run"
        );
    }

    #[test]
    fn a_run_is_clipped_to_the_window_it_is_asked_about() {
        let segments = [
            segment(TargetState::Up, 0, 10),
            segment(TargetState::Down, 10, 20),
            segment(TargetState::Up, 20, 30),
        ];
        assert_eq!(
            up_runs_between(&segments, 5_000, 25_000),
            vec![5_000, 5_000]
        );
        assert_eq!(
            up_runs_between(&segments, 30_000, 40_000),
            Vec::<i64>::new(),
            "a window that holds no `up` time has no run"
        );
    }

    #[test]
    fn runs_are_split_at_the_day_boundary() {
        let day = 10_000;
        let segments = [TimelineSegment {
            state: TargetState::Up,
            from_ms: 5_000,
            to_ms: 25_000,
        }];
        let days = up_runs_per_day(&segments, day);
        assert_eq!(days.get(&0), Some(&vec![5_000]));
        assert_eq!(days.get(&10_000), Some(&vec![10_000]));
        assert_eq!(days.get(&20_000), Some(&vec![5_000]));
    }
}
