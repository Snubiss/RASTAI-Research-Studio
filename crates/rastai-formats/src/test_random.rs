//! The seeded random numbers the unit tests share (the in-house generator of layout 4.11; no property-testing or
//! fuzzing crate in unit tests). Test builds only.

const MULTIPLIER: u64 = 6364136223846793005;
const INCREMENT: u64 = 1442695040888963407;
// An LCG's low bits repeat quickly, so only the high ones are returned.
const DISCARDED_LOW_BITS: u32 = 33;

/// A 64-bit linear congruential generator (Knuth's MMIX constants): the same seed gives the same numbers on every
/// machine, so a failing case reproduces.
pub(crate) fn next_random(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(MULTIPLIER).wrapping_add(INCREMENT);
    *seed >> DISCARDED_LOW_BITS
}
