//! Command-line syntax (clap derive).

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Headless runner for Emergence Lab models: list models and scenarios, run
/// experiments, compare replicates, validate determinism, benchmark and
/// convert pattern files.
#[derive(Debug, Parser)]
#[command(name = "emergence-cli", version, about, long_about = None)]
#[command(after_help = "\
TARGETS
    A target is a model id (its default scenario, e.g. `life.classic`), a scenario id
    qualified by folder (`conway/glider-gun-showcase`) or bare when unique (`seeds`),
    or the path of a scenario file inside a folder named after its format
    (`my-scenarios/conway/foo.ron`).

DATA
    Assets:    --assets, else $EMERGENCE_LAB_ASSETS, else <exe dir>/assets, else the
               workspace assets/ folder this binary was built from.
    Scenarios: built-in scenarios of every model crate, then the workspace scenarios/
               folder, the user folder <data>/scenarios/ ($EMERGENCE_LAB_DATA or the
               platform location) and every --scenario-dir, later ones replacing
               scenarios with the same id.

EXIT STATUS
    0 success, 1 error, 2 invalid command line, 3 validation failed.")]
pub struct Cli {
    /// Asset folder (pattern catalog, OTCA metapixel tiles, species definitions).
    #[arg(long, global = true, value_name = "DIR")]
    pub assets: Option<PathBuf>,
    /// Extra scenario directory (layout of the repository's scenarios/ folder); repeatable.
    #[arg(long = "scenario-dir", global = true, value_name = "DIR")]
    pub scenario_dirs: Vec<PathBuf>,
    /// Ignore the workspace and user scenario folders (built-in scenarios and --scenario-dir only).
    #[arg(long, global = true)]
    pub builtin_only: bool,
    /// Subcommand.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// List models or scenarios.
    #[command(subcommand)]
    List(ListCommand),
    /// Describe a model or scenario: parameters with ranges and apply policies, metrics, rules.
    Info(InfoArgs),
    /// Run a scenario headless, print a final metrics table and export the metric series.
    Run(RunArgs),
    /// Run replicates over seeds and/or parameter values and summarise them.
    Compare(CompareArgs),
    /// Check determinism, snapshot round trips and model-specific properties.
    Validate(ValidateArgs),
    /// Measure ticks per second, time per tick and model memory.
    Bench(BenchArgs),
    /// Convert a pattern file between RLE, plaintext, Life 1.05/1.06 and macrocell.
    Convert(ConvertArgs),
}

/// `list` subcommands.
#[derive(Debug, Subcommand)]
pub enum ListCommand {
    /// Every model with its name, family, crate and scenario count.
    Models {
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Every scenario (built-in and files).
    Scenarios {
        /// Only scenarios of this model id.
        #[arg(long, value_name = "ID")]
        model: Option<String>,
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
}

/// `info` arguments.
#[derive(Debug, Args)]
pub struct InfoArgs {
    /// Model id or scenario id.
    pub target: String,
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

/// Options shared by commands that build a simulation.
#[derive(Debug, Args, Clone)]
pub struct BuildArgs {
    /// Seed replacing the scenario's.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Parameter override `key=value`, applied before the first tick (repeatable).
    #[arg(long = "param", value_name = "KEY=VALUE")]
    pub params: Vec<String>,
}

/// `run` arguments.
#[derive(Debug, Args)]
pub struct RunArgs {
    /// Model id, scenario id or scenario file.
    pub target: String,
    /// Ticks to run. One tick is one step of the model, as in the desktop app (Life:
    /// generations_per_tick generations; Hashlife: 2^step_exponent; metapixel: its `step`).
    #[arg(long, default_value_t = 100)]
    pub ticks: u64,
    /// Record metrics every K ticks (the initial and final states are always recorded;
    /// counter metrics hold the value of the sampled tick).
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
    pub metrics_every: u64,
    #[command(flatten)]
    pub build: BuildArgs,
    /// Export the metric series: `.csv` (with a commented header) or `.json`.
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// Save the final state as a snapshot file (`.elsnap` binary, or `.ron`/`.json`).
    #[arg(long, value_name = "FILE")]
    pub snapshot_out: Option<PathBuf>,
    /// Continue from a snapshot file of the same model; --param overrides are applied to the
    /// restored state (parameters that require a reset are refused).
    #[arg(long, value_name = "FILE")]
    pub resume: Option<PathBuf>,
    /// Print only the final table (no progress or run header).
    #[arg(long)]
    pub quiet: bool,
}

/// `compare` arguments.
#[derive(Debug, Args)]
pub struct CompareArgs {
    /// Model id, scenario id or scenario file.
    pub target: String,
    /// Seeds: `a..b` (b excluded), `a..=b` or a comma list `1,5,9`.
    #[arg(long, value_name = "SEEDS")]
    pub seeds: Option<String>,
    /// Parameter sweep `key=v1,v2,...` (repeatable; several sweeps form a grid).
    #[arg(long = "param", value_name = "KEY=V1,V2")]
    pub params: Vec<String>,
    /// Ticks per run (one tick is one step of the model, as for `run`).
    #[arg(long, default_value_t = 100)]
    pub ticks: u64,
    /// Record metrics every K ticks.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u64).range(1..))]
    pub metrics_every: u64,
    /// Summary file: `.csv` (one row per metric and run) or `.json` (comparison table).
    #[arg(long, value_name = "FILE")]
    pub out: Option<PathBuf>,
    /// Parallel runs (default: one per available core).
    #[arg(long, value_parser = clap::value_parser!(u64).range(1..))]
    pub jobs: Option<u64>,
    /// Only show these metrics in the printed table (repeatable; the file has every metric).
    #[arg(long = "metric", value_name = "KEY")]
    pub metrics: Vec<String>,
    /// Do not print progress.
    #[arg(long)]
    pub quiet: bool,
}

/// `validate` arguments.
#[derive(Debug, Args)]
pub struct ValidateArgs {
    /// Model or scenario ids (default: every model with its default scenario).
    pub targets: Vec<String>,
    /// Ticks per phase, each a single advance(1) call (one generation for Hashlife and the metapixel).
    #[arg(long, default_value_t = 20)]
    pub ticks: u64,
    /// Seed replacing the scenarios'.
    #[arg(long)]
    pub seed: Option<u64>,
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,
}

/// `bench` arguments.
#[derive(Debug, Args)]
pub struct BenchArgs {
    /// Model id, scenario id or scenario file.
    pub target: String,
    /// Measured ticks, each one step of the model timed on its own (as for `run`).
    #[arg(long, default_value_t = 200, value_parser = clap::value_parser!(u64).range(1..))]
    pub ticks: u64,
    /// Ticks run before measuring (not timed).
    #[arg(long, default_value_t = 20)]
    pub warmup: u64,
    #[command(flatten)]
    pub build: BuildArgs,
    /// Also write the results as JSON to this file.
    #[arg(long, value_name = "FILE")]
    pub json: Option<PathBuf>,
}

/// Pattern formats of `convert`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PatternFormat {
    /// Golly extended RLE (`.rle`).
    Rle,
    /// Plaintext (`.cells`).
    Plaintext,
    /// Life 1.05 (`.lif`).
    Life105,
    /// Life 1.06 (`.lif`).
    Life106,
    /// Golly macrocell (`.mc`), two-state Life-like rules without B0.
    Macrocell,
}

/// `convert` arguments.
#[derive(Debug, Args)]
pub struct ConvertArgs {
    /// Input pattern file (format detected from the extension and content).
    pub input: PathBuf,
    /// Output file.
    pub output: PathBuf,
    /// Output format (default: from the output extension; `.lif` means Life 1.06).
    #[arg(long, value_enum)]
    pub format: Option<PatternFormat>,
}
