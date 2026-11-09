//! Line splitting shared by the text format readers.
//!
//! Pattern files come from many systems, so `\n`, `\r\n` and lone `\r` (classic
//! Mac OS) are all accepted as line terminators, and a leading UTF-8 byte order
//! mark is ignored. Line numbers are 1-based.

/// Iterator over `(line_number, line)` pairs without terminators.
pub(crate) struct Lines<'a> {
    rest: &'a str,
    number: usize,
    done: bool,
}

/// Splits `text` into lines (see the module documentation).
pub(crate) fn lines(text: &str) -> Lines<'_> {
    let rest = text.strip_prefix('\u{feff}').unwrap_or(text);
    Lines { rest, number: 0, done: rest.is_empty() }
}

impl<'a> Iterator for Lines<'a> {
    type Item = (usize, &'a str);

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        self.number += 1;
        match self.rest.find(['\n', '\r']) {
            Some(end) => {
                let line = &self.rest[..end];
                let terminator = if self.rest[end..].starts_with("\r\n") { 2 } else { 1 };
                self.rest = &self.rest[end + terminator..];
                self.done = self.rest.is_empty();
                Some((self.number, line))
            }
            None => {
                self.done = true;
                Some((self.number, self.rest))
            }
        }
    }
}

/// Lines of one metadata value (name, author, description or comment) for
/// the writers. They split at the same terminators as [`lines`] (`\n`,
/// `\r\n` and a lone `\r`), so no part of a value can land on a line of its
/// own and be read back as pattern data. An empty value is one empty line.
pub(crate) fn value_lines(value: &str) -> impl Iterator<Item = &str> {
    value.is_empty().then_some("").into_iter().chain(lines(value).map(|(_, line)| line))
}

/// A metadata value on one line: its non-blank lines trimmed and joined with
/// spaces, for fields that hold a single line (plaintext and Life 1.05 names,
/// authors and rules).
pub(crate) fn single_line(value: &str) -> String {
    value_lines(value).map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join(" ")
}

/// Returns the line with leading spaces and tabs removed, together with the
/// 1-based column of its first remaining character.
pub(crate) fn skip_indent(line: &str) -> (&str, usize) {
    let trimmed = line.trim_start_matches([' ', '\t']);
    // Spaces and tabs are single-byte characters, so bytes == characters here.
    (trimmed, line.len() - trimmed.len() + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_all_line_endings() {
        let text = "a\nb\r\nc\rd";
        let got: Vec<_> = lines(text).collect();
        assert_eq!(got, vec![(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
    }

    #[test]
    fn keeps_interior_blank_lines_and_drops_final_terminator() {
        let got: Vec<_> = lines("\u{feff}x\n\n\ny\n").collect();
        assert_eq!(got, vec![(1, "x"), (2, ""), (3, ""), (4, "y")]);
        assert_eq!(lines("").count(), 0);
    }

    #[test]
    fn value_lines_split_like_the_readers() {
        assert_eq!(value_lines("a\rb\r\nc\nd").collect::<Vec<_>>(), vec!["a", "b", "c", "d"]);
        assert_eq!(value_lines("").collect::<Vec<_>>(), vec![""]);
        assert_eq!(value_lines("x\n").collect::<Vec<_>>(), vec!["x"]);
        assert_eq!(single_line(" two \r lines "), "two lines");
        assert_eq!(single_line("a\n\n \r\nb"), "a b");
        assert_eq!(single_line(" \r "), "");
    }

    #[test]
    fn indent_column() {
        assert_eq!(skip_indent("  \tabc"), ("abc", 4));
        assert_eq!(skip_indent("abc"), ("abc", 1));
    }
}
