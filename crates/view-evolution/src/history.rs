//! Gene statistics over time, recorded from the published views.
//!
//! The shell's metric store has the prey gene means and SDs per tick, but
//! not the predators' genes or the allele-class frequencies. This history
//! records one sample per published view (only when the tick advanced) and
//! keeps memory bounded by halving its resolution when full: after a
//! decimation it holds every other sample, so the time axis stays complete.

use evolution::genes::HERITABLE_COUNT;
use evolution::stats::SpeciesStatistics;
use evolution::{EvolutionView, Species};

/// Samples kept before the history halves its resolution.
pub const MAX_SAMPLES: usize = 2_000;

/// Statistics of one species at one tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SpeciesSample {
    /// Living organisms.
    pub count: u32,
    /// Mean phenotype per heritable gene.
    pub mean: [f32; HERITABLE_COUNT],
    /// Standard deviation per heritable gene.
    pub sd: [f32; HERITABLE_COUNT],
    /// Allele-like classes per gene (`k`).
    pub classes: u8,
    /// Class frequencies, `HERITABLE_COUNT × k`, gene-major.
    pub class_freq: Vec<f32>,
}

impl SpeciesSample {
    fn from_stats(stats: &SpeciesStatistics) -> Self {
        let mut sample = Self { count: stats.count, ..Self::default() };
        if stats.count == 0 {
            return sample;
        }
        let classes = stats.genes.first().map_or(0, |g| g.class_frequencies.len()).min(255);
        sample.classes = classes as u8;
        for (i, g) in stats.genes.iter().take(HERITABLE_COUNT).enumerate() {
            sample.mean[i] = g.mean as f32;
            sample.sd[i] = g.sd as f32;
            let mut freq: Vec<f32> = g.class_frequencies.iter().map(|f| *f as f32).collect();
            freq.resize(classes, 0.0);
            sample.class_freq.extend(freq);
        }
        sample
    }

    /// Class frequencies of gene `gene`.
    pub fn classes_of(&self, gene: usize) -> &[f32] {
        let k = usize::from(self.classes);
        self.class_freq.get(gene * k..(gene + 1) * k).unwrap_or(&[])
    }
}

/// One recorded tick.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GeneSample {
    /// Tick.
    pub tick: u64,
    /// Prey statistics.
    pub prey: SpeciesSample,
    /// Predator statistics.
    pub predators: SpeciesSample,
}

impl GeneSample {
    /// Statistics of `species`.
    pub fn of(&self, species: Species) -> &SpeciesSample {
        match species {
            Species::Prey => &self.prey,
            Species::Predator => &self.predators,
        }
    }
}

/// Decimating history of gene statistics.
#[derive(Clone, Debug, Default)]
pub struct GeneHistory {
    samples: Vec<GeneSample>,
    capacity: usize,
}

/// One point of a mean ± SD band.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BandPoint {
    /// Tick.
    pub tick: f64,
    /// Mean.
    pub mean: f64,
    /// Mean − SD.
    pub low: f64,
    /// Mean + SD.
    pub high: f64,
}

impl GeneHistory {
    /// An empty history keeping at most `capacity` samples (≥ 4).
    pub fn with_capacity(capacity: usize) -> Self {
        Self { samples: Vec::new(), capacity: capacity.max(4) }
    }

    fn cap(&self) -> usize {
        if self.capacity == 0 { MAX_SAMPLES } else { self.capacity }
    }

    /// Recorded samples, oldest first.
    pub fn samples(&self) -> &[GeneSample] {
        &self.samples
    }

    /// Forgets everything.
    pub fn clear(&mut self) {
        self.samples.clear();
    }

    /// Drops samples after `tick` (the run went back in time).
    pub fn truncate_after(&mut self, tick: u64) {
        self.samples.retain(|s| s.tick <= tick);
    }

    /// Records the statistics of a view if its tick is new.
    pub fn record(&mut self, view: &EvolutionView) {
        let sample = GeneSample {
            tick: view.tick,
            prey: SpeciesSample::from_stats(&view.stats.prey),
            predators: SpeciesSample::from_stats(&view.stats.predators),
        };
        self.push(sample);
    }

    /// Appends a sample (ignored unless its tick is later than the last
    /// one), halving the resolution when full.
    pub fn push(&mut self, sample: GeneSample) {
        if let Some(last) = self.samples.last() {
            if sample.tick < last.tick {
                self.truncate_after(sample.tick);
            } else if sample.tick == last.tick {
                // Same tick re-published (e.g. after a command): keep the latest numbers.
                if let Some(last) = self.samples.last_mut() {
                    *last = sample;
                }
                return;
            }
        }
        self.samples.push(sample);
        if self.samples.len() > self.cap() {
            // Keep the first sample (the founders) and every other one after it.
            let mut i = 0;
            self.samples.retain(|_| {
                let keep = i % 2 == 0;
                i += 1;
                keep
            });
        }
    }

    /// First recorded sample with living organisms of `species` (the
    /// founders, when recording started at tick 0).
    pub fn first_of(&self, species: Species) -> Option<&GeneSample> {
        self.samples.iter().find(|s| s.of(species).count > 0)
    }

    /// Mean ± SD band of `gene` for `species` (ticks without organisms are skipped).
    pub fn band(&self, species: Species, gene: usize) -> Vec<BandPoint> {
        if gene >= HERITABLE_COUNT {
            return Vec::new();
        }
        self.samples
            .iter()
            .filter(|s| s.of(species).count > 0)
            .map(|s| {
                let st = s.of(species);
                let (m, sd) = (f64::from(st.mean[gene]), f64::from(st.sd[gene]));
                BandPoint { tick: s.tick as f64, mean: m, low: m - sd, high: m + sd }
            })
            .collect()
    }

    /// Cumulative class frequencies over time for `gene`: one series per
    /// class boundary (`k + 1` series from 0 to 1), using only samples with
    /// the latest class count. Returns `(k, ticks, cumulative[class][sample])`.
    pub fn stacked_classes(&self, species: Species, gene: usize) -> (usize, Vec<f64>, Vec<Vec<f64>>) {
        let Some(k) = self.samples.iter().rev().map(|s| s.of(species)).find(|s| s.count > 0).map(|s| s.classes) else {
            return (0, Vec::new(), Vec::new());
        };
        let k = usize::from(k);
        let used: Vec<&GeneSample> = self
            .samples
            .iter()
            .filter(|s| s.of(species).count > 0 && usize::from(s.of(species).classes) == k)
            .collect();
        let ticks = used.iter().map(|s| s.tick as f64).collect();
        let mut cumulative = vec![Vec::with_capacity(used.len()); k + 1];
        for s in &used {
            let freq = s.of(species).classes_of(gene);
            let mut acc = 0.0;
            cumulative[0].push(0.0);
            for c in 0..k {
                acc += f64::from(freq.get(c).copied().unwrap_or(0.0));
                cumulative[c + 1].push(acc.min(1.0));
            }
        }
        (k, ticks, cumulative)
    }
}

/// Mean ± SD band from a per-tick mean series of `len` samples (`mean_at(i)`
/// = `(tick, mean)` in tick order, as recorded by the shell's metric store)
/// and the SD recorded at a tick (`sd_at`), thinned to at most about
/// `max_points` evenly spaced samples (the last sample is always kept).
/// Only the kept samples are read, so the cost does not grow with the
/// length of the run.
pub fn thin_band(
    len: usize,
    mean_at: impl Fn(usize) -> (u64, f64),
    sd_at: impl Fn(u64) -> Option<f64>,
    max_points: usize,
) -> Vec<BandPoint> {
    if len == 0 {
        return Vec::new();
    }
    let stride = len.div_ceil(max_points.max(2)).max(1);
    let indices = (0..len).step_by(stride).chain((!(len - 1).is_multiple_of(stride)).then_some(len - 1));
    indices
        .filter_map(|i| {
            let (tick, m) = mean_at(i);
            let s = sd_at(tick)?;
            (m.is_finite() && s.is_finite()).then_some(BandPoint {
                tick: tick as f64,
                mean: m,
                low: m - s,
                high: m + s,
            })
        })
        .collect()
}

/// Change of the mean since the founders, in units of the founders' SD
/// (`None` without founders or variation).
pub fn response_in_sd(founder_mean: f32, founder_sd: f32, mean: f32) -> Option<f32> {
    (founder_sd > 1e-6).then(|| (mean - founder_mean) / founder_sd)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(tick: u64, mean0: f32) -> GeneSample {
        let mut prey = SpeciesSample { count: 10, classes: 2, ..Default::default() };
        prey.mean[0] = mean0;
        prey.sd[0] = 0.5;
        prey.class_freq = (0..HERITABLE_COUNT).flat_map(|_| [0.25, 0.75]).collect();
        GeneSample { tick, prey, predators: SpeciesSample::default() }
    }

    #[test]
    fn records_only_new_ticks_and_rewinds() {
        let mut h = GeneHistory::with_capacity(100);
        h.push(sample(0, 1.0));
        h.push(sample(5, 2.0));
        h.push(sample(5, 3.0));
        assert_eq!(h.samples().len(), 2);
        assert_eq!(h.samples()[1].prey.mean[0], 3.0, "a re-published tick replaces the sample");
        h.push(sample(10, 4.0));
        h.push(sample(7, 9.0));
        assert_eq!(h.samples().iter().map(|s| s.tick).collect::<Vec<_>>(), vec![0, 5, 7], "rewind truncates");
    }

    #[test]
    fn decimation_halves_resolution_and_keeps_the_founders() {
        let mut h = GeneHistory::with_capacity(8);
        for t in 0..9 {
            h.push(sample(t, t as f32));
        }
        let ticks: Vec<u64> = h.samples().iter().map(|s| s.tick).collect();
        assert_eq!(ticks, vec![0, 2, 4, 6, 8]);
        for t in 9..40 {
            h.push(sample(t, t as f32));
        }
        assert!(h.samples().len() <= 8);
        assert_eq!(h.samples()[0].tick, 0, "founders stay");
        assert_eq!(h.samples().last().unwrap().tick, 39, "latest stays");
    }

    #[test]
    fn band_and_stacked_classes() {
        let mut h = GeneHistory::with_capacity(100);
        h.push(sample(0, 1.0));
        h.push(sample(1, 2.0));
        let band = h.band(Species::Prey, 0);
        assert_eq!(band.len(), 2);
        assert_eq!((band[1].low, band[1].mean, band[1].high), (1.5, 2.0, 2.5));
        assert!(h.band(Species::Predator, 0).is_empty(), "no predators recorded");
        let (k, ticks, cum) = h.stacked_classes(Species::Prey, 3);
        assert_eq!((k, ticks.len(), cum.len()), (2, 2, 3));
        assert_eq!(cum[1][0], 0.25);
        assert_eq!(cum[2][1], 1.0);
        assert_eq!(h.first_of(Species::Prey).map(|s| s.tick), Some(0));
    }

    #[test]
    fn bands_join_series_by_tick_and_thin_them() {
        let mean: Vec<(u64, f64)> = (0..100).map(|t| (t, t as f64)).collect();
        let sd = |t: u64| (t != 10).then_some(1.0);
        let band = thin_band(mean.len(), |i| mean[i], sd, 1000);
        assert_eq!(band.len(), 99, "tick 10 has no sd and is skipped");
        assert_eq!((band[0].low, band[0].high), (-1.0, 1.0));
        let thin = thin_band(mean.len(), |i| mean[i], sd, 10);
        assert!(thin.len() <= 11);
        assert_eq!(thin.last().unwrap().tick, 99.0, "latest kept");
        let ticks: Vec<f64> = thin.iter().map(|b| b.tick).collect();
        assert!(ticks.windows(2).all(|w| w[0] < w[1]), "no duplicate last sample");
        assert!(thin_band(0, |i| mean[i], sd, 10).is_empty());
        let exact = thin_band(10, |i| mean[i], sd, 5);
        assert_eq!(exact.iter().map(|b| b.tick).collect::<Vec<_>>(), vec![0.0, 2.0, 4.0, 6.0, 8.0, 9.0]);
    }

    #[test]
    fn response_is_measured_in_founder_sds() {
        assert_eq!(response_in_sd(1.0, 0.5, 2.0), Some(2.0));
        assert_eq!(response_in_sd(1.0, 0.0, 2.0), None);
    }
}
