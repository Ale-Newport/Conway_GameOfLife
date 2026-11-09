//! Pattern file formats and automatic format detection.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::PatternError;
use crate::pattern::Pattern;
use crate::text::{lines, skip_indent};
use crate::{life105, life106, plaintext, rle};

/// A text pattern format this crate can read and write.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Format {
    /// Golly extended RLE (`.rle`).
    Rle,
    /// Plaintext (`.cells`).
    Plaintext,
    /// Life 1.05 (`.lif`, `.life`): blocks of `.`/`*` rows.
    Life105,
    /// Life 1.06 (`.lif`, `.life`): one `x y` coordinate pair per line.
    Life106,
}

impl Format {
    /// All formats.
    pub const ALL: [Format; 4] = [Self::Rle, Self::Plaintext, Self::Life105, Self::Life106];

    /// Display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::Rle => "RLE",
            Self::Plaintext => "plaintext",
            Self::Life105 => "Life 1.05",
            Self::Life106 => "Life 1.06",
        }
    }

    /// Preferred file extension (without the dot).
    pub fn extension(self) -> &'static str {
        match self {
            Self::Rle => "rle",
            Self::Plaintext => "cells",
            Self::Life105 | Self::Life106 => "lif",
        }
    }

    /// Format implied by a file extension (case-insensitive, without the
    /// dot). `.lif`/`.life` are ambiguous between Life 1.05 and 1.06 and map
    /// to [`Format::Life106`]; [`detect_format`] resolves them by content.
    pub fn from_extension(extension: &str) -> Option<Format> {
        match extension.to_ascii_lowercase().as_str() {
            "rle" => Some(Self::Rle),
            "cells" | "cell" => Some(Self::Plaintext),
            "lif" | "life" => Some(Self::Life106),
            _ => None,
        }
    }

    /// Parses `text` in this format.
    pub fn parse(self, text: &str) -> Result<Pattern, PatternError> {
        match self {
            Self::Rle => rle::parse(text),
            Self::Plaintext => plaintext::parse(text),
            Self::Life105 => life105::parse(text),
            Self::Life106 => life106::parse(text),
        }
    }

    /// Writes `pattern` in this format. RLE can store everything; the other
    /// formats reject multi-state patterns and very large dense areas.
    pub fn write(self, pattern: &Pattern) -> Result<String, PatternError> {
        match self {
            Self::Rle => Ok(rle::write(pattern)),
            Self::Plaintext => plaintext::write(pattern),
            Self::Life105 => life105::write(pattern),
            Self::Life106 => life106::write(pattern),
        }
    }
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Lower-case extension of a file name, if any.
fn extension_of(file_name: &str) -> Option<String> {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase())
}

/// Golly's test for headerless RLE: the line contains only counts, `b`, `o`,
/// `$` and whitespace (no whitespace inside a count), optionally ending with
/// `!`, and contains a digit or the `!`.
fn is_plain_rle(line: &str) -> bool {
    let (body, terminated) = match line.find('!') {
        Some(end) => {
            if !line[end + 1..].trim().is_empty() {
                return false;
            }
            (&line[..end], true)
        }
        None => (line, false),
    };
    let (mut previous_digit, mut any_digit) = (false, false);
    for ch in body.chars() {
        if ch.is_whitespace() {
            if previous_digit {
                return false;
            }
        } else if ch.is_ascii_digit() {
            previous_digit = true;
            any_digit = true;
        } else if matches!(ch, 'b' | 'o' | '$') {
            previous_digit = false;
        } else {
            return false;
        }
    }
    !previous_digit && (any_digit || terminated)
}

/// Decides the format of a pattern file.
///
/// The content is examined first, following Golly's `loadpattern`: the first
/// non-blank line `#Life 1.06` / `#Life 1.05` (or `#P ` for headerless Life
/// 1.05) selects Life 1.0x, a line starting with `#` or `x` (or a header
/// line that names `y` first) selects RLE, and `!` selects plaintext. Macrocell (`[M2]`) and MCell (`#MCell`) files are
/// reported as unsupported. When the content has no signature (bare cell rows
/// or headerless RLE) the file extension decides; without a known extension
/// Golly's headerless-RLE test is used and anything else is plaintext.
pub fn detect_format(text: &str, file_name: Option<&str>) -> Result<Format, PatternError> {
    let extension = file_name.and_then(extension_of);
    if let Some(ext) = extension.as_deref() {
        match ext {
            "gz" | "zip" => {
                return Err(PatternError::Unsupported(format!(
                    "compressed pattern files (.{ext}) must be decompressed before parsing"
                )));
            }
            "mc" => {
                return Err(PatternError::Unsupported(
                    "macrocell (.mc) files are read by the hashlife crate, not as cell lists".into(),
                ));
            }
            "mcl" => return Err(PatternError::Unsupported("MCell (.mcl) files are not supported".into())),
            _ => {}
        }
    }
    let first = lines(text).map(|(_, line)| skip_indent(line).0).find(|line| !line.trim_end().is_empty());
    let Some(first) = first else { return Err(PatternError::Empty) };

    if first.starts_with("#Life 1.06") {
        return Ok(Format::Life106);
    }
    if first.starts_with("#Life 1.05") || first.starts_with("#P ") {
        return Ok(Format::Life105);
    }
    if first.starts_with("#Life") {
        return Err(PatternError::Unsupported(format!("unknown Life file version `{}`", first.trim_end())));
    }
    if first.starts_with("#MCell") {
        return Err(PatternError::Unsupported("MCell (.mcl) files are not supported".into()));
    }
    if first.starts_with('[') {
        return Err(PatternError::Unsupported(
            "macrocell ([M2]) files are read by the hashlife crate, not as cell lists".into(),
        ));
    }
    if first.starts_with('#') || first.starts_with('x') || rle::is_header(first) {
        return Ok(Format::Rle);
    }
    if first.starts_with('!') {
        return Ok(Format::Plaintext);
    }
    match extension.as_deref() {
        Some("lif" | "life") => {
            let coordinates = first.starts_with(|c: char| c == '-' || c == '+' || c.is_ascii_digit());
            Ok(if coordinates { Format::Life106 } else { Format::Life105 })
        }
        Some("rle") => Ok(Format::Rle),
        Some("cells" | "cell") => Ok(Format::Plaintext),
        _ if is_plain_rle(first) => Ok(Format::Rle),
        _ => Ok(Format::Plaintext),
    }
}

/// Parses a pattern in any supported format, detected with [`detect_format`].
pub fn parse_any(text: &str, file_name: Option<&str>) -> Result<Pattern, PatternError> {
    detect_format(text, file_name)?.parse(text)
}

/// Reads a pattern file from disk and parses it with [`parse_any`], using the
/// file name for format detection.
///
/// Pattern data is ASCII, but old files may carry comments in a legacy
/// encoding such as Latin-1: bytes that are not valid UTF-8 are replaced with
/// U+FFFD instead of rejecting the file, as Golly also reads bytes.
pub fn read_pattern_file(path: impl AsRef<Path>) -> Result<Pattern, PatternError> {
    let path = path.as_ref();
    let bytes = std::fs::read(path)
        .map_err(|e| PatternError::Io { path: path.display().to_string(), message: e.to_string() })?;
    let text = String::from_utf8_lossy(&bytes);
    let file_name = path.file_name().map(|name| name.to_string_lossy());
    parse_any(&text, file_name.as_deref())
}

/// Writes `pattern` in `format` (see [`Format::write`]).
pub fn write_format(format: Format, pattern: &Pattern) -> Result<String, PatternError> {
    format.write(pattern)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_by_content_like_golly() {
        let cases: &[(&str, Format)] = &[
            ("#Life 1.06\n0 0\n", Format::Life106),
            ("#Life 1.05\n#P 0 0\n**\n", Format::Life105),
            ("#P 0 0\n.*\n", Format::Life105),
            ("#N Glider\nx = 3, y = 3\nbo$2bo$3o!", Format::Rle),
            ("\n\n  x = 3, y = 3\nbo$2bo$3o!", Format::Rle),
            ("y = 3, x = 3\nbo$2bo$3o!", Format::Rle),
            ("!Name: Glider\n.O.\n", Format::Plaintext),
            ("bo$2bo$3o!", Format::Rle),
            ("3o!", Format::Rle),
            ("2o$2o", Format::Rle),
            ("OO\nOO\n", Format::Plaintext),
            (".O.\n..O\nOOO", Format::Plaintext),
            ("oo", Format::Plaintext),
        ];
        for (text, format) in cases {
            assert_eq!(detect_format(text, None).unwrap(), *format, "{text:?}");
        }
    }

    #[test]
    fn extension_resolves_ambiguous_content() {
        assert_eq!(detect_format("oo\n", Some("x.rle")).unwrap(), Format::Rle);
        assert_eq!(detect_format("0 0\n1 0\n", Some("dir/a.LIF")).unwrap(), Format::Life106);
        assert_eq!(detect_format("**\n", Some("a.life")).unwrap(), Format::Life105);
        assert_eq!(detect_format("OO\n", Some("a.cells")).unwrap(), Format::Plaintext);
        // A signature in the content wins over the extension.
        assert_eq!(detect_format("!Name: x\nO\n", Some("a.rle")).unwrap(), Format::Plaintext);
    }

    #[test]
    fn unsupported_and_empty_inputs() {
        assert_eq!(detect_format("  \n\t\n", None), Err(PatternError::Empty));
        assert!(matches!(detect_format("[M2] (golly 4.0)\n", None), Err(PatternError::Unsupported(_))));
        assert!(matches!(detect_format("#MCell 4.20\n", None), Err(PatternError::Unsupported(_))));
        assert!(matches!(detect_format("x = 1, y = 1\no!", Some("a.rle.gz")), Err(PatternError::Unsupported(_))));
        assert!(matches!(detect_format("#Life 2.0\n", None), Err(PatternError::Unsupported(_))));
    }

    #[test]
    fn parse_any_reads_every_format() {
        let expected = Pattern::from_ascii(".O.\n..O\nOOO").unwrap().normalized().cells;
        let sources = [
            "x = 3, y = 3, rule = B3/S23\nbo$2bo$3o!",
            "!Name: Glider\n.O.\n..O\nOOO\n",
            "#Life 1.06\n1 0\n2 1\n0 2\n1 2\n2 2\n",
            "#Life 1.05\n#P -1 -1\n.*\n..*\n***\n",
        ];
        for source in sources {
            let pattern = parse_any(source, None).unwrap();
            assert_eq!(pattern.normalized().cells, expected, "{source}");
        }
        assert_eq!(Format::from_extension("CELLS"), Some(Format::Plaintext));
        assert_eq!(Format::Rle.extension(), "rle");
        assert_eq!(Format::Life105.to_string(), "Life 1.05");
    }

    #[test]
    fn reads_files_from_disk() {
        let dir = std::env::temp_dir().join(format!("patterns-format-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A Latin-1 'é' (0xE9) in a comment does not make the file unreadable.
        let path = dir.join("glider.LIF");
        std::fs::write(&path, b"#Life 1.05\n#D Caf\xe9\n#P 0 0\n.*\n..*\n***\n").unwrap();
        let pattern = read_pattern_file(&path).unwrap();
        assert_eq!(pattern.comments, vec!["Caf\u{fffd}"]);
        assert_eq!(pattern.population(), 5);
        // The extension decides when the content has no signature.
        let path = dir.join("bare.rle");
        std::fs::write(&path, "oo$oo").unwrap();
        assert_eq!(read_pattern_file(&path).unwrap().population(), 4);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(matches!(read_pattern_file(&path), Err(PatternError::Io { .. })));
    }

    #[test]
    fn plain_rle_heuristic() {
        assert!(is_plain_rle("bo$2bo$3o!"));
        assert!(is_plain_rle("3o"));
        assert!(!is_plain_rle("oo"));
        assert!(!is_plain_rle("2 o"));
        assert!(!is_plain_rle("3o! trailing"));
        assert!(!is_plain_rle("..O"));
    }
}
