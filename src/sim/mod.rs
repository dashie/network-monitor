//! The simulated history: what the generator produces, and the source that serves it.
//!
//! The simulated mode stores nothing. A target's schedule is a pure function of
//! `(seed, target key, UTC day)`, so the same command replays the same history, a page that
//! asks for day 12 does not have to generate days 1 to 11 first, and nothing has to be
//! written to a database or kept in memory between requests (ADR-0032).
//!
//! The dependency runs one way and stops: [`rng`] is a leaf, the generator below uses only
//! it, the four probe modules use the generator to give their own method's failures and
//! latencies to the stretches of a day, and [`history`] puts the two together behind the
//! read contract. Nothing in the generator knows what a probe is.
//!
//! A day is covered exactly — no gap, no overlap, and no two consecutive stretches in the
//! same state (INV-1) — and a fault may start near midnight and run past it. That last
//! property is why [`day`] also looks at the previous day: the day that begins inside a
//! fault opens with that fault, so the outage is one outage and not two.
//!
//! How often a target fails is its *life*, drawn once from `(seed, key)`: most targets are
//! up almost always, some fail a handful of times a day, a few flap.

pub mod history;
pub mod rng;

use rng::Rng;

/// One UTC day in milliseconds.
pub const DAY_MS: i64 = 86_400_000;

/// The shortest fault, and the longest one of a target that only fails now and then.
const MIN_FAULT_MS: i64 = 20_000;
const LONGEST_FAULT_MS: i64 = 2_400_000;
/// The longest fault of a target that flaps: it has to fit many of them in a day.
const LONGEST_FLAP_MS: i64 = 300_000;
/// Above this many faults in a day, the faults are drawn short so they can fit.
const MANY_FAULTS: usize = 8;
/// The most faults a day can hold: the shortest fault after another, all day long.
///
/// A rate above this describes a board that is down rather than a board that fails, and the
/// extra draws would be dropped as overlapping anyway.
const MAX_FAULTS_PER_DAY: usize = (DAY_MS / MIN_FAULT_MS) as usize;

/// How often a simulated target fails, relative to the board's rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Life {
    /// It never fails: the day is one long `up`.
    AlwaysUp,
    /// It fails at the rate the board states, from twenty seconds to forty minutes.
    Occasional,
    /// It flaps: about eight times the rate, in short faults.
    Flapping,
}

impl Life {
    /// What this life is worth, as a multiple of the board's fault rate.
    pub fn weight(self) -> f64 {
        match self {
            Life::AlwaysUp => 0.0,
            Life::Occasional => 1.0,
            Life::Flapping => 8.0,
        }
    }
}

/// One stretch of a day: whether the target answered in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stretch {
    pub up: bool,
    pub from_ms: i64,
    pub to_ms: i64,
}

/// One fault: when it started and when the target answered again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Outage {
    pub from_ms: i64,
    pub to_ms: i64,
}

/// One in five targets never fails, seven in ten fail now and then, one in ten flaps.
pub fn life(seed: u64, key: &str) -> Life {
    match Rng::for_draw(seed, key, 0, "life").below(100) {
        0..=19 => Life::AlwaysUp,
        20..=89 => Life::Occasional,
        _ => Life::Flapping,
    }
}

/// The stretches of one UTC day, in order, covering `[day_start_ms, day_start_ms + DAY_MS)`.
///
/// `fault_rate` is the number of faults a day of a target that fails now and then holds.
pub fn day(seed: u64, key: &str, day_start_ms: i64, fault_rate: f64) -> Vec<Stretch> {
    let day_end_ms = day_start_ms.saturating_add(DAY_MS);
    let mut stretches = Vec::with_capacity(4);
    let mut cursor = day_start_ms;

    // A fault of the day before may still be running at midnight, and the day that begins
    // inside it opens with it. The last fault the day before *drew* is that one, even when a
    // fault from the day before that swept it away: it is the same fault, still running.
    if let Some(previous) = raw_outages(seed, key, day_start_ms - DAY_MS, fault_rate).last() {
        let spilled_to = previous.to_ms.min(day_end_ms);
        if spilled_to > cursor {
            stretches.push(Stretch {
                up: false,
                from_ms: cursor,
                to_ms: spilled_to,
            });
            cursor = spilled_to;
        }
    }

    for outage in outages(seed, key, day_start_ms, fault_rate) {
        // A fault that would start before the day does is one the spill already covers.
        if outage.from_ms < cursor || outage.to_ms <= cursor {
            continue;
        }
        stretches.push(Stretch {
            up: true,
            from_ms: cursor,
            to_ms: outage.from_ms,
        });
        stretches.push(Stretch {
            up: false,
            from_ms: outage.from_ms,
            to_ms: outage.to_ms.min(day_end_ms),
        });
        cursor = outage.to_ms.min(day_end_ms);
    }

    if cursor < day_end_ms {
        stretches.push(Stretch {
            up: true,
            from_ms: cursor,
            to_ms: day_end_ms,
        });
    }

    stretches
}

/// The faults that start inside one UTC day, in order, never overlapping.
///
/// A fault may end after midnight, and a fault that the day before is still running through
/// at this instant never starts at all: the day carries it instead, so a fault is counted
/// once, in the day it starts.
pub fn outages(seed: u64, key: &str, day_start_ms: i64, fault_rate: f64) -> Vec<Outage> {
    let spill = raw_outages(seed, key, day_start_ms - DAY_MS, fault_rate)
        .last()
        .map(|outage| outage.to_ms)
        .filter(|end| *end > day_start_ms);
    match spill {
        Some(end) => raw_outages(seed, key, day_start_ms, fault_rate)
            .into_iter()
            .filter(|outage| outage.from_ms >= end)
            .collect(),
        None => raw_outages(seed, key, day_start_ms, fault_rate),
    }
}

/// How many faults start inside one UTC day.
///
/// The count of a window is a number, and asking for it must not build the faults it counts:
/// this walks the same draws as [`outages`] and keeps no event, no string and no `Fault`.
pub fn fault_count(seed: u64, key: &str, day_start_ms: i64, fault_rate: f64) -> usize {
    outages(seed, key, day_start_ms, fault_rate).len()
}

/// The faults the draws of one UTC day ask for, before the day before is taken into
/// account, in order and never overlapping.
///
/// A fault that would overlap the one before it is dropped rather than shortened, so the
/// duration of a fault is always the one it was drawn with.
fn raw_outages(seed: u64, key: &str, day_start_ms: i64, fault_rate: f64) -> Vec<Outage> {
    let mut rng = Rng::for_draw(seed, key, day_start_ms, "faults");
    let expected = fault_rate * life(seed, key).weight();
    if expected <= 0.0 {
        return Vec::new();
    }
    // A draw in `[0, 2 × expected]`, so the day holds the expected count on average while
    // one day differs from the next.
    let count = (rng.below((expected * 2.0).round() as u64 + 1) as usize).min(MAX_FAULTS_PER_DAY);
    let longest = if count > MANY_FAULTS {
        LONGEST_FLAP_MS
    } else {
        LONGEST_FAULT_MS
    };

    // The starts first, then the durations, so a seed keeps the same stream it always had;
    // then one sort of the pairs, which is what costs when a day holds a thousand faults.
    let mut drawn: Vec<Outage> = (0..count)
        .map(|_| Outage {
            from_ms: day_start_ms.saturating_add(rng.between(0, DAY_MS)),
            to_ms: 0,
        })
        .collect();
    for outage in &mut drawn {
        outage.to_ms = outage
            .from_ms
            .saturating_add(rng.between(MIN_FAULT_MS, longest));
    }
    drawn.sort_unstable_by_key(|outage| outage.from_ms);

    let mut outages: Vec<Outage> = Vec::with_capacity(count);
    for outage in drawn {
        if outages
            .last()
            .map_or(true, |previous| outage.from_ms >= previous.to_ms)
        {
            outages.push(outage);
        }
    }
    outages
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tuple of a target that never fails, of an occasional one, and of a flapping one.
    fn example(life: Life, seed: u64) -> (u64, String) {
        let key = "icmp://1.1.1.1";
        for probe in 0..10_000 {
            let seed = seed + probe;
            if super::life(seed, key) == life {
                return (seed, key.to_string());
            }
        }
        panic!("no seed draws {life:?}");
    }

    /// The rate the tests draw with: a board of targets that fail now and then.
    const RATE: f64 = 3.0;

    fn covers_the_day(stretches: &[Stretch], day_start_ms: i64) {
        assert!(!stretches.is_empty(), "a day is never empty");
        assert_eq!(
            stretches[0].from_ms, day_start_ms,
            "the day starts with one"
        );
        assert_eq!(
            stretches.last().expect("a stretch").to_ms,
            day_start_ms + DAY_MS,
            "the day ends with one"
        );
        for pair in stretches.windows(2) {
            assert_eq!(pair[0].to_ms, pair[1].from_ms, "no gap and no overlap");
            assert_ne!(pair[0].up, pair[1].up, "INV-1: the state must change");
        }
    }

    #[test]
    fn a_day_covers_itself_and_the_result_is_reproducible() {
        let (seed, key) = example(Life::Occasional, 0);
        let day_start_ms = 1_767_225_600_000;
        let first = day(seed, &key, day_start_ms, RATE);
        covers_the_day(&first, day_start_ms);
        assert_eq!(
            first,
            day(seed, &key, day_start_ms, RATE),
            "same tuple, same day"
        );
        assert_ne!(
            first,
            day(seed + 1, &key, day_start_ms, RATE),
            "another seed draws another day"
        );
        assert_ne!(
            first,
            day(seed, &key, day_start_ms + DAY_MS, RATE),
            "another day draws another day"
        );
    }

    /// The faults of thirty days, so an expectation about a rate is a measurement.
    fn faults_in_a_month(seed: u64, key: &str) -> usize {
        let day_start_ms = 1_767_225_600_000;
        (0..30)
            .map(|offset| outages(seed, key, day_start_ms + offset * DAY_MS, RATE).len())
            .sum()
    }

    #[test]
    fn every_life_keeps_its_promise() {
        let day_start_ms = 1_767_225_600_000;
        let (up, up_key) = example(Life::AlwaysUp, 0);
        assert_eq!(
            day(up, &up_key, day_start_ms, RATE),
            vec![Stretch {
                up: true,
                from_ms: day_start_ms,
                to_ms: day_start_ms + DAY_MS
            }],
            "a target that never fails never fails"
        );

        // A target that fails now and then holds the rate, and one that flaps holds about
        // eight times as many; both are close enough to their promise over a month.
        let (occasional, occasional_key) = example(Life::Occasional, 0);
        for offset in 0..30 {
            let start = day_start_ms + offset * DAY_MS;
            covers_the_day(&day(occasional, &occasional_key, start, RATE), start);
        }
        let occasional_faults = faults_in_a_month(occasional, &occasional_key);
        assert!(
            (30..=150).contains(&occasional_faults),
            "an occasional target should hold about {RATE} faults a day, got {occasional_faults} a month"
        );

        let (flapping, flapping_key) = example(Life::Flapping, 0);
        for offset in 0..30 {
            let start = day_start_ms + offset * DAY_MS;
            covers_the_day(&day(flapping, &flapping_key, start, RATE), start);
        }
        let flapping_faults = faults_in_a_month(flapping, &flapping_key);
        assert!(
            flapping_faults > occasional_faults * 2,
            "a flapping target must be far busier: {flapping_faults} against {occasional_faults}"
        );
    }

    /// The rate is the knob: nothing goes down at zero, and the faults grow with it.
    #[test]
    fn the_rate_moves_the_faults_and_zero_means_a_quiet_board() {
        let day_start_ms = 1_767_225_600_000;
        for life in [Life::Occasional, Life::Flapping] {
            let (seed, key) = example(life, 0);
            for offset in 0..5 {
                let start = day_start_ms + offset * DAY_MS;
                let quiet = day(seed, &key, start, 0.0);
                covers_the_day(&quiet, start);
                assert_eq!(quiet.len(), 1, "a quiet board has one stretch a day");
                assert!(quiet[0].up, "and it is up");
            }

            let count = |rate: f64| {
                (0..30)
                    .map(|offset| outages(seed, &key, day_start_ms + offset * DAY_MS, rate).len())
                    .sum::<usize>()
            };
            let (low, high) = (count(1.0), count(10.0));
            assert!(
                high > low,
                "a higher rate must draw more faults: {high} against {low}"
            );
        }
    }

    #[test]
    fn a_fault_that_runs_past_midnight_is_one_outage() {
        let day_start_ms = 1_767_225_600_000;
        let mut crossing = None;
        for candidate in 0..20_000 {
            let (seed, key) = example(Life::Flapping, candidate);
            if let Some(last) = outages(seed, &key, day_start_ms, RATE).last() {
                if last.to_ms > day_start_ms + DAY_MS {
                    crossing = Some((seed, key, *last));
                    break;
                }
            }
        }
        let (seed, key, last) = crossing.expect("a fault past midnight");

        let next = day(seed, &key, day_start_ms + DAY_MS, RATE);
        covers_the_day(&next, day_start_ms + DAY_MS);
        assert!(!next[0].up, "the day opens inside the fault");
        assert_eq!(
            next[0].to_ms, last.to_ms,
            "it ends when the fault of the day before ends"
        );
    }
}

#[cfg(test)]
mod probe_sim {
    use std::net::Ipv4Addr;

    use crate::probe::{
        simulated_day, Endpoint, RecordType, Scheme, SimulatedEpisode, SimulatedState, StatusMatch,
    };

    const DAY_START_MS: i64 = 1_767_225_600_000;
    const RATE: f64 = 3.0;

    fn endpoints() -> Vec<Endpoint> {
        vec![
            Endpoint::Icmp {
                address: Ipv4Addr::new(1, 1, 1, 1),
            },
            Endpoint::Tcp {
                host: "example.com".to_string(),
                port: 443,
            },
            Endpoint::Dns {
                name: "example.com".to_string(),
                resolver: Some("9.9.9.9".parse().expect("an address")),
                record: RecordType::A,
            },
            Endpoint::Http {
                scheme: Scheme::Https,
                host: "example.com".to_string(),
                port: 443,
                target: "/health".to_string(),
                expect: StatusMatch::default(),
            },
        ]
    }

    fn covers(stretches: &[SimulatedEpisode], day_start_ms: i64) {
        assert!(!stretches.is_empty(), "a day is never empty");
        assert_eq!(stretches[0].from_ms, day_start_ms);
        assert_eq!(
            stretches.last().expect("an episode").to_ms,
            day_start_ms + super::DAY_MS
        );
        for pair in stretches.windows(2) {
            assert_eq!(pair[0].to_ms, pair[1].from_ms, "no gap and no overlap");
            assert_ne!(pair[0].state, pair[1].state, "INV-1: the state changes");
        }
    }

    #[test]
    fn the_same_tuple_replays_the_same_day_and_another_seed_draws_another() {
        for endpoint in endpoints() {
            let first = simulated_day(&endpoint, 5, DAY_START_MS, RATE);
            covers(&first, DAY_START_MS);
            assert_eq!(
                first,
                simulated_day(&endpoint, 5, DAY_START_MS, RATE),
                "{endpoint} is not reproducible"
            );
            let other = simulated_day(&endpoint, 6, DAY_START_MS, RATE);
            assert_ne!(first, other, "{endpoint} ignores the seed");
            let later = simulated_day(&endpoint, 5, DAY_START_MS + super::DAY_MS, RATE);
            assert_ne!(first, later, "{endpoint} ignores the day");
        }
    }

    #[test]
    fn a_fault_states_a_cause_of_its_own_method_and_an_answer_states_its_latency() {
        for endpoint in endpoints() {
            let prefix = format!("{}.", endpoint.method().as_str().to_uppercase());
            let mut faults = 0;
            for day in 0..40u64 {
                for episode in simulated_day(
                    &endpoint,
                    3 + day,
                    DAY_START_MS + day as i64 * super::DAY_MS,
                    RATE,
                ) {
                    match episode.state {
                        SimulatedState::Up => {
                            assert!(episode.cause.is_none(), "an answer states no cause");
                            assert!(episode.rtt_us.is_some_and(|rtt| rtt > 0));
                        }
                        SimulatedState::Down => {
                            faults += 1;
                            assert!(episode.rtt_us.is_none(), "a fault states no latency");
                            let cause = episode.cause.expect("a fault states a cause");
                            assert!(
                                cause.code().starts_with(&prefix),
                                "{endpoint} stated {}",
                                cause.code()
                            );
                            assert!(!cause.message().contains("unknown"), "{}", cause.message());
                        }
                    }
                }
            }
            assert!(faults > 0, "{endpoint} never failed in forty days");
        }
    }

    #[test]
    fn a_fault_that_crosses_midnight_is_continuous_across_the_two_days() {
        for endpoint in endpoints() {
            for day in 0..60 {
                let start = DAY_START_MS + day * super::DAY_MS;
                let tonight = simulated_day(&endpoint, 9, start, RATE);
                let Some(last) = tonight.last() else {
                    continue;
                };
                let tomorrow = simulated_day(&endpoint, 9, start + super::DAY_MS, RATE);
                covers(&tomorrow, start + super::DAY_MS);
                if !last.state.eq(&SimulatedState::Down) || last.to_ms <= start + super::DAY_MS {
                    continue;
                }
                assert_eq!(
                    tomorrow[0].state,
                    SimulatedState::Down,
                    "tomorrow opens inside the fault"
                );
                assert_eq!(
                    tonight.last().map(|episode| episode.to_ms),
                    Some(tomorrow[0].to_ms),
                    "the fault ends where tonight said it would"
                );
                return;
            }
        }
    }
}
