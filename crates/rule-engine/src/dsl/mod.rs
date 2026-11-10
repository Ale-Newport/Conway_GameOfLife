//! The no-code rule language.
//!
//! * [`lexer`] — tokens with line/column spans.
//! * [`parser`] — recursive-descent parser (grammar in the module docs).
//! * [`ast`] — the authoring AST, serialisable for visual editors.
//! * [`printer`] — canonical pretty-printer that round-trips through the parser.
//!
//! Validation against a [`crate::LifeSystem`] (unknown names, types,
//! arities...) and lowering to the runtime IR live in [`mod@crate::compile`].

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod printer;

pub use ast::{Action, Arg, BinOp, Cost, Event, Expr, ExprKind, Name, Product, RuleAst, RuleMeta, UnOp};
pub use parser::{parse_expr, parse_program, parse_rule};
pub use printer::{print_expr, print_program, print_rule};

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
        # Bacteria divide when they have stored enough energy.
        rule divide for bacterium:
            when energy >= 12 and nearby(bacterium, radius=1.5) < 4
            then reproduce(probability=0.5)
            priority 300 cooldown 5 cost energy 1

        rule "flee predator" for rabbit:
            when nearby(fox, radius=6) > 0 || not (self.state == "resting")
            then move_away(nearest(fox, radius=6), speed=self.speed * 1.5), emit(alarm, 1)
            priority 900 probability 0.75 produces scared once;

        rule infect for cell: on_collision with virus
            then transform_into(infected) probability 0.3

        rule burst for infected: on_death then spawn(virus, 6)

        rule grow for plant: on_state_enter("seedling")
            when -age + 2 * (3 - x) / 4 > -1.5 and random() < 0.5
            then add self.size += 0.1, set self.speed = 0, add energy -= 1
    "#;

    #[test]
    fn parses_all_constructs() {
        let rules = parse_program(SAMPLE).unwrap();
        assert_eq!(rules.len(), 5);
        assert_eq!(rules[0].meta.priority, 300);
        assert_eq!(rules[0].meta.cooldown, Some(5));
        assert_eq!(rules[1].name, "flee predator");
        assert!(rules[1].meta.once);
        assert_eq!(rules[1].meta.produces[0].name.text, "scared");
        assert!(matches!(&rules[2].event, Event::Collision { with: Some(n) } if n.text == "virus"));
        assert_eq!(rules[3].event, Event::Death);
        assert_eq!(rules[4].actions.len(), 3);
        assert!(matches!(&rules[4].actions[2], Action::Add { subtract: true, .. }));
    }

    #[test]
    fn pretty_printer_round_trips() {
        let rules = parse_program(SAMPLE).unwrap();
        let printed = print_program(&rules);
        let again = parse_program(&printed).unwrap();
        assert_eq!(again, rules, "parse(print(ast)) must equal ast:\n{printed}");
        assert_eq!(print_program(&again), printed, "printing must be idempotent");
    }

    #[test]
    fn precedence_and_parentheses_are_preserved() {
        for src in [
            "a - (b - c)",
            "(a - b) - c",
            "a / (b * c)",
            "not (a and b) or c",
            "(not a) == b",
            "-(3) + -3 - --x",
            "a and (b and c)",
            "(a or b) and c",
            "(a < b) == true",
        ] {
            let e = parse_expr(src).unwrap();
            let printed = print_expr(&e);
            assert_eq!(parse_expr(&printed).unwrap(), e, "{src} -> {printed}");
        }
        assert_eq!(print_expr(&parse_expr("(a - b) - c").unwrap()), "a - b - c");
        assert_eq!(print_expr(&parse_expr("a - (b - c)").unwrap()), "a - (b - c)");
    }

    #[test]
    fn syntax_errors_have_positions_and_recovery() {
        let src = "rule a for x:\n  when energy > then die()\nrule b for x: then die() priority high\nrule c for x: then die()";
        let errors = parse_program(src).unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(errors[0].code, "E015");
        let span = errors[0].span.unwrap();
        assert_eq!((span.line, span.col), (2, 17));
        assert_eq!(errors[1].span.unwrap().line, 3);
        let chained = parse_expr("a < b < c").unwrap_err();
        assert!(chained[0].message.contains("chained"));
        let missing_then = parse_rule("rule a for x: when age > 3").unwrap_err();
        assert!(missing_then[0].message.contains("`then`"));
    }

    #[test]
    fn deep_nesting_is_rejected_not_overflowed() {
        let src = format!("{}1{}", "(".repeat(500), ")".repeat(500));
        let err = parse_expr(&src).unwrap_err();
        assert!(err[0].message.contains("nested deeper"));
    }

    #[test]
    fn long_operator_chains_count_as_nesting() {
        // A flat chain builds a left-nested tree as deep as the chain is long;
        // without a bound, a 1 MB rule overflowed the stack (process abort).
        let ok = format!("rule a for x: when {} > 0 then die()", vec!["energy"; 60].join(" + "));
        assert!(parse_rule(&ok).is_ok());
        for op in ["+", "*", "and", "or", "-", "/"] {
            let long = vec!["energy"; 50_000].join(&format!(" {op} "));
            let err = parse_expr(&long).unwrap_err();
            assert!(err[0].message.contains("nested deeper"), "{op}: {:?}", err[0]);
        }
        // Run on a small stack: the whole pipeline stays bounded.
        let h = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(|| {
                let src = format!("rule a for x: when {} > 0 then die()", vec!["energy"; 100_000].join(" + "));
                parse_rule(&src).is_err()
            })
            .unwrap();
        assert!(h.join().unwrap());
    }

    #[test]
    fn errors_do_not_leak_into_later_rules() {
        let mut src = String::new();
        for i in 0..40 {
            src.push_str(&format!("rule bad{i} for x: when ((((((energy > then die()\n"));
        }
        src.push_str(&format!("rule good for x: when {}energy{} > 0 then die()\n", "(".repeat(40), ")".repeat(40)));
        let errors = parse_program(&src).unwrap_err();
        assert_eq!(errors.len(), 40, "the valid rule must not inherit depth from failed ones: {errors:?}");
        // An error that stops at the next `rule` keyword does not swallow that rule.
        let errors = parse_program("rule a for x: when energy >\nrule b for x: when nonsense( then die()").unwrap_err();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert_eq!(errors[1].span.unwrap().line, 2);
    }

    #[test]
    fn misspelt_clause_keywords_get_a_hint() {
        let d = parse_rule("rule a for x: wehn energy > 1 then die()").unwrap_err();
        assert_eq!(d[0].help.as_deref(), Some("did you mean `when`?"), "{:?}", d[0]);
        let d = parse_rule("rule a for x: when energy > 1 thn die()").unwrap_err();
        assert_eq!(d[0].help.as_deref(), Some("did you mean `then`?"), "{:?}", d[0]);
    }

    #[test]
    fn extreme_numbers_print_compactly_and_round_trip() {
        for v in [1e300, -2.5e-9, 1e16, 123_456_789_012_345_680.0, 1e-6, 0.000_000_5, 5e-324, f64::MAX, 0.1, -0.0] {
            let printed = printer::print_number(v);
            assert!(printed.len() < 30, "{v}: {printed}");
            let back = parse_expr(&printed).unwrap();
            assert!(matches!(back.kind, ExprKind::Num(n) if n == v), "{v} -> {printed} -> {back:?}");
        }
        assert_eq!(printer::print_number(1e300), "1e300");
        assert_eq!(printer::print_number(12.5), "12.5");
    }

    #[test]
    fn ast_serialises_to_json_and_ron() {
        let rules = parse_program(SAMPLE).unwrap();
        let json = serde_json::to_string(&rules).unwrap();
        let back: Vec<RuleAst> = serde_json::from_str(&json).unwrap();
        assert_eq!(back, rules);
        let ron_text = ron::to_string(&rules).unwrap();
        let back: Vec<RuleAst> = ron::from_str(&ron_text).unwrap();
        assert_eq!(back, rules);
    }
}
