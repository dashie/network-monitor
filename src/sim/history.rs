//! The read contract over the generator.
//!
//! [`SimulatedSource`] is the simulated sibling of `store::DatabaseSource`: the page, the API
//! and the export ask it the same questions and get the same shapes back, but nothing is read
//! from and nothing is written to a database. Each request regenerates the days it needs from
//! `(seed, target key, UTC day)` and reduces them with the same helpers the SQL path uses, so
//! the two implementations cannot answer the same question differently (ADR-0032).
//!
//! Two rules make the regeneration cheap enough and faithful enough:
//!
//! - a day is generated together with the day before it, so the state at any instant and the
//!   end of a fault that opened earlier are known without replaying the whole history;
//! - nothing exists before `now - history_days` and nothing after `now`, so the simulated past
//!   has the same shape as a database that was started that many days ago.

use crate::config::Target;
use crate::history::{
    DailyDrops, EventFilter, EventRow, ExportedEvent, Fault, FaultFilter, History, HistoryError,
    HistorySource, TargetState, TargetStateRow, TargetTimeline, TimelineSegment,
};
use crate::probe::{simulated_day, SimulatedEpisode, SimulatedState};
use crate::store::{daily_drops_from_spans, Event};
use crate::time::now_ms;

use super::{life, outages, raw_outages, Life, Outage, DAY_MS};

/// A board of simulated targets: the configuration, the seed and how far back the past goes.
#[derive(Debug, Clone)]
pub struct SimulatedSource {
    targets: Vec<Target>,
    seed: u64,
    history_days: i64,
    fault_rate: f64,
}

impl SimulatedSource {
    pub fn new(targets: Vec<Target>, seed: u64, history_days: i64, fault_rate: f64) -> Self {
        SimulatedSource {
            targets,
            seed,
            history_days,
            fault_rate,
        }
    }

    /// The targets the board was built from, in configuration order.
    pub fn targets(&self) -> &[Target] {
        &self.targets
    }
}

impl HistorySource for SimulatedSource {
    fn open(&self) -> Result<Box<dyn History>, HistoryError> {
        Ok(Box::new(SimulatedHistory {
            targets: self.targets.clone(),
            seed: self.seed,
            history_days: self.history_days,
            fault_rate: self.fault_rate,
            now_ms: now_ms(),
        }))
    }

    fn describe(&self) -> String {
        format!(
            "a simulated history of {} day(s) of {} target(s), seed {}, {} fault(s) per day",
            self.history_days,
            self.targets.len(),
            self.seed,
            self.fault_rate
        )
    }
}

/// One request's view of the simulated board.
#[derive(Debug, Clone)]
pub struct SimulatedHistory {
    targets: Vec<Target>,
    seed: u64,
    history_days: i64,
    /// The faults a day of a target that fails now and then holds.
    fault_rate: f64,
    /// The instant the request was opened at; the simulated history never reaches past it.
    now_ms: i64,
}

impl SimulatedHistory {
    /// The instant the simulated history begins: nothing exists before it.
    ///
    /// The past covers whole UTC days, so `--history 30d` means the thirty days that end
    /// today and the same command replays the same days. A past that started at the instant
    /// a request arrived would move with the clock, and two requests a second apart would
    /// disagree about the first fault of the window.
    fn first_ms(&self) -> i64 {
        day_start_of(self.now_ms).saturating_sub(self.history_days.saturating_sub(1) * DAY_MS)
    }

    /// Where a target sits on the board, which is half of the identity of its events.
    fn target_index(&self, key: &str) -> usize {
        self.targets
            .iter()
            .position(|target| target.key() == key)
            .unwrap_or_default()
    }

    /// The id of a simulated transition: its instant and its target, so the id of an event
    /// does not depend on the window that was asked for.
    ///
    /// The instant sits above `TARGET_SHIFT` bits, so `id >> TARGET_SHIFT` gives it back and
    /// an export can start at the page it was asked for instead of at the beginning of the
    /// simulated past.
    fn event_id(&self, key: &str, occurred_at_ms: i64) -> i64 {
        occurred_at_ms
            .saturating_mul(1 << TARGET_SHIFT)
            .saturating_add(self.target_index(key) as i64)
    }

    fn priority_of(&self, key: &str) -> Option<i64> {
        self.targets
            .iter()
            .find(|target| target.key() == key)
            .and_then(|target| target.priority)
    }

    /// The lane order of the real board: the configured priority, lowest first, then the
    /// targets without one, by key.
    fn sort_into_lanes<T>(&self, rows: &mut [T], key_of: impl Fn(&T) -> &str) {
        rows.sort_by(|left, right| {
            let (left_priority, right_priority) = (
                self.priority_of(key_of(left)),
                self.priority_of(key_of(right)),
            );
            left_priority
                .is_none()
                .cmp(&right_priority.is_none())
                .then_with(|| left_priority.cmp(&right_priority))
                .then_with(|| key_of(left).cmp(key_of(right)))
        });
    }

    /// The episode a target is inside at `instant`.
    ///
    /// A day covers itself, so the day that contains the instant holds the answer even when
    /// the episode began the day before.
    fn episode_at(&self, target: &Target, instant: i64) -> Option<SimulatedEpisode> {
        simulated_day(
            &target.endpoint,
            self.seed,
            day_start_of(instant),
            self.fault_rate,
        )
        .into_iter()
        .find(|episode| episode.from_ms <= instant && instant < episode.to_ms)
    }

    /// The transitions of a window of a day or two.
    ///
    /// This is the unit every read below is built from, never a call over the whole past: a
    /// request must cost what it returns, whatever `--history` and `--fault-rate` say. One
    /// day is generated before `from_ms`, so the state at `from_ms` and a fault that opened
    /// earlier are known, and one day after `to_ms`, so a fault that ends after the window
    /// still states when it ended. An episode that repeats the state before it is not a
    /// transition and is not emitted, which is the rule the monitor writes by (BR-1).
    fn transitions(&self, from_ms: i64, to_ms: i64) -> Vec<Event> {
        let first = self.first_ms();
        let to_ms = to_ms.min(self.now_ms.saturating_add(1));
        let from_ms = from_ms.min(to_ms);
        if to_ms <= first || to_ms <= from_ms {
            return Vec::new();
        }
        let window_start = from_ms.saturating_sub(DAY_MS).max(first);

        let mut events = Vec::new();
        for target in &self.targets {
            let mut previous = TargetState::Unknown;
            // The history begins with a verdict, as a monitor's does: the target was up or
            // down when the simulated past opened, and that is its first event.
            if window_start == first {
                if let Some(episode) = self.episode_at(target, first) {
                    events.push(event_of(target, &episode, TargetState::Unknown, first));
                    previous = state_of(&episode);
                }
            }
            let mut day = day_start_of(window_start);
            while day < to_ms {
                for episode in simulated_day(&target.endpoint, self.seed, day, self.fault_rate) {
                    if episode.from_ms >= to_ms {
                        break;
                    }
                    let state = state_of(&episode);
                    let was = previous;
                    previous = state;
                    if state == was || episode.from_ms < window_start {
                        continue;
                    }
                    events.push(event_of(target, &episode, was, episode.from_ms));
                }
                day = day.saturating_add(DAY_MS);
            }
        }

        events.sort_by(|left, right| {
            left.occurred_at_ms
                .cmp(&right.occurred_at_ms)
                .then_with(|| left.target.cmp(&right.target))
        });
        events
    }

    /// The first and the last UTC day a window touches, clipped to the covered past.
    fn day_span(&self, from_ms: i64, to_ms: i64) -> Option<(i64, i64)> {
        let to_ms = to_ms.min(self.now_ms.saturating_add(1));
        let from_ms = from_ms.max(self.first_ms());
        if to_ms <= from_ms {
            return None;
        }
        Some((day_start_of(from_ms), day_start_of(to_ms - 1)))
    }

    /// The transitions that happened inside one UTC day, in the order the database would
    /// have given them: by instant, then by the position of the target on the board.
    fn day_events(&self, day_start_ms: i64) -> Vec<Event> {
        let day_end_ms = day_start_ms.saturating_add(DAY_MS);
        let mut events: Vec<Event> = self
            .transitions(day_start_ms, day_end_ms)
            .into_iter()
            .filter(|event| {
                event.occurred_at_ms >= day_start_ms && event.occurred_at_ms < day_end_ms
            })
            .collect();
        events.sort_by_key(|event| (event.occurred_at_ms, self.target_index(&event.target)));
        events
    }

    /// When a fault ended, as of this request: `None` while it is still running.
    ///
    /// The draws know where a fault will end, and the history must not state it before it
    /// happens — `store::Fault` states no end for a target that is still down, the page
    /// prints "and counting" instead of a duration, and a window that reaches into the
    /// future measures no down time there.
    fn ended_at(&self, outage: &Outage) -> Option<i64> {
        (outage.to_ms <= self.now_ms).then_some(outage.to_ms)
    }

    /// The faults that started inside one UTC day, newest first.
    ///
    /// The starts come from the draws of the day, which is the one list of faults there is:
    /// the day a fault began in is the day it is counted and listed in. An episode cannot
    /// answer this — the day after a fault opens with that same fault, and a stretch that
    /// begins at midnight merely continues one that began the day before. The episode is read
    /// only for what it alone states: the cause the target failed with.
    fn day_faults(&self, day_start_ms: i64, target: Option<&str>) -> Vec<Fault> {
        let day_end_ms = day_start_ms.saturating_add(DAY_MS);
        let mut faults: Vec<Fault> = Vec::new();
        for each in &self.targets {
            let key = each.key();
            if target.is_some_and(|wanted| wanted != key) {
                continue;
            }
            let episodes = simulated_day(&each.endpoint, self.seed, day_start_ms, self.fault_rate);
            for outage in outages(self.seed, &key, day_start_ms, self.fault_rate) {
                if outage.from_ms < day_start_ms || outage.from_ms >= day_end_ms {
                    continue;
                }
                let Some(episode) = episodes
                    .iter()
                    .find(|episode| episode.from_ms == outage.from_ms)
                else {
                    continue;
                };
                faults.push(Fault {
                    event_id: self.event_id(&key, outage.from_ms),
                    target: key.clone(),
                    label: each.label.clone(),
                    interface: each.interface.clone(),
                    started_at_ms: outage.from_ms,
                    ended_at_ms: self.ended_at(&outage),
                    cause: episode.cause.as_ref().map(|cause| cause.code().to_string()),
                    cause_message: episode
                        .cause
                        .as_ref()
                        .map(|cause| cause.message().to_string()),
                });
            }
        }
        faults.sort_by(|left, right| {
            right
                .started_at_ms
                .cmp(&left.started_at_ms)
                .then_with(|| right.event_id.cmp(&left.event_id))
        });
        faults
    }

    /// The target a needle names, by key, by label, or by the bare address an ICMP target
    /// used to be known by.
    fn target_of(&self, needle: &str) -> Option<&Target> {
        let bare = format!("icmp://{needle}");
        self.targets.iter().find(|target| {
            let key = target.key();
            key == needle || target.label.as_deref() == Some(needle) || key == bare
        })
    }

    /// One target's row, or `None` when the needle names nothing on the board.
    fn row_of(&self, needle: &str) -> Option<TargetStateRow> {
        let key = self.target_of(needle)?.key();
        self.rows().into_iter().find(|row| row.target == key)
    }

    /// The current row of every target, in lane order.
    ///
    /// The state is the last transition at or before `now`, and a target whose history has
    /// not begun yet is not on the board. Only the stretch the target is inside is read —
    /// plus as many days before it as that stretch has lasted —, never the whole past: a
    /// page must cost what its window costs, whatever `--history` and `--fault-rate` say.
    fn rows(&self) -> Vec<TargetStateRow> {
        let first = self.first_ms();
        let now = self.now_ms;
        let mut rows: Vec<TargetStateRow> = self
            .targets
            .iter()
            .filter_map(|target| {
                let episode = self.episode_at(target, now)?;
                Some(TargetStateRow {
                    target: target.key(),
                    interface: target.interface.clone(),
                    label: target.label.clone(),
                    state: state_of(&episode),
                    first_seen_at_ms: first,
                    last_change_at_ms: self.run_start_ms(target, &episode),
                    last_rtt_us: episode.rtt_us,
                })
            })
            .collect();
        self.sort_into_lanes(&mut rows, |row| row.target.as_str());
        rows
    }

    /// Where the stretch that `episode` belongs to began.
    ///
    /// A day is cut at midnight, so the episode a target is inside may start at a boundary
    /// where nothing happened: the run began earlier, and the walk goes back day by day
    /// while the state holds. A target that never fails has been up since the history began,
    /// which is known from its life and needs no walk at all.
    fn run_start_ms(&self, target: &Target, episode: &SimulatedEpisode) -> i64 {
        let first = self.first_ms();
        let state = state_of(episode);
        let mut start = episode.from_ms;
        if life(self.seed, &target.key()) == Life::AlwaysUp {
            return first;
        }
        while start > first {
            match self.episode_at(target, start - 1) {
                Some(previous) if state_of(&previous) == state && previous.from_ms < start => {
                    start = previous.from_ms;
                }
                _ => break,
            }
        }
        start
    }

    /// The segments of one target over `[from_ms, to_ms)`, clipped and merged.
    ///
    /// The window may reach into the future, as the page's default window does; the simulated
    /// history stops at `now` and its last state extends to the end of the window, which is
    /// what the chart of the real board draws too.
    fn segments_of(&self, target: &Target, from_ms: i64, to_ms: i64) -> Vec<TimelineSegment> {
        let first = self.first_ms();
        let now = self.now_ms;
        if to_ms <= from_ms || to_ms <= first || from_ms > now {
            return Vec::new();
        }
        let from_ms = from_ms.max(first);
        let mut segments: Vec<TimelineSegment> = Vec::new();
        let mut day = day_start_of(from_ms);
        while day < to_ms {
            for episode in simulated_day(&target.endpoint, self.seed, day, self.fault_rate) {
                let state = state_of(&episode);
                let from = episode.from_ms.max(from_ms);
                let to = episode.to_ms.min(to_ms);
                if to <= from {
                    continue;
                }
                match segments.last_mut() {
                    Some(last) if last.state == state && last.to_ms == from => last.to_ms = to,
                    _ => segments.push(TimelineSegment {
                        state,
                        from_ms: from,
                        to_ms: to,
                    }),
                }
            }
            day = day.saturating_add(DAY_MS);
        }
        if let Some(last) = segments.last_mut() {
            last.to_ms = to_ms;
        }
        segments
    }
}

impl SimulatedHistory {
    /// One transition as the event list states it.
    fn row_of_event(&self, event: &Event) -> EventRow {
        EventRow {
            id: self.event_id(&event.target, event.occurred_at_ms),
            target: event.target.clone(),
            interface: event.interface.clone(),
            previous_state: event.previous_state,
            state: event.state,
            occurred_at_ms: event.occurred_at_ms,
            rtt_us: event.rtt_us,
            cause: event.cause.clone(),
            cause_message: event.cause_message.clone(),
        }
    }
}

/// One transition as the export writes it.
fn exported_of(event: &Event, id: i64) -> ExportedEvent {
    ExportedEvent {
        id,
        target: event.target.clone(),
        label: event.label.clone(),
        interface: event.interface.clone(),
        state: event.state,
        occurred_at_ms: event.occurred_at_ms,
        rtt_us: event.rtt_us,
        cause: event.cause.clone(),
        cause_message: event.cause_message.clone(),
    }
}

/// How many of a day's draws started inside the window, once the fault of the day before
/// that is still running at midnight is taken into account.
fn outage_starts<'a>(
    drawn: impl Iterator<Item = &'a Outage>,
    day_start_ms: i64,
    from_ms: i64,
    to_ms: i64,
    spill_end: i64,
) -> usize {
    let first = from_ms.max(day_start_ms);
    let last = to_ms.min(day_start_ms.saturating_add(DAY_MS));
    drawn
        .filter(|outage| outage.from_ms >= first && outage.from_ms < last)
        .filter(|outage| outage.from_ms >= spill_end)
        .count()
}

/// Whether a fault is behind the keyset cursor, the way the reader's `WHERE` clause asks.
fn after_cursor(fault: &Fault, filter: &FaultFilter) -> bool {
    match filter.after_ms {
        Some(after_ms) => {
            let after_id = filter.after_id.unwrap_or(i64::MAX);
            fault.started_at_ms < after_ms
                || (fault.started_at_ms == after_ms && fault.event_id < after_id)
        }
        None => true,
    }
}

/// The bits of an event id that hold the position of the target on the board.
///
/// A board is one thread per target in the real monitor, so four thousand of them is far
/// beyond anything this tool runs, and the instant keeps its exact place in the id.
const TARGET_SHIFT: u32 = 12;

/// The instant an event id states.
fn id_instant(id: i64) -> i64 {
    id >> TARGET_SHIFT
}

/// The start of the UTC day that contains `instant`.
fn day_start_of(instant: i64) -> i64 {
    instant.div_euclid(DAY_MS).saturating_mul(DAY_MS)
}

/// The state an episode reports.
fn state_of(episode: &SimulatedEpisode) -> TargetState {
    match episode.state {
        SimulatedState::Up => TargetState::Up,
        SimulatedState::Down => TargetState::Down,
    }
}

/// One transition of a target, as the history records it.
fn event_of(
    target: &Target,
    episode: &SimulatedEpisode,
    previous_state: TargetState,
    occurred_at_ms: i64,
) -> Event {
    Event {
        target: target.key(),
        interface: target.interface.clone(),
        label: target.label.clone(),
        priority: target.priority,
        previous_state,
        state: state_of(episode),
        occurred_at_ms,
        rtt_us: episode.rtt_us,
        cause: episode.cause.as_ref().map(|cause| cause.code().to_string()),
        cause_message: episode
            .cause
            .as_ref()
            .map(|cause| cause.message().to_string()),
    }
}

impl History for SimulatedHistory {
    fn states(&self) -> Result<Vec<TargetStateRow>, HistoryError> {
        Ok(self.rows())
    }

    fn find_target(&self, needle: &str) -> Result<Option<TargetStateRow>, HistoryError> {
        Ok(self.row_of(needle))
    }

    fn resolve_target(&self, needle: &str) -> Result<Option<String>, HistoryError> {
        Ok(self.target_of(needle).map(|target| target.key()))
    }

    fn timeline(&self, from_ms: i64, to_ms: i64) -> Result<Vec<TargetTimeline>, HistoryError> {
        let mut timelines: Vec<TargetTimeline> = self
            .targets
            .iter()
            .map(|target| TargetTimeline {
                target: target.key(),
                label: target.label.clone(),
                segments: self.segments_of(target, from_ms, to_ms),
            })
            .collect();
        self.sort_into_lanes(&mut timelines, |timeline| timeline.target.as_str());
        Ok(timelines)
    }

    fn timeline_of(
        &self,
        target: &str,
        from_ms: i64,
        to_ms: i64,
    ) -> Result<TargetTimeline, HistoryError> {
        Ok(match self.target_of(target) {
            Some(configured) => TargetTimeline {
                target: configured.key(),
                label: configured.label.clone(),
                segments: self.segments_of(configured, from_ms, to_ms),
            },
            None => TargetTimeline {
                target: target.to_string(),
                label: None,
                segments: Vec::new(),
            },
        })
    }

    fn daily_drops(
        &self,
        target: Option<&str>,
        from_ms: i64,
        to_ms: i64,
        now: i64,
    ) -> Result<Vec<DailyDrops>, HistoryError> {
        let first = self.first_ms();
        let first_covered_day = day_start_of(first);
        let mut days = Vec::new();
        let mut day = day_start_of(from_ms);
        while day < to_ms {
            let first_of_day = day.max(from_ms);
            let last_of_day = day.saturating_add(DAY_MS).min(to_ms);
            // A day the simulated past does not reach is a zero, not a day of an invented
            // board.
            if last_of_day <= first {
                days.push(DailyDrops {
                    day_start_ms: day,
                    drops: 0,
                    down_ms: 0,
                });
                day = day.saturating_add(DAY_MS);
                continue;
            }

            // The faults of this day and the last one of the day before, which is the only
            // one that can still be running: a histogram of a window that may be years long
            // is built from the instants of the faults, never from the faults.
            let mut spans: Vec<(i64, Option<i64>)> = Vec::new();
            let mut sample_days = vec![day];
            if day.saturating_sub(DAY_MS) >= first_covered_day {
                sample_days.push(day.saturating_sub(DAY_MS));
            }
            for sample in sample_days {
                for each in &self.targets {
                    let key = each.key();
                    if target.is_some_and(|wanted| wanted != key) {
                        continue;
                    }
                    spans.extend(
                        outages(self.seed, &key, sample, self.fault_rate)
                            .into_iter()
                            .map(|outage| (outage.from_ms, self.ended_at(&outage))),
                    );
                }
            }

            days.extend(daily_drops_from_spans(
                &spans,
                first_of_day,
                last_of_day,
                now,
            ));
            day = day.saturating_add(DAY_MS);
        }
        Ok(days)
    }

    /// One page of faults, newest first.
    ///
    /// The page is filled one day at a time, from the newest day backwards, and the walk
    /// stops as soon as it holds what was asked for: a request that states a window of ten
    /// years and a page of ten rows reads ten rows, not ten years.
    fn faults(&self, filter: &FaultFilter) -> Result<Vec<Fault>, HistoryError> {
        // The past ends at `now`: a window that reaches past it holds nothing there, and a
        // fault that has not happened yet is not a fault.
        let from_ms = filter
            .from_ms
            .unwrap_or(self.first_ms())
            .max(self.first_ms());
        let to_ms = filter
            .to_ms
            .unwrap_or(self.now_ms.saturating_add(1))
            .min(self.now_ms.saturating_add(1));
        let offset = filter.offset.unwrap_or(0) as usize;
        let limit = filter.limit.map_or(usize::MAX, |limit| limit as usize);
        let wanted = offset.saturating_add(limit);
        let Some((first_day, last_day)) = self.day_span(from_ms, to_ms) else {
            return Ok(Vec::new());
        };

        let mut page: Vec<Fault> = Vec::new();
        let mut day = last_day;
        while page.len() < wanted {
            for fault in self.day_faults(day, filter.target.as_deref()) {
                if fault.started_at_ms < from_ms || fault.started_at_ms >= to_ms {
                    continue;
                }
                if !after_cursor(&fault, filter) {
                    continue;
                }
                page.push(fault);
                if page.len() == wanted {
                    break;
                }
            }
            if day <= first_day {
                break;
            }
            day = day.saturating_sub(DAY_MS);
        }
        Ok(page.into_iter().skip(offset).take(limit).collect())
    }

    /// How many faults a window holds.
    ///
    /// The answer is a number, so the count builds none of what it counts: it walks the
    /// draws of the window one day at a time and keeps no fault, no event and no string.
    fn count_faults(&self, filter: &FaultFilter) -> Result<i64, HistoryError> {
        let from_ms = filter
            .from_ms
            .unwrap_or(self.first_ms())
            .max(self.first_ms());
        let to_ms = filter
            .to_ms
            .unwrap_or(self.now_ms.saturating_add(1))
            .min(self.now_ms.saturating_add(1));
        let Some((first_day, last_day)) = self.day_span(from_ms, to_ms) else {
            return Ok(0);
        };

        let mut total = 0;
        for target in &self.targets {
            let key = target.key();
            if filter.target.as_deref().is_some_and(|wanted| wanted != key) {
                continue;
            }
            // One generation per day, walking forward: the last fault of a day is the spill
            // the next day opens with, so carrying it forward keeps the count exact without
            // generating any day twice. The first day gets the spill the day before it states,
            // which is the rule `outages` applies and the page therefore shows; starting at
            // `i64::MIN` would count the fault that spill sweeps away and state a total the
            // page can never reach.
            let mut spill_end = raw_outages(
                self.seed,
                &key,
                first_day.saturating_sub(DAY_MS),
                self.fault_rate,
            )
            .last()
            .map_or(i64::MIN, |outage| outage.to_ms);
            let mut day = first_day;
            while day <= last_day {
                let drawn = raw_outages(self.seed, &key, day, self.fault_rate);
                total += outage_starts(drawn.iter(), day, from_ms, to_ms, spill_end) as i64;
                spill_end = drawn.last().map_or(i64::MIN, |outage| outage.to_ms);
                day = day.saturating_add(DAY_MS);
            }
        }
        Ok(total)
    }

    fn recent_faults(&self, limit: u32) -> Result<Vec<Fault>, HistoryError> {
        self.faults(&FaultFilter {
            limit: Some(limit),
            ..FaultFilter::default()
        })
    }

    fn recent_faults_for(&self, target: &str, limit: u32) -> Result<Vec<Fault>, HistoryError> {
        self.faults(&FaultFilter {
            target: Some(target.to_string()),
            limit: Some(limit),
            ..FaultFilter::default()
        })
    }

    /// One page of transitions, newest first, filled one day at a time from the newest day
    /// backwards and stopping at the page.
    fn events(&self, filter: &EventFilter) -> Result<Vec<EventRow>, HistoryError> {
        let from_ms = filter.since_ms.unwrap_or(self.first_ms());
        let to_ms = filter.until_ms.unwrap_or(self.now_ms.saturating_add(1));
        let offset = filter.offset.unwrap_or(0) as usize;
        let limit = filter.limit.map_or(usize::MAX, |limit| limit as usize);
        let wanted = offset.saturating_add(limit);
        let Some((first_day, last_day)) = self.day_span(from_ms, to_ms) else {
            return Ok(Vec::new());
        };

        let mut rows: Vec<EventRow> = Vec::new();
        let mut day = last_day;
        while rows.len() < wanted {
            for event in self.day_events(day).into_iter().rev() {
                if event.occurred_at_ms < from_ms || event.occurred_at_ms >= to_ms {
                    continue;
                }
                if filter
                    .target
                    .as_deref()
                    .is_some_and(|key| event.target != key)
                {
                    continue;
                }
                rows.push(self.row_of_event(&event));
                if rows.len() == wanted {
                    break;
                }
            }
            if day <= first_day {
                break;
            }
            day = day.saturating_sub(DAY_MS);
        }
        Ok(rows.into_iter().skip(offset).take(limit).collect())
    }

    /// One page of the export, oldest first, starting at the day the cursor names.
    ///
    /// The cursor is an id, and an id states its instant, so a page of an export costs that
    /// page: the walk begins at the day of the last row the caller read, not at the
    /// beginning of the simulated past.
    fn events_after(
        &self,
        target: Option<&str>,
        after_id: i64,
        limit: u32,
    ) -> Result<Vec<ExportedEvent>, HistoryError> {
        let from_ms = id_instant(after_id.max(0)).max(self.first_ms());
        let Some((first_day, last_day)) = self.day_span(from_ms, self.now_ms.saturating_add(1))
        else {
            return Ok(Vec::new());
        };

        let mut rows: Vec<ExportedEvent> = Vec::new();
        let mut day = first_day;
        while day <= last_day && rows.len() < limit as usize {
            for event in self.day_events(day) {
                let id = self.event_id(&event.target, event.occurred_at_ms);
                if id <= after_id {
                    continue;
                }
                if target.is_some_and(|key| event.target != key) {
                    continue;
                }
                rows.push(exported_of(&event, id));
                if rows.len() == limit as usize {
                    break;
                }
            }
            day = day.saturating_add(DAY_MS);
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::net::Ipv4Addr;
    use std::time::Duration;

    use crate::probe::{Endpoint, RecordType, Scheme, StatusMatch};

    use super::*;

    /// A mid-morning instant, so a test window around it does not sit on a day boundary.
    const NOW: i64 = 1_767_225_600_000 + 12 * 3_600_000;

    fn targets() -> Vec<Target> {
        vec![
            Target {
                endpoint: Endpoint::Icmp {
                    address: Ipv4Addr::new(1, 1, 1, 1),
                },
                interface: Some("en0".to_string()),
                label: Some("gw".to_string()),
                priority: Some(20),
                timeout: Duration::from_millis(2_000),
                interval: Duration::from_secs(30),
            },
            Target {
                endpoint: Endpoint::Tcp {
                    host: "example.com".to_string(),
                    port: 443,
                },
                interface: None,
                label: Some("web".to_string()),
                priority: Some(10),
                timeout: Duration::from_millis(2_000),
                interval: Duration::from_secs(30),
            },
            Target {
                endpoint: Endpoint::Http {
                    scheme: Scheme::Https,
                    host: "example.com".to_string(),
                    port: 443,
                    target: "/health".to_string(),
                    expect: StatusMatch::default(),
                },
                interface: None,
                label: None,
                priority: None,
                timeout: Duration::from_millis(2_000),
                interval: Duration::from_secs(30),
            },
            Target {
                endpoint: Endpoint::Dns {
                    name: "example.com".to_string(),
                    resolver: None,
                    record: RecordType::A,
                },
                interface: None,
                label: None,
                priority: None,
                timeout: Duration::from_millis(2_000),
                interval: Duration::from_secs(30),
            },
        ]
    }

    fn history(history_days: i64, seed: u64) -> SimulatedHistory {
        SimulatedHistory {
            targets: targets(),
            seed,
            history_days,
            fault_rate: 3.0,
            now_ms: NOW,
        }
    }

    /// A history whose last two days hold plenty of faults.
    fn with_faults() -> SimulatedHistory {
        for seed in 0..500 {
            let history = history(30, seed);
            if history.recent_faults(50).expect("a read").len() >= 50 {
                return history;
            }
        }
        panic!("no seed produced faults");
    }

    #[test]
    fn the_states_are_the_last_transition_at_or_before_now_in_lane_order() {
        let history = with_faults();
        let rows = history.states().expect("a read");
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows.iter()
                .map(|row| row.target.as_str())
                .collect::<Vec<_>>(),
            vec![
                "tcp://example.com:443",
                "icmp://1.1.1.1",
                "dns://example.com",
                "https://example.com/health",
            ],
            "priority first, then the targets without one, by key"
        );

        let row = &rows[0];
        assert!(row.last_change_at_ms <= NOW);
        assert!(row.first_seen_at_ms <= row.last_change_at_ms);
        assert_eq!(row.first_seen_at_ms, history.first_ms());
        assert_eq!(row.label.as_deref(), Some("web"));
        assert_eq!(row.interface, None);
        assert!(matches!(row.state, TargetState::Up | TargetState::Down));
        if row.state == TargetState::Up {
            assert!(row.last_rtt_us.is_some());
        } else {
            assert_eq!(row.last_rtt_us, None);
        }
    }

    /// The age of the current state is the age of the whole run, not the age of the UTC day
    /// it happens to be drawn in: the timeline of the same board must agree with the row.
    ///
    /// A day is cut at midnight whether or not anything happened there, so a row that took
    /// the start of its day as the last change would state a target up since midnight that
    /// has in truth been up for days.
    #[test]
    fn the_last_change_of_a_row_is_the_start_of_the_run_the_timeline_draws() {
        let history = with_faults();
        let rows = history.states().expect("a read");
        let timelines = history.timeline(history.first_ms(), NOW).expect("a read");
        for row in &rows {
            let timeline = timelines
                .iter()
                .find(|timeline| timeline.target == row.target)
                .expect("one lane per target");
            let last = timeline.segments.last().expect("a segment");
            assert_eq!(row.state, last.state, "{}", row.target);
            assert_eq!(
                row.last_change_at_ms, last.from_ms,
                "{}: the row and the timeline disagree about when the run began",
                row.target
            );
        }
    }

    /// A target that never fails has been up since the history began, however long the
    /// history is, and reading its row must not walk it.
    #[test]
    fn a_target_that_never_fails_has_been_up_since_the_history_began() {
        for seed in 0..500 {
            let history = history(3_660, seed);
            let Some(index) = history
                .targets
                .iter()
                .position(|target| life(seed, &target.key()) == Life::AlwaysUp)
            else {
                continue;
            };
            let started = std::time::Instant::now();
            let rows = history.states().expect("a read");
            let elapsed = started.elapsed();
            let row = &rows[index];
            assert_eq!(row.state, TargetState::Up);
            assert_eq!(row.first_seen_at_ms, history.first_ms());
            assert_eq!(
                row.last_change_at_ms,
                history.first_ms(),
                "a target that never fails has been up since the history began"
            );
            assert!(
                elapsed < std::time::Duration::from_secs(2),
                "reading a board of {seed} took {elapsed:?}: the whole past was regenerated"
            );
            return;
        }
        panic!("no seed drew a target that never fails");
    }

    #[test]
    fn a_target_is_named_by_key_by_label_or_by_its_bare_address() {
        let history = with_faults();
        let by_key = history
            .find_target("icmp://1.1.1.1")
            .expect("a read")
            .expect("a target");
        for needle in ["gw", "1.1.1.1"] {
            assert_eq!(
                history.find_target(needle).expect("a read").as_ref(),
                Some(&by_key),
                "{needle} names the ICMP target"
            );
            assert_eq!(
                history.resolve_target(needle).expect("a read").as_deref(),
                Some("icmp://1.1.1.1")
            );
        }
        assert!(history.find_target("nothing").expect("a read").is_none());
        assert!(history.resolve_target("nothing").expect("a read").is_none());
    }

    #[test]
    fn the_timeline_covers_the_window_exactly() {
        let history = with_faults();
        let from_ms = NOW - 6 * 3_600_000;
        let timelines = history.timeline(from_ms, NOW).expect("a read");
        assert_eq!(timelines.len(), 4);
        assert_eq!(timelines[0].target, "tcp://example.com:443");

        for timeline in &timelines {
            let segments = &timeline.segments;
            assert!(!segments.is_empty(), "{} has no segments", timeline.target);
            assert_eq!(segments[0].from_ms, from_ms);
            assert_eq!(segments.last().expect("a segment").to_ms, NOW);
            for pair in segments.windows(2) {
                assert_eq!(pair[0].to_ms, pair[1].from_ms, "no gap and no overlap");
                assert_ne!(pair[0].state, pair[1].state, "INV-1");
            }
        }
    }

    #[test]
    fn the_timeline_of_a_window_in_the_future_stops_at_now() {
        let history = with_faults();
        let timelines = history
            .timeline(NOW - 3_600_000, NOW + 3_600_000)
            .expect("a read");
        for timeline in &timelines {
            assert_eq!(
                timeline.segments.last().expect("a segment").to_ms,
                NOW + 3_600_000,
                "the last state extends to the end of the window"
            );
            assert!(timeline
                .segments
                .iter()
                .all(|segment| segment.from_ms < NOW + 3_600_000));
        }
    }

    #[test]
    fn the_daily_histogram_has_one_entry_per_day_and_honours_the_target() {
        let history = with_faults();
        let from_ms = NOW - 2 * DAY_MS;
        let days = history
            .daily_drops(None, from_ms, NOW, NOW)
            .expect("a read");
        assert_eq!(days.len(), 3, "three UTC days touch the window");
        assert_eq!(days[0].day_start_ms, from_ms.div_euclid(DAY_MS) * DAY_MS);
        assert!(days.iter().map(|day| day.drops).sum::<u32>() > 0);
        assert!(days
            .iter()
            .all(|day| day.down_ms <= DAY_MS && day.down_ms >= 0));

        let one = history
            .daily_drops(Some("icmp://1.1.1.1"), from_ms, NOW, NOW)
            .expect("a read");
        assert!(
            one.iter().map(|day| day.drops).sum::<u32>()
                < days.iter().map(|day| day.drops).sum::<u32>(),
            "one target cannot hold every drop of the board"
        );
    }

    #[test]
    fn the_faults_honour_the_window_the_page_and_the_cursor() {
        let history = with_faults();
        let from_ms = NOW - 2 * DAY_MS;
        let filter = FaultFilter {
            from_ms: Some(from_ms),
            to_ms: Some(NOW),
            ..FaultFilter::default()
        };
        let all = history.faults(&filter).expect("a read");
        assert!(all.len() > 4, "the fixture has faults to page");
        assert!(
            all.iter()
                .all(|fault| fault.started_at_ms >= from_ms && fault.started_at_ms < NOW),
            "every fault started inside the window"
        );
        assert!(all.windows(2).all(|pair| {
            pair[0].started_at_ms > pair[1].started_at_ms
                || (pair[0].started_at_ms == pair[1].started_at_ms
                    && pair[0].event_id > pair[1].event_id)
        }));

        let page = history
            .faults(&FaultFilter {
                limit: Some(2),
                offset: Some(1),
                ..filter.clone()
            })
            .expect("a read");
        assert_eq!(page, all[1..3].to_vec());

        let cursor = history
            .faults(&FaultFilter {
                after_ms: Some(all[1].started_at_ms),
                after_id: Some(all[1].event_id),
                ..filter.clone()
            })
            .expect("a read");
        assert_eq!(
            cursor,
            all[2..].to_vec(),
            "the walk resumes after the cursor"
        );

        assert_eq!(
            history.count_faults(&filter).expect("a read"),
            all.len() as i64,
            "counting ignores the page"
        );
        let counted = history
            .count_faults(&FaultFilter {
                limit: Some(2),
                ..filter
            })
            .expect("a read");
        assert_eq!(counted, all.len() as i64);
    }

    /// The count of a window and the page over it are two different walks — the count reads
    /// the draws of a day, the page reads the transitions — and they must agree.
    ///
    /// They have disagreed twice, and both times only on a boundary the fixture did not
    /// reach: a window that reaches past `now`, and a window whose first day begins inside a
    /// fault still running from the day before. So the sweep is over the rates and over the
    /// alignment of the window inside its day, at midnight above all.
    #[test]
    fn the_count_of_a_window_is_the_page_over_it() {
        for rate in [3.0, 50.0, 500.0] {
            for alignment in [0, 3_600_000, 12 * 3_600_000] {
                let history = SimulatedHistory {
                    fault_rate: rate,
                    ..history(30, 11)
                };
                let base = day_start_of(NOW - 20 * DAY_MS) + alignment;
                for step in 0..6 {
                    let from_ms = base + step * 2 * DAY_MS;
                    let filter = FaultFilter {
                        from_ms: Some(from_ms),
                        to_ms: Some(from_ms + 2 * 3_600_000),
                        ..FaultFilter::default()
                    };
                    let page = history
                        .faults(&FaultFilter {
                            limit: Some(1_000),
                            ..filter.clone()
                        })
                        .expect("a read");
                    assert_eq!(
                        history.count_faults(&filter).expect("a read"),
                        page.len() as i64,
                        "the count and the page disagree over {from_ms} at rate {rate},                          {alignment} ms into the day"
                    );
                }
            }
        }
    }

    /// Every fault a day lists is a fault that day drew, and none is listed twice.
    ///
    /// The day after a fault opens with that same fault, so a list built from the stretches
    /// of a day would state one fault twice: once where it began and once at the midnight it
    /// ran through — which is exactly the defect the round-3 review found on the API.
    #[test]
    fn a_fault_is_listed_once_in_the_day_it_began() {
        let history = SimulatedHistory {
            fault_rate: 500.0,
            ..history(10, 3)
        };
        let mut listed: Vec<i64> = Vec::new();
        let mut day = day_start_of(NOW - 5 * DAY_MS);
        while day < NOW {
            let page = history
                .faults(&FaultFilter {
                    from_ms: Some(day),
                    to_ms: Some(day.saturating_add(DAY_MS)),
                    limit: Some(5_000),
                    ..FaultFilter::default()
                })
                .expect("a read");
            // One generation of the day per target, not one per fault.
            let mut drawn_starts: BTreeMap<String, Vec<i64>> = BTreeMap::new();
            for fault in &page {
                let drawn = drawn_starts.entry(fault.target.clone()).or_insert_with(|| {
                    outages(history.seed, &fault.target, day, history.fault_rate)
                        .into_iter()
                        .map(|outage| outage.from_ms)
                        .collect()
                });
                assert!(
                    drawn.contains(&fault.started_at_ms),
                    "{} states {} as a fault of the day, and the day never drew it",
                    fault.target,
                    fault.started_at_ms
                );
                assert!(
                    !listed.contains(&fault.event_id),
                    "{} is listed twice",
                    fault.event_id
                );
                listed.push(fault.event_id);
            }
            day = day.saturating_add(DAY_MS);
        }
        assert!(
            listed.len() > 100,
            "the fixture lists {} faults",
            listed.len()
        );
    }

    /// A fault still running at `now` states no end, exactly as the store states none for a
    /// target that is still down — and the histogram measures no down time in the time that
    /// has not happened yet.
    ///
    /// The draws know where every fault will end; the history must not say it before it does.
    #[test]
    fn a_fault_still_running_states_no_end_and_no_future_down_time() {
        let targets = targets();
        let key = targets[0].key();
        let day = day_start_of(NOW - 6 * DAY_MS);

        for seed in 0..500 {
            let Some(outage) = outages(seed, &key, day, 50.0)
                .into_iter()
                .find(|outage| outage.to_ms - outage.from_ms > 600_000)
            else {
                continue;
            };
            let middle = outage.from_ms + (outage.to_ms - outage.from_ms) / 2;
            let running = SimulatedHistory {
                targets: targets.clone(),
                seed,
                history_days: 2,
                fault_rate: 50.0,
                now_ms: middle,
            };

            let listed = running
                .faults(&FaultFilter {
                    from_ms: Some(day),
                    to_ms: Some(middle.saturating_add(1)),
                    limit: Some(200),
                    ..FaultFilter::default()
                })
                .expect("a read");
            let listed = listed
                .iter()
                .find(|fault| fault.started_at_ms == outage.from_ms && fault.target == key)
                .expect("the running fault is listed");
            assert_eq!(
                listed.ended_at_ms, None,
                "a fault still running states no end"
            );

            let down_ms: i64 = running
                .daily_drops(Some(&key), day, middle + 6 * 3_600_000, middle)
                .expect("a read")
                .iter()
                .map(|bucket| bucket.down_ms)
                .sum();
            assert!(
                down_ms <= middle - day,
                "the histogram measured {down_ms} ms of down time in the {} ms that have                  happened",
                middle - day
            );

            // Once it has ended, the same fault states the end it had.
            let finished = SimulatedHistory {
                now_ms: outage.to_ms.saturating_add(1),
                ..running.clone()
            };
            let listed = finished
                .faults(&FaultFilter {
                    from_ms: Some(day),
                    to_ms: Some(outage.to_ms.saturating_add(2)),
                    limit: Some(200),
                    ..FaultFilter::default()
                })
                .expect("a read");
            let listed = listed
                .iter()
                .find(|fault| fault.started_at_ms == outage.from_ms && fault.target == key)
                .expect("the ended fault is listed");
            assert_eq!(listed.ended_at_ms, Some(outage.to_ms));
            return;
        }
        panic!("no seed drew a fault long enough to stand inside");
    }

    #[test]
    fn the_newest_faults_are_newest_first_and_state_whether_they_are_still_open() {
        let history = with_faults();
        let faults = history.recent_faults(20).expect("a read");
        assert_eq!(faults.len(), 20);
        assert!(faults
            .windows(2)
            .all(|pair| pair[0].started_at_ms >= pair[1].started_at_ms));

        let of_one = history
            .recent_faults_for("icmp://1.1.1.1", 5)
            .expect("a read");
        assert!(of_one.len() <= 5);
        assert!(of_one.iter().all(|fault| fault.target == "icmp://1.1.1.1"));
        for fault in &of_one {
            match fault.ended_at_ms {
                Some(end) => assert!(end > fault.started_at_ms),
                None => assert!(fault.started_at_ms <= NOW),
            }
        }
    }

    #[test]
    fn the_events_are_newest_first_and_state_the_transition_they_record() {
        let history = with_faults();
        let from_ms = NOW - 6 * 3_600_000;
        let rows = history
            .events(&EventFilter {
                since_ms: Some(from_ms),
                until_ms: Some(NOW),
                ..EventFilter::default()
            })
            .expect("a read");
        assert!(!rows.is_empty());
        assert!(rows
            .iter()
            .all(|row| row.occurred_at_ms >= from_ms && row.occurred_at_ms < NOW));
        for row in &rows {
            assert_ne!(
                row.state, row.previous_state,
                "BR-1: a transition changes it"
            );
            match row.state {
                TargetState::Down => {
                    assert!(row.cause.is_some(), "a failure states its cause");
                    assert!(row.cause_message.is_some());
                    assert_eq!(row.rtt_us, None);
                }
                TargetState::Up => {
                    assert_eq!(row.cause, None);
                    assert!(row.rtt_us.is_some());
                }
                TargetState::Unknown => panic!("a simulated transition is never unknown"),
            }
        }

        let page = history
            .events(&EventFilter {
                since_ms: Some(from_ms),
                until_ms: Some(NOW),
                limit: Some(3),
                offset: Some(1),
                ..EventFilter::default()
            })
            .expect("a read");
        assert_eq!(page, rows[1..4].to_vec());
    }

    #[test]
    fn the_export_walks_the_history_by_id() {
        let history = with_faults();
        let first = history.events_after(None, 0, 50).expect("the first page");
        assert_eq!(first.len(), 50);
        assert!(first.windows(2).all(|pair| pair[0].id < pair[1].id));
        assert!(
            first
                .windows(2)
                .all(|pair| pair[0].occurred_at_ms <= pair[1].occurred_at_ms),
            "the export walks the history in time order"
        );

        let second = history
            .events_after(None, first.last().expect("an event").id, 50)
            .expect("the second page");
        assert_eq!(second.len(), 50);
        assert!(
            second[0].id > first.last().expect("an event").id,
            "the walk resumes after the last row it read"
        );

        let of_one = history
            .events_after(Some("dns://example.com"), 0, 10)
            .expect("a page of one target");
        assert!(of_one
            .iter()
            .all(|event| event.target == "dns://example.com"));
    }

    #[test]
    fn nothing_exists_before_the_covered_past() {
        let history = history(2, 0);
        let long_ago = NOW - 10 * DAY_MS;
        assert!(history
            .timeline(long_ago, long_ago + 3_600_000)
            .expect("a read")
            .iter()
            .all(|timeline| timeline.segments.is_empty()));
        assert!(history
            .faults(&FaultFilter {
                from_ms: Some(long_ago),
                to_ms: Some(long_ago + DAY_MS),
                ..FaultFilter::default()
            })
            .expect("a read")
            .is_empty());
        assert!(history
            .events(&EventFilter {
                since_ms: Some(long_ago),
                until_ms: Some(long_ago + DAY_MS),
                ..EventFilter::default()
            })
            .expect("a read")
            .is_empty());

        let row = &history.states().expect("a read")[0];
        assert_eq!(row.first_seen_at_ms, history.first_ms());
        assert_eq!(
            history.first_ms(),
            day_start_of(NOW) - DAY_MS,
            "two days: today and the whole day before it"
        );
    }

    #[test]
    fn the_same_call_twice_states_the_same_thing() {
        let history = with_faults();
        let read = |history: &SimulatedHistory| {
            (
                history.states().expect("a read"),
                history.timeline(NOW - DAY_MS, NOW).expect("a read"),
                history
                    .faults(&FaultFilter {
                        from_ms: Some(NOW - 2 * DAY_MS),
                        to_ms: Some(NOW),
                        ..FaultFilter::default()
                    })
                    .expect("a read"),
                history
                    .events(&EventFilter {
                        since_ms: Some(NOW - DAY_MS),
                        until_ms: Some(NOW),
                        ..EventFilter::default()
                    })
                    .expect("a read"),
            )
        };
        assert_eq!(read(&history), read(&history));
    }

    #[test]
    fn two_sources_of_the_same_seed_answer_the_same_thing() {
        let source = SimulatedSource::new(targets(), 7, 30, 3.0);
        let first = source.open().expect("a history");
        let second = source.open().expect("a history");
        let from_ms = crate::time::now_ms() - 2 * DAY_MS;
        let to_ms = crate::time::now_ms();
        assert_eq!(
            first.timeline(from_ms, to_ms).expect("a read"),
            second.timeline(from_ms, to_ms).expect("a read")
        );
        assert_eq!(
            first
                .faults(&FaultFilter {
                    from_ms: Some(from_ms),
                    to_ms: Some(to_ms),
                    ..FaultFilter::default()
                })
                .expect("a read"),
            second
                .faults(&FaultFilter {
                    from_ms: Some(from_ms),
                    to_ms: Some(to_ms),
                    ..FaultFilter::default()
                })
                .expect("a read")
        );
        assert!(source.describe().contains("seed 7"));
    }

    #[test]
    fn a_day_covered_by_thirty_days_is_the_same_day_covered_by_sixty() {
        let source = with_faults();
        let longer = SimulatedHistory {
            history_days: 60,
            ..source.clone()
        };
        let from_ms = NOW - 2 * DAY_MS;
        assert_eq!(
            source.timeline(from_ms, NOW).expect("a read"),
            longer.timeline(from_ms, NOW).expect("a read")
        );
        let present = |history: &SimulatedHistory| {
            history
                .states()
                .expect("a read")
                .into_iter()
                .map(|row| {
                    (
                        row.target,
                        row.state,
                        row.last_change_at_ms,
                        row.last_rtt_us,
                    )
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            present(&source),
            present(&longer),
            "a longer past does not change the present"
        );
        assert_ne!(
            source.first_ms(),
            longer.first_ms(),
            "only the beginning of the past moves"
        );
    }
}
