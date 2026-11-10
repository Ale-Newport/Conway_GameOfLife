//! Pretty-printer: AST → canonical DSL text.
//!
//! The output re-parses to an equal AST (`parse(print(ast)) == ast`, spans
//! ignored) and printing is idempotent (`print(parse(print(x))) ==
//! print(x)`). Parentheses are emitted only where precedence or
//! associativity requires them to preserve the tree shape.

use std::fmt::Write as _;

use crate::dsl::ast::{Action, Arg, Event, Expr, ExprKind, Name, RuleAst, UnOp};
use crate::dsl::parser::is_keyword;

/// Precedence of `not` (between `and` and comparisons).
const NOT_PREC: u8 = 3;
/// Precedence of unary minus.
const NEG_PREC: u8 = 7;
/// Precedence of literals, paths, calls and parenthesised expressions.
const ATOM_PREC: u8 = 8;

/// Whether `word` can be printed without quotes.
fn is_bare(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !is_keyword(word)
}

fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// A name as an identifier when possible, otherwise as a string literal.
pub fn print_name(text: &str) -> String {
    if is_bare(text) { text.to_string() } else { quote(text) }
}

/// Number literal that parses back to the same `f64`. Rust's `Display`
/// and `LowerExp` both print the shortest digits that round-trip; very
/// large or very small magnitudes use the exponent form (`1e300`, `2.5e-9`)
/// so that literals stay short. Non-finite values have no literal form
/// (validation rejects them) and print as `NaN` / `inf`.
pub fn print_number(value: f64) -> String {
    let a = value.abs();
    let s =
        if a != 0.0 && a.is_finite() && !(1e-6..1e16).contains(&a) { format!("{value:e}") } else { format!("{value}") };
    if s == "-0" { "0".to_string() } else { s }
}

fn precedence(e: &Expr) -> u8 {
    match &e.kind {
        ExprKind::Binary { op, .. } => op.precedence(),
        ExprKind::Unary { op: UnOp::Not, .. } => NOT_PREC,
        ExprKind::Unary { op: UnOp::Neg, .. } => NEG_PREC,
        // A negative literal prints with a leading `-`; treat it like unary minus.
        ExprKind::Num(n) if n.is_sign_negative() && *n != 0.0 => NEG_PREC,
        _ => ATOM_PREC,
    }
}

fn write_child(out: &mut String, e: &Expr, min_prec: u8) {
    if precedence(e) < min_prec {
        out.push('(');
        write_expr(out, e);
        out.push(')');
    } else {
        write_expr(out, e);
    }
}

fn write_args(out: &mut String, args: &[Arg]) {
    out.push('(');
    for (i, arg) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        if let Some(name) = &arg.name {
            out.push_str(name);
            out.push('=');
        }
        write_expr(out, &arg.value);
    }
    out.push(')');
}

fn write_expr(out: &mut String, e: &Expr) {
    match &e.kind {
        ExprKind::Num(n) => out.push_str(&print_number(*n)),
        ExprKind::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        ExprKind::Str(s) => out.push_str(&quote(s)),
        ExprKind::Path(segments) => out.push_str(&segments.join(".")),
        ExprKind::Call { name, args } => {
            out.push_str(name);
            write_args(out, args);
        }
        ExprKind::Unary { op: UnOp::Not, expr } => {
            out.push_str("not ");
            write_child(out, expr, NOT_PREC);
        }
        ExprKind::Unary { op: UnOp::Neg, expr } => {
            out.push('-');
            // `-(3)` keeps `Neg(Num(3))` distinct from the literal `-3`.
            if matches!(expr.kind, ExprKind::Num(_)) || precedence(expr) < NEG_PREC {
                out.push('(');
                write_expr(out, expr);
                out.push(')');
            } else {
                write_expr(out, expr);
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let p = op.precedence();
            // Comparisons do not chain, so both sides must bind tighter.
            let left_min = if op.is_comparison() { p + 1 } else { p };
            write_child(out, lhs, left_min);
            let _ = write!(out, " {} ", op.symbol());
            write_child(out, rhs, p + 1);
        }
    }
}

/// Prints an expression.
pub fn print_expr(e: &Expr) -> String {
    let mut out = String::new();
    write_expr(&mut out, e);
    out
}

fn print_action(a: &Action) -> String {
    let mut out = String::new();
    match a {
        Action::Call { name, args, .. } => {
            out.push_str(name);
            write_args(&mut out, args);
        }
        Action::Set { path, value, .. } => {
            let _ = write!(out, "set {} = ", path.join("."));
            write_expr(&mut out, value);
        }
        Action::Add { path, value, subtract, .. } => {
            let _ = write!(out, "add {} {} ", path.join("."), if *subtract { "-=" } else { "+=" });
            write_expr(&mut out, value);
        }
    }
    out
}

fn print_event(e: &Event) -> Option<String> {
    match e {
        Event::Tick => None,
        Event::Birth => Some("on_birth".into()),
        Event::Death => Some("on_death".into()),
        Event::Collision { with: None } => Some("on_collision".into()),
        Event::Collision { with: Some(Name { text, .. }) } => Some(format!("on_collision with {}", print_name(text))),
        Event::StateEnter { state } => Some(format!("on_state_enter({})", print_name(&state.text))),
    }
}

/// Prints one rule in canonical multi-line form.
pub fn print_rule(rule: &RuleAst) -> String {
    let mut out = format!("rule {} for {}:", print_name(&rule.name), print_name(&rule.entity.text));
    if let Some(event) = print_event(&rule.event) {
        let _ = write!(out, "\n    {event}");
    }
    if let Some(cond) = &rule.condition {
        let _ = write!(out, "\n    when {}", print_expr(cond));
    }
    let actions: Vec<String> = rule.actions.iter().map(print_action).collect();
    let _ = write!(out, "\n    then {}", actions.join(", "));
    let m = &rule.meta;
    let mut mods = Vec::new();
    if m.priority != 0 {
        mods.push(format!("priority {}", m.priority));
    }
    if let Some(p) = m.probability {
        mods.push(format!("probability {}", print_number(p)));
    }
    if let Some(c) = m.cooldown {
        mods.push(format!("cooldown {c}"));
    }
    for c in &m.costs {
        mods.push(format!("cost {} {}", c.path.join("."), print_number(c.amount)));
    }
    for p in &m.produces {
        if p.amount == 1.0 {
            mods.push(format!("produces {}", print_name(&p.name.text)));
        } else {
            mods.push(format!("produces {} {}", print_name(&p.name.text), print_number(p.amount)));
        }
    }
    if m.once {
        mods.push("once".into());
    }
    if !mods.is_empty() {
        let _ = write!(out, "\n    {}", mods.join(" "));
    }
    out
}

/// Prints a program: rules separated by blank lines.
pub fn print_program(rules: &[RuleAst]) -> String {
    rules.iter().map(print_rule).collect::<Vec<_>>().join("\n\n")
}
