//! `convert`: pattern files through the `patterns` crate (RLE, plaintext,
//! Life 1.05, Life 1.06) and Hashlife's macrocell reader and writer.

use std::path::Path;

use anyhow::{Context, bail};
use patterns::{Format, Pattern};

use crate::cli::{ConvertArgs, PatternFormat};
use crate::output::outln;
use crate::output::{flat, write_file};

/// Largest pattern file read.
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

fn extension(path: &Path) -> String {
    path.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase()
}

fn label(format: PatternFormat) -> &'static str {
    match format {
        PatternFormat::Rle => Format::Rle.label(),
        PatternFormat::Plaintext => Format::Plaintext.label(),
        PatternFormat::Life105 => Format::Life105.label(),
        PatternFormat::Life106 => Format::Life106.label(),
        PatternFormat::Macrocell => "macrocell",
    }
}

fn cell_format(format: PatternFormat) -> Option<Format> {
    match format {
        PatternFormat::Rle => Some(Format::Rle),
        PatternFormat::Plaintext => Some(Format::Plaintext),
        PatternFormat::Life105 => Some(Format::Life105),
        PatternFormat::Life106 => Some(Format::Life106),
        PatternFormat::Macrocell => None,
    }
}

/// Reads a pattern file; returns it with the detected format.
///
/// # Errors
/// Unreadable files and every parse error (with line and column).
pub fn read(path: &Path) -> anyhow::Result<(Pattern, PatternFormat)> {
    let bytes = persistence::read_limited(path, MAX_INPUT_BYTES).map_err(flat)?;
    // Old pattern files may carry Latin-1 comments; Golly also reads bytes.
    let text = String::from_utf8_lossy(&bytes);
    let fail = || format!("{}", path.display());
    if extension(path) == "mc" || text.trim_start().starts_with("[M2]") {
        let universe = hashlife::macrocell::read(&text).with_context(fail)?;
        let population = universe.population();
        if population > patterns::MAX_CELLS as u64 {
            bail!(
                "{}: {population} live cells exceed the {} cells a pattern file can hold",
                path.display(),
                patterns::MAX_CELLS
            );
        }
        let mut pattern = Pattern::from_live_cells(universe.cells()).with_rule(universe.rule().to_string());
        if universe.generation() > 0 {
            pattern.comments.push(format!("Generation {} (from the macrocell file)", universe.generation()));
        }
        return Ok((pattern, PatternFormat::Macrocell));
    }
    let name = path.file_name().map(|n| n.to_string_lossy());
    let format = patterns::detect_format(&text, name.as_deref()).with_context(fail)?;
    let pattern = format.parse(&text).with_context(fail)?;
    let detected = match format {
        Format::Rle => PatternFormat::Rle,
        Format::Plaintext => PatternFormat::Plaintext,
        Format::Life105 => PatternFormat::Life105,
        Format::Life106 => PatternFormat::Life106,
    };
    Ok((pattern, detected))
}

/// Writes `pattern` in `format`.
///
/// # Errors
/// Formats that cannot hold the pattern (multi-state cells outside RLE,
/// rules Hashlife cannot run for macrocell files).
pub fn write(pattern: &Pattern, format: PatternFormat) -> anyhow::Result<String> {
    if let Some(cells) = cell_format(format) {
        return Ok(cells.write(pattern)?);
    }
    if pattern.max_state() > 1 {
        bail!("macrocell files hold two-state patterns; this pattern has {} states", pattern.max_state() + 1);
    }
    let rule_text = pattern.rule.as_deref().unwrap_or("B3/S23");
    let rule = hashlife::Rule::parse(rule_text)
        .with_context(|| format!("macrocell files need a two-state Life-like rule without B0, not `{rule_text}`"))?;
    let mut universe = hashlife::Universe::with_rule(rule);
    universe.insert_cells(pattern.live_cells());
    Ok(hashlife::macrocell::write(&universe))
}

/// The output format: `--format`, else the output file's extension.
fn output_format(args: &ConvertArgs) -> anyhow::Result<PatternFormat> {
    if let Some(format) = args.format {
        return Ok(format);
    }
    let ext = extension(&args.output);
    if ext == "mc" {
        return Ok(PatternFormat::Macrocell);
    }
    match Format::from_extension(&ext) {
        Some(Format::Rle) => Ok(PatternFormat::Rle),
        Some(Format::Plaintext) => Ok(PatternFormat::Plaintext),
        Some(Format::Life105) => Ok(PatternFormat::Life105),
        Some(Format::Life106) => Ok(PatternFormat::Life106),
        None => bail!(
            "cannot tell the output format from `{}`; use .rle, .cells, .lif, .mc or --format",
            args.output.display()
        ),
    }
}

/// `convert IN OUT [--format F]`.
///
/// # Errors
/// Read, conversion and write errors.
pub fn convert(args: &ConvertArgs) -> anyhow::Result<()> {
    let format = output_format(args)?;
    let (pattern, input_format) = read(&args.input)?;
    let text = write(&pattern, format).with_context(|| format!("cannot write {}", args.output.display()))?;
    write_file(&args.output, &text)?;
    let size = match pattern.bounding_box() {
        Some(_) => format!("{}×{}", pattern.width(), pattern.height()),
        None => "empty".to_string(),
    };
    outln!(
        "{} ({}) -> {} ({}): {} live cells, bounding box {size}, rule {}",
        args.input.display(),
        label(input_format),
        args.output.display(),
        label(format),
        pattern.population(),
        pattern.rule.as_deref().unwrap_or("B3/S23 (default)")
    );
    Ok(())
}
