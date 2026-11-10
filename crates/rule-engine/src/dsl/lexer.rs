//! Tokenizer for the rule DSL.
//!
//! Produces tokens with 1-based line/column [`Span`]s. Keywords are not
//! special at this level: they are identifiers that the parser compares
//! case-insensitively. Comments start with `#` or `//` and run to the end of
//! the line.

use crate::error::{Diagnostic, Span};

/// Token kinds.
#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    /// Identifier or keyword.
    Ident(String),
    /// Unsigned number literal.
    Num(f64),
    /// String literal (escapes `\"` and `\\` resolved).
    Str(String),
    /// `(`
    LParen,
    /// `)`
    RParen,
    /// `,`
    Comma,
    /// `:`
    Colon,
    /// `;`
    Semi,
    /// `.`
    Dot,
    /// `=`
    Assign,
    /// `==`
    EqEq,
    /// `!=`
    NotEq,
    /// `<`
    Lt,
    /// `<=`
    Le,
    /// `>`
    Gt,
    /// `>=`
    Ge,
    /// `+`
    Plus,
    /// `-`
    Minus,
    /// `*`
    Star,
    /// `/`
    Slash,
    /// `+=`
    PlusEq,
    /// `-=`
    MinusEq,
    /// `&&` (alias of `and`)
    AndAnd,
    /// `||` (alias of `or`)
    OrOr,
    /// `!` (alias of `not`)
    Bang,
    /// End of input.
    Eof,
}

impl Tok {
    /// Human-readable description for error messages.
    pub fn describe(&self) -> String {
        match self {
            Self::Ident(s) => format!("`{s}`"),
            Self::Num(n) => format!("number {n}"),
            Self::Str(s) => format!("string \"{s}\""),
            Self::Eof => "end of input".to_string(),
            other => format!("`{}`", other.symbol()),
        }
    }

    fn symbol(&self) -> &'static str {
        match self {
            Self::LParen => "(",
            Self::RParen => ")",
            Self::Comma => ",",
            Self::Colon => ":",
            Self::Semi => ";",
            Self::Dot => ".",
            Self::Assign => "=",
            Self::EqEq => "==",
            Self::NotEq => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::PlusEq => "+=",
            Self::MinusEq => "-=",
            Self::AndAnd => "&&",
            Self::OrOr => "||",
            Self::Bang => "!",
            Self::Ident(_) | Self::Num(_) | Self::Str(_) | Self::Eof => "",
        }
    }
}

/// A token and its location.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    /// Kind and payload.
    pub tok: Tok,
    /// Location.
    pub span: Span,
}

/// Maximum accepted source length in bytes (guards the editor against
/// pathological pastes; real rule sets are a few kilobytes).
pub const MAX_SOURCE_BYTES: usize = 1 << 20;

struct Cursor<'a> {
    chars: std::iter::Peekable<std::str::Chars<'a>>,
    line: u32,
    col: u32,
}

impl Cursor<'_> {
    fn peek(&mut self) -> Option<char> {
        self.chars.peek().copied()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.chars.next()?;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }
}

/// Splits `source` into tokens (always ending with [`Tok::Eof`]).
pub fn tokenize(source: &str) -> Result<Vec<Token>, Diagnostic> {
    if source.len() > MAX_SOURCE_BYTES {
        return Err(Diagnostic::error(
            "E015",
            format!("rule text is {} bytes long; the limit is {MAX_SOURCE_BYTES}", source.len()),
        ));
    }
    let mut cur = Cursor { chars: source.chars().peekable(), line: 1, col: 1 };
    let mut out = Vec::new();
    loop {
        // Skip whitespace and comments.
        while let Some(c) = cur.peek() {
            if c.is_whitespace() {
                cur.bump();
            } else if c == '#' {
                while cur.peek().is_some_and(|c| c != '\n') {
                    cur.bump();
                }
            } else if c == '/' && source_has_line_comment(&cur) {
                while cur.peek().is_some_and(|c| c != '\n') {
                    cur.bump();
                }
            } else {
                break;
            }
        }
        let (line, col) = (cur.line, cur.col);
        let Some(c) = cur.bump() else {
            out.push(Token { tok: Tok::Eof, span: Span::new(line, col, 1) });
            return Ok(out);
        };
        let tok = match c {
            '(' => Tok::LParen,
            ')' => Tok::RParen,
            ',' => Tok::Comma,
            ':' => Tok::Colon,
            ';' => Tok::Semi,
            '.' => Tok::Dot,
            '*' => Tok::Star,
            '/' => Tok::Slash,
            '=' => {
                if cur.eat('=') {
                    Tok::EqEq
                } else {
                    Tok::Assign
                }
            }
            '!' => {
                if cur.eat('=') {
                    Tok::NotEq
                } else {
                    Tok::Bang
                }
            }
            '<' => {
                if cur.eat('=') {
                    Tok::Le
                } else {
                    Tok::Lt
                }
            }
            '>' => {
                if cur.eat('=') {
                    Tok::Ge
                } else {
                    Tok::Gt
                }
            }
            '+' => {
                if cur.eat('=') {
                    Tok::PlusEq
                } else {
                    Tok::Plus
                }
            }
            '-' => {
                if cur.eat('=') {
                    Tok::MinusEq
                } else {
                    Tok::Minus
                }
            }
            '&' if cur.eat('&') => Tok::AndAnd,
            '|' if cur.eat('|') => Tok::OrOr,
            '"' => lex_string(&mut cur, line, col)?,
            c if c.is_ascii_digit() => lex_number(&mut cur, c, line, col)?,
            c if c.is_ascii_alphabetic() || c == '_' => {
                let mut ident = String::from(c);
                while let Some(n) = cur.peek().filter(|n| n.is_ascii_alphanumeric() || *n == '_') {
                    ident.push(n);
                    cur.bump();
                }
                Tok::Ident(ident)
            }
            other => {
                let hint = match other {
                    '&' => Some("use `and` (or `&&`)"),
                    '|' => Some("use `or` (or `||`)"),
                    '\'' => Some("strings use double quotes: \"name\""),
                    _ => None,
                };
                let mut d =
                    Diagnostic::error("E015", format!("unexpected character `{other}`")).at(Span::new(line, col, 1));
                if let Some(h) = hint {
                    d = d.help(h);
                }
                return Err(d);
            }
        };
        let len = if cur.line == line { (cur.col - col).max(1) } else { 1 };
        out.push(Token { tok, span: Span::new(line, col, len) });
    }
}

fn source_has_line_comment(cur: &Cursor<'_>) -> bool {
    let mut probe = cur.chars.clone();
    probe.next() == Some('/') && probe.next() == Some('/')
}

fn lex_string(cur: &mut Cursor<'_>, line: u32, col: u32) -> Result<Tok, Diagnostic> {
    let mut s = String::new();
    loop {
        match cur.bump() {
            Some('"') => return Ok(Tok::Str(s)),
            Some('\\') => match cur.bump() {
                Some('"') => s.push('"'),
                Some('\\') => s.push('\\'),
                Some(other) => {
                    return Err(Diagnostic::error("E015", format!("unknown escape `\\{other}` in string"))
                        .at(Span::new(cur.line, cur.col.saturating_sub(2).max(1), 2))
                        .help("only \\\" and \\\\ are supported"));
                }
                None => break,
            },
            Some('\n') | None => break,
            Some(c) => s.push(c),
        }
    }
    Err(Diagnostic::error("E015", "unterminated string").at(Span::new(line, col, 1)).help("close the string with `\"`"))
}

fn lex_number(cur: &mut Cursor<'_>, first: char, line: u32, col: u32) -> Result<Tok, Diagnostic> {
    let mut text = String::from(first);
    while let Some(d) = cur.peek().filter(char::is_ascii_digit) {
        text.push(d);
        cur.bump();
    }
    // A fraction needs a digit after the dot, so `a.b` paths are unaffected.
    if cur.peek() == Some('.') {
        let mut probe = cur.chars.clone();
        probe.next();
        if probe.next().is_some_and(|c| c.is_ascii_digit()) {
            text.push('.');
            cur.bump();
            while let Some(d) = cur.peek().filter(char::is_ascii_digit) {
                text.push(d);
                cur.bump();
            }
        }
    }
    if matches!(cur.peek(), Some('e' | 'E')) {
        let mut probe = cur.chars.clone();
        probe.next();
        let next = probe.next();
        let exponent_follows = match next {
            Some(c) if c.is_ascii_digit() => true,
            Some('+' | '-') => probe.next().is_some_and(|c| c.is_ascii_digit()),
            _ => false,
        };
        if exponent_follows {
            for _ in 0..2 {
                if let Some(c) = cur.peek().filter(|c| matches!(c, 'e' | 'E' | '+' | '-')) {
                    text.push(c);
                    cur.bump();
                }
            }
            while let Some(d) = cur.peek().filter(char::is_ascii_digit) {
                text.push(d);
                cur.bump();
            }
        }
    }
    match text.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(Tok::Num(v)),
        _ => Err(Diagnostic::error("E015", format!("number `{text}` is out of range")).at(Span::new(
            line,
            col,
            text.chars().count() as u32,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<Tok> {
        tokenize(src).unwrap().into_iter().map(|t| t.tok).collect()
    }

    #[test]
    fn tokens_and_positions() {
        let toks = tokenize("rule a:\n  when self.energy >= 2.5 # comment\n then eat(other)").unwrap();
        assert_eq!(toks[0].tok, Tok::Ident("rule".into()));
        let ge = toks.iter().find(|t| t.tok == Tok::Ge).unwrap();
        assert_eq!((ge.span.line, ge.span.col, ge.span.len), (2, 20, 2));
        assert!(toks.iter().any(|t| t.tok == Tok::Num(2.5)));
        assert_eq!(toks.last().unwrap().tok, Tok::Eof);
    }

    #[test]
    fn numbers_paths_and_operators() {
        assert_eq!(
            kinds("a.b 1.5e2 3 x+=1 y-=2 != && || // c"),
            vec![
                Tok::Ident("a".into()),
                Tok::Dot,
                Tok::Ident("b".into()),
                Tok::Num(150.0),
                Tok::Num(3.0),
                Tok::Ident("x".into()),
                Tok::PlusEq,
                Tok::Num(1.0),
                Tok::Ident("y".into()),
                Tok::MinusEq,
                Tok::Num(2.0),
                Tok::NotEq,
                Tok::AndAnd,
                Tok::OrOr,
                Tok::Eof
            ]
        );
    }

    #[test]
    fn strings_with_escapes_and_errors() {
        assert_eq!(kinds(r#""a\"b""#)[0], Tok::Str("a\"b".into()));
        let err = tokenize("when x == \"open").unwrap_err();
        assert_eq!(err.code, "E015");
        assert_eq!(err.span.unwrap().col, 11);
        let err = tokenize("a & b").unwrap_err();
        assert!(err.help.unwrap().contains("and"));
        let err = tokenize("1e999").unwrap_err();
        assert!(err.message.contains("out of range"));
    }
}
