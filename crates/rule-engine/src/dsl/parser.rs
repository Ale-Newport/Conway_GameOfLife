//! Recursive-descent parser for the rule DSL.
//!
//! Grammar (ISO-14977 EBNF; keywords are case-insensitive, `#` and `//`
//! start comments):
//!
//! ```text
//! program   = { rule } ;
//! rule      = "rule" , name , "for" , ident , ":" , [ event ] , [ "when" , expr ] ,
//!             "then" , action , { "," , action } , { modifier } , [ ";" ] ;
//! name      = ident | string ;
//! event     = "on_tick" | "on_birth" | "on_death"
//!           | ( "on_collision" | "on_contact" ) , [ "with" , name ]
//!           | "on_state_enter" , "(" , name , ")" ;
//! modifier  = "priority" , [ "-" ] , int | "probability" , number | "cooldown" , int
//!           | "cost" , path , number | "produces" , name , [ number ] | "once" ;
//! action    = "set" , path , "=" , expr | "add" , path , ( "+=" | "-=" ) , expr | call ;
//! expr      = and_expr , { ( "or" | "||" ) , and_expr } ;
//! and_expr  = not_expr , { ( "and" | "&&" ) , not_expr } ;
//! not_expr  = ( "not" | "!" ) , not_expr | cmp_expr ;
//! cmp_expr  = sum , [ ( "<" | "<=" | ">" | ">=" | "==" | "!=" ) , sum ] ;
//! sum       = term , { ( "+" | "-" ) , term } ;
//! term      = unary , { ( "*" | "/" ) , unary } ;
//! unary     = "-" , unary | primary ;
//! primary   = number | string | "true" | "false" | call | path | "(" , expr , ")" ;
//! call      = ident , "(" , [ arg , { "," , arg } ] , ")" ;
//! arg       = [ ident , "=" ] , expr ;
//! path      = ident , { "." , ident } ;
//! ```
//!
//! `-` directly followed by a number literal folds into a negative literal.
//! Comparisons do not chain (`a < b < c` is an error with a hint). After a
//! syntax error the parser skips to the next `rule` keyword, so one mistake
//! does not hide the diagnostics of later rules.
//!
//! **Depth limit.** Every parenthesis, `not`, unary `-` and every extra
//! operand of an operator chain (`a + b + c` is the left-nested tree
//! `(a + b) + c`) counts as one level; the resulting tree is never deeper
//! than [`MAX_EXPR_DEPTH`], so the recursive passes that follow (printer,
//! validation, evaluation, drop) cannot overflow the stack on hostile input.

use crate::dsl::ast::{Action, Arg, BinOp, Cost, Event, Expr, ExprKind, Name, Product, RuleAst, RuleMeta, UnOp};
use crate::dsl::lexer::{Tok, Token, tokenize};
use crate::error::{Diagnostic, Span};

/// Words that cannot be used as bare names (quote them instead).
pub const KEYWORDS: &[&str] = &[
    "rule",
    "for",
    "when",
    "then",
    "and",
    "or",
    "not",
    "true",
    "false",
    "priority",
    "probability",
    "cooldown",
    "cost",
    "produces",
    "once",
    "set",
    "add",
    "with",
    "on_tick",
    "on_birth",
    "on_death",
    "on_collision",
    "on_contact",
    "on_state_enter",
];

/// Words that may follow the rule header (for "did you mean" hints).
const RULE_WORDS: &[&str] = &["when", "then", "on_tick", "on_birth", "on_death", "on_collision", "on_state_enter"];

/// Whether `word` is a reserved keyword (case-insensitive).
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(word))
}

/// Maximum nesting depth of expressions, counting parentheses, `not`, unary
/// `-` and the operands of operator chains (bounds recursion on hostile
/// input; see the module documentation).
pub const MAX_EXPR_DEPTH: usize = 64;

type PResult<T> = Result<T, Diagnostic>;

struct Parser {
    toks: Vec<Token>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        // `tokenize` always ends with Eof and `bump` never moves past it.
        &self.toks[self.pos.min(self.toks.len() - 1)]
    }

    fn peek_at(&self, offset: usize) -> &Tok {
        &self.toks[(self.pos + offset).min(self.toks.len() - 1)].tok
    }

    fn bump(&mut self) -> Token {
        let t = self.peek().clone();
        if self.pos < self.toks.len() - 1 {
            self.pos += 1;
        }
        t
    }

    fn at_kw(&self, kw: &str) -> bool {
        matches!(&self.peek().tok, Tok::Ident(s) if s.eq_ignore_ascii_case(kw))
    }

    fn eat_kw(&mut self, kw: &str) -> bool {
        if self.at_kw(kw) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn eat(&mut self, tok: &Tok) -> bool {
        if &self.peek().tok == tok {
            self.bump();
            true
        } else {
            false
        }
    }

    fn unexpected(&self, expected: &str) -> Diagnostic {
        let t = self.peek();
        Diagnostic::error("E015", format!("expected {expected}, found {}", t.tok.describe())).at(t.span)
    }

    fn expect(&mut self, tok: &Tok, expected: &str) -> PResult<Token> {
        if &self.peek().tok == tok { Ok(self.bump()) } else { Err(self.unexpected(expected)) }
    }

    fn expect_kw(&mut self, kw: &str) -> PResult<Token> {
        if self.at_kw(kw) { Ok(self.bump()) } else { Err(self.unexpected(&format!("`{kw}`"))) }
    }

    /// Identifier that is not a keyword.
    fn ident(&mut self, what: &str) -> PResult<(String, Span)> {
        match &self.peek().tok {
            Tok::Ident(s) if !is_keyword(s) => {
                let s = s.clone();
                let span = self.bump().span;
                Ok((s, span))
            }
            Tok::Ident(s) => {
                let d = self.unexpected(what).help(format!("`{s}` is a keyword; write it as a string: \"{s}\""));
                Err(d)
            }
            _ => Err(self.unexpected(what)),
        }
    }

    /// Identifier or string literal.
    fn name(&mut self, what: &str) -> PResult<Name> {
        if let Tok::Str(s) = &self.peek().tok {
            let text = s.clone();
            let span = self.bump().span;
            return Ok(Name { text, span });
        }
        let (text, span) = self.ident(what)?;
        Ok(Name { text, span })
    }

    fn number(&mut self, what: &str) -> PResult<(f64, Span)> {
        let negative = self.peek().tok == Tok::Minus && matches!(self.peek_at(1), Tok::Num(_));
        let start = self.peek().span;
        if negative {
            self.bump();
        }
        match self.peek().tok {
            Tok::Num(n) => {
                self.bump();
                Ok((if negative { -n } else { n }, start))
            }
            _ => Err(self.unexpected(what)),
        }
    }

    fn integer(&mut self, what: &str) -> PResult<(i64, Span)> {
        let (n, span) = self.number(what)?;
        if n.fract() != 0.0 || n.abs() > 9.0e15 {
            return Err(Diagnostic::error("E012", format!("{what} must be a whole number, got {n}")).at(span));
        }
        Ok((n as i64, span))
    }

    fn skip_to_next_rule(&mut self) {
        while !matches!(self.peek().tok, Tok::Eof) && !self.at_kw("rule") {
            self.bump();
        }
    }

    // ----------------------------------------------------------------- rules

    fn rule(&mut self) -> PResult<RuleAst> {
        let span = self.expect_kw("rule")?.span;
        let name = self.name("a rule name")?.text;
        self.expect_kw("for")?;
        let entity = self.name("an entity type name")?;
        self.expect(&Tok::Colon, "`:` after the entity type")?;
        let event = self.event()?;
        let condition = if self.eat_kw("when") { Some(self.expr()?) } else { None };
        if !self.at_kw("then") {
            let d = self.unexpected("`then` followed by actions");
            let typo = match &self.peek().tok {
                Tok::Ident(w) => crate::error::did_you_mean(&w.to_ascii_lowercase(), RULE_WORDS.iter().copied()),
                _ => None,
            };
            return Err(match typo {
                Some(h) => d.help(h),
                None if condition.is_none() => d.help("a rule needs `then <action>`"),
                None => d,
            });
        }
        self.bump();
        let mut actions = vec![self.action()?];
        while self.eat(&Tok::Comma) {
            actions.push(self.action()?);
        }
        let meta = self.modifiers()?;
        self.eat(&Tok::Semi);
        if !matches!(self.peek().tok, Tok::Eof) && !self.at_kw("rule") {
            let d = self.unexpected("`,` between actions, a modifier, `;` or the next `rule`");
            return Err(d.help("modifiers are priority, probability, cooldown, cost, produces and once"));
        }
        Ok(RuleAst { name, entity, event, condition, actions, meta, span })
    }

    fn event(&mut self) -> PResult<Event> {
        let Tok::Ident(word) = &self.peek().tok else { return Ok(Event::Tick) };
        let lower = word.to_ascii_lowercase();
        let event = match lower.as_str() {
            "on_tick" => {
                self.bump();
                Event::Tick
            }
            "on_birth" => {
                self.bump();
                Event::Birth
            }
            "on_death" => {
                self.bump();
                Event::Death
            }
            "on_collision" | "on_contact" => {
                self.bump();
                let with = if self.eat_kw("with") { Some(self.name("an entity type after `with`")?) } else { None };
                Event::Collision { with }
            }
            "on_state_enter" => {
                self.bump();
                self.expect(&Tok::LParen, "`(` after on_state_enter")?;
                let state = self.name("a state name")?;
                self.expect(&Tok::RParen, "`)`")?;
                Event::StateEnter { state }
            }
            _ if lower.starts_with("on_") => {
                return Err(self.unexpected("an event").help(
                    "events are on_tick, on_birth, on_death, on_collision [with TYPE] and on_state_enter(STATE)",
                ));
            }
            _ => Event::Tick,
        };
        Ok(event)
    }

    fn modifiers(&mut self) -> PResult<RuleMeta> {
        let mut meta = RuleMeta::default();
        loop {
            if self.eat_kw("priority") {
                let (p, span) = self.integer("a priority")?;
                meta.priority = i32::try_from(p).map_err(|_| {
                    Diagnostic::error("E012", format!("priority {p} is outside the 32-bit range")).at(span)
                })?;
            } else if self.eat_kw("probability") {
                meta.probability = Some(self.number("a probability")?.0);
            } else if self.eat_kw("cooldown") {
                let (c, span) = self.integer("a cooldown")?;
                meta.cooldown = Some(u64::try_from(c).map_err(|_| {
                    Diagnostic::error("E012", format!("cooldown must not be negative (got {c})")).at(span)
                })?);
            } else if self.at_kw("cost") {
                let span = self.bump().span;
                let (path, _) = self.path()?;
                let amount = self.number("a cost amount")?.0;
                meta.costs.push(Cost { path, amount, span });
            } else if self.eat_kw("produces") {
                let name = self.name("a product (state, entity type or resource)")?;
                let amount = if matches!(self.peek().tok, Tok::Num(_)) { self.number("an amount")?.0 } else { 1.0 };
                meta.produces.push(Product { name, amount });
            } else if self.eat_kw("once") {
                meta.once = true;
            } else {
                return Ok(meta);
            }
        }
    }

    fn path(&mut self) -> PResult<(Vec<String>, Span)> {
        let (first, span) = self.ident("a property name")?;
        let mut segments = vec![first];
        while self.peek().tok == Tok::Dot {
            self.bump();
            segments.push(self.ident("a property name after `.`")?.0);
        }
        Ok((segments, span))
    }

    fn action(&mut self) -> PResult<Action> {
        let span = self.peek().span;
        if self.eat_kw("set") {
            let (path, _) = self.path()?;
            self.expect(&Tok::Assign, "`=` after the property")?;
            let value = self.expr()?;
            return Ok(Action::Set { path, value, span });
        }
        if self.eat_kw("add") {
            let (path, _) = self.path()?;
            let subtract = match self.peek().tok {
                Tok::PlusEq => false,
                Tok::MinusEq => true,
                _ => return Err(self.unexpected("`+=` or `-=`")),
            };
            self.bump();
            let value = self.expr()?;
            return Ok(Action::Add { path, value, subtract, span });
        }
        let (name, span) = self.ident("an action")?;
        if self.peek().tok != Tok::LParen {
            return Err(self.unexpected("`(` after the action name").help(format!("write `{name}()`")));
        }
        let args = self.args()?;
        Ok(Action::Call { name, args, span })
    }

    fn args(&mut self) -> PResult<Vec<Arg>> {
        self.expect(&Tok::LParen, "`(`")?;
        let mut args = Vec::new();
        if self.eat(&Tok::RParen) {
            return Ok(args);
        }
        loop {
            // Argument names may be keywords (`probability=0.5`).
            let name = match (&self.peek().tok, self.peek_at(1)) {
                (Tok::Ident(n), Tok::Assign) => {
                    let n = n.clone();
                    self.bump();
                    self.bump(); // `=`
                    Some(n)
                }
                _ => None,
            };
            let value = self.expr()?;
            args.push(Arg { name, value });
            if self.eat(&Tok::RParen) {
                return Ok(args);
            }
            self.expect(&Tok::Comma, "`,` or `)` in the argument list")?;
        }
    }

    // ----------------------------------------------------------- expressions

    fn enter(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_EXPR_DEPTH {
            return Err(Diagnostic::error("E015", format!("expression nested deeper than {MAX_EXPR_DEPTH} levels"))
                .at(self.peek().span)
                .help("long chains such as `a + b + c + ...` count as nesting; split the rule or simplify it"));
        }
        Ok(())
    }

    /// Parses a left-associative chain `operand { op operand }`; every extra
    /// operand deepens the resulting tree by one level and is counted.
    fn chain(&mut self, operand: fn(&mut Self) -> PResult<Expr>, op_of: fn(&Self) -> Option<BinOp>) -> PResult<Expr> {
        let mut lhs = operand(self)?;
        let mut extra = 0;
        while let Some(op) = op_of(self) {
            self.bump();
            self.enter()?;
            extra += 1;
            let rhs = operand(self)?;
            lhs = joined(op, lhs, rhs);
        }
        self.depth -= extra;
        Ok(lhs)
    }

    fn expr(&mut self) -> PResult<Expr> {
        self.enter()?;
        let e = self.chain(Self::and_expr, |p| (p.at_kw("or") || p.peek().tok == Tok::OrOr).then_some(BinOp::Or))?;
        self.depth -= 1;
        Ok(e)
    }

    fn and_expr(&mut self) -> PResult<Expr> {
        self.chain(Self::not_expr, |p| (p.at_kw("and") || p.peek().tok == Tok::AndAnd).then_some(BinOp::And))
    }

    fn not_expr(&mut self) -> PResult<Expr> {
        if self.at_kw("not") || self.peek().tok == Tok::Bang {
            let span = self.bump().span;
            self.enter()?;
            let inner = self.not_expr()?;
            self.depth -= 1;
            return Ok(Expr { kind: ExprKind::Unary { op: UnOp::Not, expr: Box::new(inner) }, span });
        }
        self.cmp_expr()
    }

    fn cmp_expr(&mut self) -> PResult<Expr> {
        let lhs = self.sum()?;
        let Some(op) = cmp_op(&self.peek().tok) else { return Ok(lhs) };
        let op_span = self.bump().span;
        let rhs = self.sum()?;
        if cmp_op(&self.peek().tok).is_some() {
            return Err(Diagnostic::error("E015", "comparisons cannot be chained")
                .at(self.peek().span)
                .help("write `a < b and b < c`"));
        }
        let mut e = joined(op, lhs, rhs);
        if !e.span.is_known() {
            e.span = op_span;
        }
        Ok(e)
    }

    fn sum(&mut self) -> PResult<Expr> {
        self.chain(Self::term, |p| match p.peek().tok {
            Tok::Plus => Some(BinOp::Add),
            Tok::Minus => Some(BinOp::Sub),
            _ => None,
        })
    }

    fn term(&mut self) -> PResult<Expr> {
        self.chain(Self::unary, |p| match p.peek().tok {
            Tok::Star => Some(BinOp::Mul),
            Tok::Slash => Some(BinOp::Div),
            _ => None,
        })
    }

    fn unary(&mut self) -> PResult<Expr> {
        if self.peek().tok == Tok::Minus {
            let span = self.peek().span;
            if let Tok::Num(n) = *self.peek_at(1) {
                self.bump();
                self.bump();
                return Ok(Expr { kind: ExprKind::Num(-n), span });
            }
            self.bump();
            self.enter()?;
            let inner = self.unary()?;
            self.depth -= 1;
            return Ok(Expr { kind: ExprKind::Unary { op: UnOp::Neg, expr: Box::new(inner) }, span });
        }
        self.primary()
    }

    fn primary(&mut self) -> PResult<Expr> {
        let span = self.peek().span;
        let kind = match self.peek().tok.clone() {
            Tok::Num(n) => {
                self.bump();
                ExprKind::Num(n)
            }
            Tok::Str(s) => {
                self.bump();
                ExprKind::Str(s)
            }
            Tok::LParen => {
                self.bump();
                let inner = self.expr()?;
                self.expect(&Tok::RParen, "`)`")?;
                return Ok(Expr { kind: inner.kind, span });
            }
            Tok::Ident(word) if word.eq_ignore_ascii_case("true") => {
                self.bump();
                ExprKind::Bool(true)
            }
            Tok::Ident(word) if word.eq_ignore_ascii_case("false") => {
                self.bump();
                ExprKind::Bool(false)
            }
            Tok::Ident(_) => {
                if *self.peek_at(1) == Tok::LParen {
                    let (name, _) = self.ident("a function name")?;
                    let args = self.args()?;
                    ExprKind::Call { name, args }
                } else {
                    ExprKind::Path(self.path()?.0)
                }
            }
            _ => return Err(self.unexpected("a value, name, call or `(`")),
        };
        Ok(Expr { kind, span })
    }
}

fn joined(op: BinOp, lhs: Expr, rhs: Expr) -> Expr {
    let span = lhs.span;
    Expr { kind: ExprKind::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span }
}

fn cmp_op(tok: &Tok) -> Option<BinOp> {
    Some(match tok {
        Tok::Lt => BinOp::Lt,
        Tok::Le => BinOp::Le,
        Tok::Gt => BinOp::Gt,
        Tok::Ge => BinOp::Ge,
        Tok::EqEq => BinOp::Eq,
        Tok::NotEq => BinOp::Ne,
        Tok::Assign => BinOp::Eq,
        _ => return None,
    })
}

fn parser_for(source: &str) -> Result<Parser, Vec<Diagnostic>> {
    let toks = tokenize(source).map_err(|d| vec![d])?;
    Ok(Parser { toks, pos: 0, depth: 0 })
}

/// Parses a whole program (any number of rules). All syntax errors are
/// reported, one per broken rule.
pub fn parse_program(source: &str) -> Result<Vec<RuleAst>, Vec<Diagnostic>> {
    let mut p = parser_for(source)?;
    let mut rules = Vec::new();
    let mut errors = Vec::new();
    while !matches!(p.peek().tok, Tok::Eof) {
        // A failed rule may leave the depth counter raised; every rule starts afresh.
        p.depth = 0;
        if !p.at_kw("rule") {
            errors.push(p.unexpected("`rule`").help("every rule starts with `rule NAME for TYPE:`"));
            p.bump();
            p.skip_to_next_rule();
            continue;
        }
        let start = p.pos;
        match p.rule() {
            Ok(rule) => rules.push(rule),
            Err(d) => {
                errors.push(d);
                // Resume at the next `rule`, but never skip the `rule` keyword the
                // error stopped at (it starts the next rule), and always make progress.
                if p.pos == start || !p.at_kw("rule") {
                    p.bump();
                }
                p.skip_to_next_rule();
            }
        }
    }
    if errors.is_empty() { Ok(rules) } else { Err(errors) }
}

/// Parses exactly one rule.
pub fn parse_rule(source: &str) -> Result<RuleAst, Vec<Diagnostic>> {
    let mut rules = parse_program(source)?;
    match rules.len() {
        1 => Ok(rules.remove(0)),
        0 => Err(vec![
            Diagnostic::error("E015", "expected a rule, found nothing").help("write `rule NAME for TYPE: then ...`"),
        ]),
        n => Err(vec![
            Diagnostic::error("E015", format!("expected exactly one rule, found {n}"))
                .at(rules[1].span)
                .help("put each rule in its own entry"),
        ]),
    }
}

/// Parses a single expression (used for relationship conditions).
pub fn parse_expr(source: &str) -> Result<Expr, Vec<Diagnostic>> {
    let mut p = parser_for(source)?;
    let e = p.expr().map_err(|d| vec![d])?;
    if !matches!(p.peek().tok, Tok::Eof) {
        return Err(vec![p.unexpected("end of the expression")]);
    }
    Ok(e)
}
