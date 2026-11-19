//! Command-line options of the desktop application.

use std::path::PathBuf;

use interface::AppMode;
use thiserror::Error;

/// Usage text printed by `--help`.
pub const USAGE: &str = "\
Emergence Lab — a laboratory for artificial life

USAGE:
    emergence-lab [OPTIONS]

OPTIONS:
    --mode <NAME>               Open a laboratory directly (e.g. classic-conway, ecosystem,
                                continuous-life; see the list below)
    --smoke-screenshot <PNG>    Save a screenshot after a few frames and exit (automated checks)
    --smoke-frames <N>          Frames to render before that screenshot [default: 30]
    --smoke-sequence <N>        Save N numbered screenshots (<PNG stem>-0001.png, ...) instead of one,
                                for demo recordings
    --smoke-every <K>           Frames between sequence screenshots [default: 2]
    -h, --help                  Print this help
    -V, --version               Print the version

ENVIRONMENT:
    EMERGENCE_LAB_ASSETS        Asset folder (default: <exe dir>/assets, then the workspace assets)
    EMERGENCE_LAB_DATA          Folder for saves, exports and screenshots
";

/// Default number of frames before a smoke screenshot.
pub const DEFAULT_SMOKE_FRAMES: u32 = 30;

/// Default number of frames between the screenshots of a sequence.
pub const DEFAULT_SMOKE_EVERY: u32 = 2;

/// Largest accepted `--smoke-sequence` length.
pub const MAX_SMOKE_SEQUENCE: u32 = 10_000;

/// A numbered series of screenshots, used to record demo animations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sequence {
    /// Number of screenshots.
    pub count: u32,
    /// Frames between two screenshots.
    pub every: u32,
}

/// Automated visual check: screenshot after `frames` frames, then exit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmokeCapture {
    /// PNG to write.
    pub path: PathBuf,
    /// Frames to render first.
    pub frames: u32,
    /// Save a numbered series instead of one screenshot.
    pub sequence: Option<Sequence>,
}

/// Parsed options for a normal run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Mode to open at startup.
    pub mode: Option<AppMode>,
    /// Smoke screenshot request.
    pub smoke: Option<SmokeCapture>,
}

/// What the command line asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Start the application.
    Run(Options),
    /// Print usage.
    Help,
    /// Print the version.
    Version,
}

/// Command-line errors.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum CliError {
    /// Unknown argument.
    #[error("unknown argument `{0}` (see --help)")]
    Unknown(String),
    /// A flag without its value.
    #[error("`{0}` needs a value")]
    MissingValue(&'static str),
    /// The same flag twice.
    #[error("`{0}` was given more than once")]
    Repeated(&'static str),
    /// Unknown mode name.
    #[error("unknown mode `{name}`; valid names: {valid}")]
    UnknownMode {
        /// Name given.
        name: String,
        /// Accepted names.
        valid: String,
    },
    /// Frame count that is not a positive integer.
    #[error("`--smoke-frames` must be a whole number from 1 to 100000, got `{0}`")]
    InvalidFrames(String),
    /// `--smoke-frames` without `--smoke-screenshot`.
    #[error("`--smoke-frames` requires `--smoke-screenshot`")]
    FramesWithoutScreenshot,
    /// `--smoke-sequence` or `--smoke-every` with a value out of range.
    #[error("`{flag}` must be a whole number from 1 to {max}, got `{value}`")]
    InvalidSequence {
        /// The flag.
        flag: &'static str,
        /// Value given.
        value: String,
        /// Largest accepted value.
        max: u32,
    },
    /// `--smoke-sequence` / `--smoke-every` without what they need.
    #[error("`{0}` requires `--smoke-screenshot` (and `--smoke-every` requires `--smoke-sequence`)")]
    SequenceWithoutScreenshot(&'static str),
}

/// Parses a whole number in `1..=max` for `flag`.
fn bounded(flag: &'static str, text: String, max: u32) -> Result<u32, CliError> {
    text.parse::<u32>().ok().filter(|n| (1..=max).contains(n)).ok_or(CliError::InvalidSequence {
        flag,
        value: text,
        max,
    })
}

/// Parses the arguments (without the program name).
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Command, CliError> {
    let mut mode = None;
    let mut screenshot: Option<PathBuf> = None;
    let mut frames: Option<u32> = None;
    let mut sequence: Option<u32> = None;
    let mut every: Option<u32> = None;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = |name: &'static str| inline.clone().or_else(|| args.next()).ok_or(CliError::MissingValue(name));
        match flag.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--mode" => {
                let name = value("--mode")?;
                if mode.is_some() {
                    return Err(CliError::Repeated("--mode"));
                }
                mode = Some(AppMode::from_name(&name).ok_or_else(|| CliError::UnknownMode {
                    name,
                    valid: AppMode::ALL.iter().map(|m| m.slug()).collect::<Vec<_>>().join(", "),
                })?);
            }
            "--smoke-screenshot" => {
                let path = value("--smoke-screenshot")?;
                if screenshot.replace(PathBuf::from(path)).is_some() {
                    return Err(CliError::Repeated("--smoke-screenshot"));
                }
            }
            "--smoke-frames" => {
                let text = value("--smoke-frames")?;
                let n = text
                    .parse::<u32>()
                    .ok()
                    .filter(|n| (1..=100_000).contains(n))
                    .ok_or(CliError::InvalidFrames(text))?;
                if frames.replace(n).is_some() {
                    return Err(CliError::Repeated("--smoke-frames"));
                }
            }
            "--smoke-sequence" => {
                let n = bounded("--smoke-sequence", value("--smoke-sequence")?, MAX_SMOKE_SEQUENCE)?;
                if sequence.replace(n).is_some() {
                    return Err(CliError::Repeated("--smoke-sequence"));
                }
            }
            "--smoke-every" => {
                let n = bounded("--smoke-every", value("--smoke-every")?, 1_000)?;
                if every.replace(n).is_some() {
                    return Err(CliError::Repeated("--smoke-every"));
                }
            }
            _ => return Err(CliError::Unknown(arg)),
        }
    }
    if every.is_some() && sequence.is_none() {
        return Err(CliError::SequenceWithoutScreenshot("--smoke-every"));
    }
    let sequence = sequence.map(|count| Sequence { count, every: every.unwrap_or(DEFAULT_SMOKE_EVERY) });
    let smoke = match (screenshot, frames, sequence) {
        (Some(path), frames, sequence) => {
            Some(SmokeCapture { path, frames: frames.unwrap_or(DEFAULT_SMOKE_FRAMES), sequence })
        }
        (None, Some(_), _) => return Err(CliError::FramesWithoutScreenshot),
        (None, None, Some(_)) => return Err(CliError::SequenceWithoutScreenshot("--smoke-sequence")),
        (None, None, None) => None,
    };
    Ok(Command::Run(Options { mode, smoke }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str]) -> Result<Command, CliError> {
        parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn defaults_help_and_version() {
        assert_eq!(run(&[]), Ok(Command::Run(Options::default())));
        assert_eq!(run(&["--help"]), Ok(Command::Help));
        assert_eq!(run(&["-V"]), Ok(Command::Version));
    }

    #[test]
    fn mode_and_smoke_options() {
        let parsed = run(&["--mode", "Classic Conway", "--smoke-screenshot", "out.png", "--smoke-frames=12"]).unwrap();
        assert_eq!(
            parsed,
            Command::Run(Options {
                mode: Some(AppMode::ClassicConway),
                smoke: Some(SmokeCapture { path: "out.png".into(), frames: 12, sequence: None }),
            })
        );
        let parsed = run(&["--mode=ecosystem", "--smoke-screenshot", "x.png"]).unwrap();
        assert_eq!(
            parsed,
            Command::Run(Options {
                mode: Some(AppMode::EcosystemSimulator),
                smoke: Some(SmokeCapture { path: "x.png".into(), frames: DEFAULT_SMOKE_FRAMES, sequence: None }),
            })
        );
    }

    #[test]
    fn sequence_options() {
        let parsed = run(&["--smoke-screenshot", "rec/a.png", "--smoke-sequence", "90", "--smoke-every=3"]).unwrap();
        let Command::Run(Options { smoke: Some(capture), .. }) = parsed else { panic!("{parsed:?}") };
        assert_eq!(capture.sequence, Some(Sequence { count: 90, every: 3 }));
        let parsed = run(&["--smoke-screenshot", "a.png", "--smoke-sequence", "5"]).unwrap();
        let Command::Run(Options { smoke: Some(capture), .. }) = parsed else { panic!("{parsed:?}") };
        assert_eq!(capture.sequence, Some(Sequence { count: 5, every: DEFAULT_SMOKE_EVERY }));
        assert_eq!(run(&["--smoke-sequence", "5"]), Err(CliError::SequenceWithoutScreenshot("--smoke-sequence")));
        assert_eq!(
            run(&["--smoke-screenshot", "a.png", "--smoke-every", "2"]),
            Err(CliError::SequenceWithoutScreenshot("--smoke-every"))
        );
        assert!(matches!(
            run(&["--smoke-screenshot", "a.png", "--smoke-sequence", "0"]),
            Err(CliError::InvalidSequence { flag: "--smoke-sequence", .. })
        ));
    }

    #[test]
    fn errors_are_specific() {
        assert_eq!(run(&["--mode"]), Err(CliError::MissingValue("--mode")));
        assert!(matches!(run(&["--mode", "pong"]), Err(CliError::UnknownMode { .. })));
        assert_eq!(run(&["--smoke-frames", "5"]), Err(CliError::FramesWithoutScreenshot));
        assert_eq!(
            run(&["--smoke-screenshot", "a.png", "--smoke-frames", "0"]),
            Err(CliError::InvalidFrames("0".into()))
        );
        assert_eq!(run(&["--mode", "evolution", "--mode", "3d-life"]), Err(CliError::Repeated("--mode")));
        assert_eq!(run(&["--fast"]), Err(CliError::Unknown("--fast".into())));
        let message = run(&["--mode", "pong"]).unwrap_err().to_string();
        assert!(message.contains("classic-conway"), "{message}");
    }
}
