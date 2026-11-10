//! Deterministic random streams.
//!
//! Every random decision draws from a stream derived from the scenario seed,
//! a subsystem constant, the tick and the entity id, so results never depend
//! on iteration order, thread scheduling or how many other entities drew
//! (research note §7.7). The model additionally keeps a stateful
//! [`simulation_core::SimRng`] for commands and initial placement.

use simulation_core::SimRng;

/// Animal decisions (utility noise, wander jitter).
pub const DECIDE: u64 = 0xEC0_0001;
/// Attack outcomes.
pub const ATTACK: u64 = 0xEC0_0002;
/// Physiology (ageing deaths, litter sizes, offspring genomes and sexes).
pub const LIFE: u64 = 0xEC0_0003;
/// Trees (seeding, establishment, mortality).
pub const TREES: u64 = 0xEC0_0004;
/// Fire and wind damage.
pub const DISTURBANCE: u64 = 0xEC0_0005;
/// NetLogo rule variant (one stream per tick).
pub const NETLOGO: u64 = 0xEC0_0006;
/// Initial placement and commands (seeds the stateful generator).
pub const SETUP: u64 = 0xEC0_0007;
/// Colony foragers, laying and swarming.
pub const COLONY: u64 = 0xEC0_0008;
/// Disease transmission, recovery and death.
pub const DISEASE: u64 = 0xEC0_0009;

/// Stream for `(seed, subsystem, tick, id)`.
pub fn stream(seed: u64, subsystem: u64, tick: u64, id: u64) -> SimRng {
    let key = subsystem.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(29)
        ^ tick.wrapping_mul(0xD6E8_FEB8_6659_FD93)
        ^ id.wrapping_mul(0xA076_1D64_78BD_642F).rotate_left(17);
    SimRng::derive(seed, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn streams_are_distinct_and_reproducible() {
        let a = stream(1, DECIDE, 10, 5).next_u64();
        assert_eq!(a, stream(1, DECIDE, 10, 5).next_u64());
        assert_ne!(a, stream(1, DECIDE, 10, 6).next_u64());
        assert_ne!(a, stream(1, DECIDE, 11, 5).next_u64());
        assert_ne!(a, stream(1, ATTACK, 10, 5).next_u64());
        assert_ne!(a, stream(2, DECIDE, 10, 5).next_u64());
    }
}
