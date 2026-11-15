//! The deterministic random source of the simulation.
//!
//! The simulated mode keeps no history: it replays it. Every draw therefore has to be a
//! pure function of the tuple the caller is asking about — `(seed, target key, UTC day,
//! purpose)` — so the same command shows the same history twice, and so a page that asks
//! for day 12 does not have to generate days 1 to 11 first.
//!
//! That rules out a stateful generator advanced in a loop across days, and it rules out a
//! dependency on a crate: [`Rng`] is `splitmix64`, a dozen lines of arithmetic, and the
//! seeding is an FNV-1a mix of the tuple. Both live here, in one leaf module, so nothing
//! else in the simulation has to think about reproducibility.

/// A `splitmix64` generator: a 64-bit state, one mix per draw.
///
/// It is not cryptographic and does not need to be. What it must do is return the same
/// stream for the same seed on every platform and every run, which pure integer arithmetic
/// guarantees.
#[derive(Debug, Clone)]
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng { state: seed }
    }

    /// The stream of one tuple: `(seed, target key, day, purpose)`.
    ///
    /// A caller never has to remember an order of calls or share one generator: the same
    /// tuple always replays the same stream.
    pub fn for_draw(seed: u64, key: &str, day_start_ms: i64, purpose: &str) -> Rng {
        Rng::new(seed)
            .then(day_start_ms)
            .then_str(key)
            .then_str(purpose)
    }

    /// Mix one instant into the seed.
    pub fn then(self, value: i64) -> Rng {
        Rng::new(mix(self.state, value as u64))
    }

    /// Mix one name into the seed.
    pub fn then_str(self, value: &str) -> Rng {
        let mut hash = self.state ^ 0xcbf2_9ce4_8422_2325;
        for byte in value.as_bytes() {
            hash = (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        Rng::new(finish(hash))
    }

    /// The next value of the stream.
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        finish(self.state)
    }

    /// A value in `[0, bound)`; `0` when the bound is empty.
    pub fn below(&mut self, bound: u64) -> u64 {
        if bound == 0 {
            return 0;
        }
        self.next_u64() % bound
    }

    /// A value in `[low, high)`; `low` when the range is empty.
    pub fn between(&mut self, low: i64, high: i64) -> i64 {
        if high <= low {
            return low;
        }
        low + self.below((high - low) as u64) as i64
    }
}

/// Mix one number into a seed.
fn mix(seed: u64, value: u64) -> u64 {
    finish(seed ^ value.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// The `splitmix64` finalizer: it avalanches every input bit across the output.
fn finish(value: u64) -> u64 {
    let mut z = value;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_tuple_replays_the_same_stream() {
        let mut first = Rng::for_draw(7, "icmp://1.1.1.1", 86_400_000, "faults");
        let mut second = Rng::for_draw(7, "icmp://1.1.1.1", 86_400_000, "faults");
        for _ in 0..100 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn a_different_part_of_the_tuple_draws_differently() {
        let draw = |seed, key: &str, day, purpose: &str| {
            let mut rng = Rng::for_draw(seed, key, day, purpose);
            (0..8).map(|_| rng.next_u64()).collect::<Vec<_>>()
        };
        let reference = draw(7, "icmp://1.1.1.1", 86_400_000, "faults");
        assert_ne!(reference, draw(8, "icmp://1.1.1.1", 86_400_000, "faults"));
        assert_ne!(reference, draw(7, "icmp://8.8.8.8", 86_400_000, "faults"));
        assert_ne!(reference, draw(7, "icmp://1.1.1.1", 172_800_000, "faults"));
        assert_ne!(reference, draw(7, "icmp://1.1.1.1", 86_400_000, "life"));
    }

    #[test]
    fn a_range_stays_inside_its_bounds() {
        let mut rng = Rng::new(11);
        for _ in 0..1_000 {
            let value = rng.between(20_000, 2_400_000);
            assert!((20_000..2_400_000).contains(&value), "got {value}");
            assert!(rng.below(4) < 4);
        }
        assert_eq!(rng.between(5, 5), 5, "an empty range is its lower bound");
        assert_eq!(rng.between(9, 2), 9, "a reversed range is its lower bound");
        assert_eq!(rng.below(0), 0, "an empty bound is zero");
    }
}
