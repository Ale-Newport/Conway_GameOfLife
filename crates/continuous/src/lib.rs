//! # continuous — Continuous Artificial Life
//!
//! Continuous-state cellular automata on a periodic grid, implemented from
//! the primary sources summarised in `docs/research/continuous-life.md`:
//!
//! * **Lenia** (Chan 2019) and **Expanded Lenia** (Chan 2020): kernel shells
//!   with rings `β` and exponential / polynomial / rectangular / staircase /
//!   Gaussian-bump cores normalised to `Σ K = 1`; Gaussian, polynomial and
//!   rectangular growth; `A ← clip(A + dt·G(K ∗ A), 0, 1)` with `dt = 1/T`;
//!   several channels and kernels with per-kernel `r, β, μ, σ, h` and
//!   source/destination channels ([`lenia`]).
//! * **SmoothLife** (Rafler 2011): anti-aliased disc and annulus fillings,
//!   sigmoid transition with two mixing variants, time-stepping modes 0–2
//!   ([`smoothlife`]).
//! * **Flow-Lenia** (Plantec et al. 2023): affinity, flow
//!   `F = (1−α)∇U − α∇A_Σ`, reintegration tracking with exact mass
//!   conservation ([`flow`]).
//!
//! Convolutions use a 2-D FFT built from `rustfft` row and column transforms
//! with precomputed kernel spectra and rayon over row blocks ([`fft`],
//! [`convolve`]); direct summation serves tiny kernels and as the test
//! oracle. All three models run behind one [`simulation_core::Simulation`]
//! implementation, [`ContinuousSim`], built from RON [`scenario`] files
//! (embedded copies of `scenarios/continuous/` are available through
//! [`scenario::builtin`]).
//!
//! Everything is deterministic: randomness comes from the
//! [`simulation_core::SimRng`] stored in the state, and parallel loops only
//! compute rows of separate output buffers, so results are bitwise identical
//! for any thread count (on one platform and build).

pub mod convolve;
pub mod edit;
pub mod error;
pub mod fft;
pub mod flow;
pub mod growth;
pub mod kernel;
pub mod lenia;
pub mod parallel;
pub mod rle;
pub mod rule;
pub mod scenario;
pub mod sim;
pub mod smoothlife;
pub mod stats;

pub use convolve::{ConvolutionMethod, ResolvedMethod};
pub use edit::{Blend, Brush, BrushMode, CellData, CreatureCells, Placement, Region};
pub use error::ContinuousError;
pub use flow::{FlowKernel, FlowLeniaRule};
pub use growth::GrowthFunction;
pub use kernel::{GaussianRing, KernelCore};
pub use lenia::{GrowthNormalization, LeniaKernel, LeniaRule};
pub use rule::{FLOW_EDIT_LIMIT, FLOW_LENIA_ID, LENIA_ID, ModelKind, Rule, SMOOTHLIFE_ID};
pub use scenario::{InitElement, LibraryCreature, Scenario, ScenarioHeader, SoupShape};
pub use sim::view::{ContinuousView, ContinuousViewRequest, ViewLayer};
pub use sim::{ContinuousCommand, ContinuousSim, SCHEMA_VERSION};
pub use smoothlife::{SmoothLifeMixing, SmoothLifeRule, SmoothLifeStepping};
pub use stats::ChannelStats;
