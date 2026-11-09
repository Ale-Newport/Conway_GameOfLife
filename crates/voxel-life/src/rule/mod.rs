//! Three-dimensional outer-totalistic rules with Generations-style decay.
//!
//! A [`Rule3D`] is the canonical form of every notation the crate accepts
//! (Softology `S/B/C/N`, letters `B…/S…`, Bays `E_l E_u F_l F_u` and Golly
//! `3D…`). Its semantics are those of Golly's Generations algorithm, which is
//! also what Softology's *Visions of Chaos* executes:
//!
//! ```text
//! n = number of neighbours in state 1
//! state 0      → 1 if n ∈ B, else 0
//! state 1      → 1 if n ∈ S, else (2 if C > 2, else 0)
//! state k ≥ 2  → k + 1 if k + 1 < C, else 0
//! ```

mod parse;
#[cfg(test)]
mod tests;

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::RuleError;

/// Largest neighbour count of any supported neighbourhood.
pub const MAX_COUNT: u32 = 26;

/// Neighbourhood of a voxel on the cubic lattice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Neighbourhood {
    /// The 26 voxels at Chebyshev distance 1 (Softology `M`).
    #[default]
    Moore,
    /// The 6 face-adjacent voxels (Softology `N`, Golly suffix `F`/`V`).
    VonNeumann,
}

const MOORE_OFFSETS: [[i32; 3]; 26] = {
    let mut out = [[0; 3]; 26];
    let mut i = 0;
    let mut n = 0;
    while n < 27 {
        let offset = [n % 3 - 1, (n / 3) % 3 - 1, n / 9 - 1];
        if !(offset[0] == 0 && offset[1] == 0 && offset[2] == 0) {
            out[i] = offset;
            i += 1;
        }
        n += 1;
    }
    out
};

const VON_NEUMANN_OFFSETS: [[i32; 3]; 6] = [[-1, 0, 0], [1, 0, 0], [0, -1, 0], [0, 1, 0], [0, 0, -1], [0, 0, 1]];

impl Neighbourhood {
    /// Number of neighbours.
    pub fn size(self) -> u32 {
        match self {
            Self::Moore => 26,
            Self::VonNeumann => 6,
        }
    }

    /// Neighbour offsets `[dx, dy, dz]`.
    pub fn offsets(self) -> &'static [[i32; 3]] {
        match self {
            Self::Moore => &MOORE_OFFSETS,
            Self::VonNeumann => &VON_NEUMANN_OFFSETS,
        }
    }

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Moore => "Moore",
            Self::VonNeumann => "von Neumann",
        }
    }

    /// Softology neighbourhood letter (`M` or `N`).
    pub fn letter(self) -> char {
        match self {
            Self::Moore => 'M',
            Self::VonNeumann => 'N',
        }
    }
}

/// A set of neighbour counts `0..=26`, stored as a bit mask (bit `n` = count `n`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CountSet(u32);

impl CountSet {
    /// The empty set.
    pub const EMPTY: CountSet = CountSet(0);

    /// Set of the counts `lo..=hi` (empty when `lo > hi`; counts above 26 are ignored).
    pub fn range(lo: u32, hi: u32) -> Self {
        (lo..=hi.min(MAX_COUNT)).collect()
    }

    /// Raw mask (bit `n` = count `n`).
    pub fn bits(self) -> u32 {
        self.0
    }

    /// Whether `count` is in the set.
    #[inline]
    pub fn contains(self, count: u32) -> bool {
        count <= MAX_COUNT && self.0 >> count & 1 == 1
    }

    /// Whether the set is empty.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Largest count in the set.
    pub fn max(self) -> Option<u32> {
        (self.0 != 0).then(|| 31 - self.0.leading_zeros())
    }

    /// Counts in increasing order.
    pub fn iter(self) -> impl Iterator<Item = u32> {
        (0..=MAX_COUNT).filter(move |&n| self.contains(n))
    }

    /// Maximal runs of consecutive counts as inclusive `(lo, hi)` pairs.
    pub fn runs(self) -> Vec<(u32, u32)> {
        let mut runs: Vec<(u32, u32)> = Vec::new();
        for n in self.iter() {
            match runs.last_mut() {
                Some((_, hi)) if *hi + 1 == n => *hi = n,
                _ => runs.push((n, n)),
            }
        }
        runs
    }

    /// The set as a single interval `(lo, hi)`, if it is one.
    pub fn as_interval(self) -> Option<(u32, u32)> {
        match self.runs().as_slice() {
            [single] => Some(*single),
            _ => None,
        }
    }

    /// Softology list: singletons as `a`, runs as `a-b`, comma separated.
    pub fn to_softology(self) -> String {
        let items: Vec<String> =
            self.runs().iter().map(|&(lo, hi)| if lo == hi { lo.to_string() } else { format!("{lo}-{hi}") }).collect();
        items.join(",")
    }

    /// Golly 3D.lua list: runs of exactly two as `a,b`, longer runs as `a..b`.
    pub fn to_golly(self) -> String {
        let mut items: Vec<String> = Vec::new();
        for (lo, hi) in self.runs() {
            match hi - lo {
                0 => items.push(lo.to_string()),
                1 => items.extend([lo.to_string(), hi.to_string()]),
                _ => items.push(format!("{lo}..{hi}")),
            }
        }
        items.join(",")
    }

    /// Letter-notation list: concatenated digits when every count is below
    /// 10 (`45`), otherwise the Softology list. A lone count of 10 or more is
    /// written `a-a` so it cannot be read back as separate digits.
    pub fn to_letters(self) -> String {
        if self.max().is_none_or(|m| m < 10) {
            return self.iter().map(|n| n.to_string()).collect();
        }
        match self.runs().as_slice() {
            [(lo, hi)] if lo == hi => format!("{lo}-{lo}"),
            _ => self.to_softology(),
        }
    }
}

impl FromIterator<u32> for CountSet {
    fn from_iter<T: IntoIterator<Item = u32>>(iter: T) -> Self {
        CountSet(iter.into_iter().filter(|&n| n <= MAX_COUNT).fold(0, |mask, n| mask | 1 << n))
    }
}

/// A three-dimensional outer-totalistic rule with `states` states.
///
/// Invariants (checked by [`Rule3D::new`]): `2 ≤ states ≤ 256`, no count
/// exceeds the neighbourhood size, and 0 is not a birth count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Rule3D {
    survive: CountSet,
    birth: CountSet,
    states: u16,
    neighbourhood: Neighbourhood,
}

impl Rule3D {
    /// Builds and validates a rule.
    pub fn new(
        survive: CountSet,
        birth: CountSet,
        states: u16,
        neighbourhood: Neighbourhood,
    ) -> Result<Self, RuleError> {
        if !(2..=256).contains(&states) {
            return Err(RuleError::States(states.to_string()));
        }
        for (field, set) in [("survival list", survive), ("birth list", birth)] {
            if let Some(max) = set.max()
                && max > neighbourhood.size()
            {
                return Err(RuleError::CountOutOfRange {
                    field,
                    count: max,
                    size: neighbourhood.size(),
                    neighbourhood: neighbourhood.label(),
                });
            }
        }
        if birth.contains(0) {
            return Err(RuleError::BirthOnZero);
        }
        Ok(Self { survive, birth, states, neighbourhood })
    }

    /// Parses any supported notation; see the crate documentation.
    pub fn parse(text: &str) -> Result<Self, RuleError> {
        parse::parse(text)
    }

    /// Bays' Life 4555 (`B5/S45`).
    pub fn bays_4555() -> Self {
        Self {
            survive: CountSet::range(4, 5),
            birth: CountSet::range(5, 5),
            states: 2,
            neighbourhood: Neighbourhood::Moore,
        }
    }

    /// Bays' Life 5766 (`B6/S567`), Golly's default 3D rule.
    pub fn bays_5766() -> Self {
        Self {
            survive: CountSet::range(5, 7),
            birth: CountSet::range(6, 6),
            states: 2,
            neighbourhood: Neighbourhood::Moore,
        }
    }

    /// Survival counts.
    pub fn survive(&self) -> CountSet {
        self.survive
    }

    /// Birth counts.
    pub fn birth(&self) -> CountSet {
        self.birth
    }

    /// Number of states `C` (2 = two-state rule).
    pub fn states(&self) -> u16 {
        self.states
    }

    /// Neighbourhood.
    pub fn neighbourhood(&self) -> Neighbourhood {
        self.neighbourhood
    }

    /// Next state of a voxel in state `state` with `alive_neighbours`
    /// neighbours in state 1.
    pub fn next_state(&self, state: u8, alive_neighbours: u32) -> u8 {
        match state {
            0 => u8::from(self.birth.contains(alive_neighbours)),
            1 if self.survive.contains(alive_neighbours) => 1,
            1 => u8::from(self.states > 2) * 2,
            s if u16::from(s) + 1 < self.states => s + 1,
            _ => 0,
        }
    }

    /// Lookup tables used by the engines.
    pub(crate) fn transition(&self) -> Transition {
        let mut from_empty = [0u8; 27];
        let mut from_alive = [0u8; 27];
        for n in 0..27u32 {
            from_empty[n as usize] = self.next_state(0, n);
            from_alive[n as usize] = self.next_state(1, n);
        }
        Transition { from_empty, from_alive, states: self.states }
    }

    /// Canonical Softology form `S/B/C/N`, e.g. `13-26/13-14,17-19/2/M`.
    pub fn to_softology(&self) -> String {
        format!(
            "{}/{}/{}/{}",
            self.survive.to_softology(),
            self.birth.to_softology(),
            self.states,
            self.neighbourhood.letter()
        )
    }

    /// Letter form `B…/S…`, followed by `/C<states>` for multi-state rules and
    /// `/N` for the von Neumann neighbourhood, e.g. `B5/S45`.
    pub fn to_letters(&self) -> String {
        let mut out = format!("B{}/S{}", self.birth.to_letters(), self.survive.to_letters());
        if self.states > 2 {
            out.push_str(&format!("/C{}", self.states));
        }
        if self.neighbourhood == Neighbourhood::VonNeumann {
            out.push_str("/N");
        }
        out
    }

    /// Bays' notation `E_l E_u F_l F_u` (`5766`, or `(10 21 10 21)` when a
    /// bound has two digits). Only two-state Moore rules whose survival and
    /// birth sets are single intervals have one.
    pub fn to_bays(&self) -> Option<String> {
        if self.states != 2 || self.neighbourhood != Neighbourhood::Moore {
            return None;
        }
        let (el, eu) = self.survive.as_interval()?;
        let (fl, fu) = self.birth.as_interval()?;
        Some(if [el, eu, fl, fu].iter().all(|&b| b < 10) {
            format!("{el}{eu}{fl}{fu}")
        } else {
            format!("({el} {eu} {fl} {fu})")
        })
    }

    /// Golly 3D.lua form `3D<S>/<B>` with suffix `F` for von Neumann. Golly's
    /// 3D rules have two states, so multi-state rules have none.
    pub fn to_golly(&self) -> Option<String> {
        (self.states == 2).then(|| {
            let suffix = if self.neighbourhood == Neighbourhood::VonNeumann { "F" } else { "" };
            format!("3D{}/{}{suffix}", self.survive.to_golly(), self.birth.to_golly())
        })
    }
}

impl fmt::Display for Rule3D {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_softology())
    }
}

impl FromStr for Rule3D {
    type Err = RuleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl TryFrom<String> for Rule3D {
    type Error = RuleError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value)
    }
}

impl From<Rule3D> for String {
    fn from(rule: Rule3D) -> Self {
        rule.to_softology()
    }
}

/// Precomputed transition tables of a [`Rule3D`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct Transition {
    from_empty: [u8; 27],
    from_alive: [u8; 27],
    states: u16,
}

impl Transition {
    /// Next state for `state` with `alive` (≤ 26) neighbours in state 1.
    #[inline]
    pub(crate) fn apply(&self, state: u8, alive: u8) -> u8 {
        match state {
            0 => self.from_empty[usize::from(alive)],
            1 => self.from_alive[usize::from(alive)],
            s if u16::from(s) + 1 < self.states => s + 1,
            _ => 0,
        }
    }
}
