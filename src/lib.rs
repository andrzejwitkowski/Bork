pub mod ast;

use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub parser);

pub use ast::*;

use lalrpop_util::ParseError;
use parser::Token;

pub type Error<'input> = ParseError<usize, Token<'input>, &'static str>;

pub fn parse(source: &str) -> Result<Program, Error<'_>> {
    parser::ProgramParser::new().parse(source)
}

pub const MVP_SAMPLE: &str = r#"
fun action(a: Int, b: Int, block: (Int, Int) -> Int): Int {
    return block(a, b)
}

fun main() {
        var accumulator = 0
        val threshold = 5

        for (i in 0..10) {
            accumulator = action(accumulator, i) { acc, current ->
                if (current > threshold) {
                    acc + current
                } else {
                    acc
                }
            }
        }
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_parses() {
        assert!(parse("").is_ok());
    }

    #[test]
    fn parses_mvp_sample() {
        let prog = parse(MVP_SAMPLE).expect("sample should parse");
        assert_eq!(prog.functions.len(), 2);
        assert_eq!(prog.functions[0].name, "action");
        assert_eq!(prog.functions[1].name, "main");
        assert_eq!(
            prog.functions[1].return_type,
            Type::Named {
                name: "Unit".into(),
                nullable: false,
            }
        );
        assert!(matches!(
            prog.functions[0].params[2].ty,
            Type::Func { .. }
        ));
        assert!(matches!(prog.functions[1].body.stmts[2], Stmt::For { .. }));
    }

    #[test]
    fn parses_statements_without_newline_separators() {
        let prog = parse("fun main(): Unit { val x = 1 var y = 2 }")
            .expect("whitespace should separate statements");

        assert_eq!(prog.functions[0].body.stmts.len(), 2);
    }

    #[test]
    fn nullable_function_return_belongs_to_return_type() {
        let prog = parse("fun f(): () -> Int? { return 1 }")
            .expect("nullable function return type should parse");

        assert_eq!(
            prog.functions[0].return_type,
            Type::Func {
                params: vec![],
                ret: Box::new(Type::Named {
                    name: "Int".into(),
                    nullable: true,
                }),
                nullable: false,
            }
        );
    }

    #[test]
    fn nullable_function_type_as_param() {
        let prog = parse("fun f(x: ((Int) -> Int)?): Int { return 1 }")
            .expect("nullable function type param should parse");

        assert_eq!(
            prog.functions[0].params[0].ty,
            Type::Func {
                params: vec![Type::Named {
                    name: "Int".into(),
                    nullable: false,
                }],
                ret: Box::new(Type::Named {
                    name: "Int".into(),
                    nullable: false,
                }),
                nullable: true,
            }
        );
    }

    #[test]
    fn nullable_function_type_allows_space_before_question_mark() {
        let prog = parse("fun f(x: ((Int) -> Int) ?): Int { return 1 }")
            .expect("whitespace before function nullability should be insignificant");

        assert!(matches!(
            prog.functions[0].params[0].ty,
            Type::Func { nullable: true, .. }
        ));
    }

    #[test]
    fn nullable_function_types_compose_in_lists_and_returns() {
        let prog = parse(
            "fun f(x: (((Int) -> Int) ?, Int) -> ((Int) -> Int) ?): Int { return 1 }",
        )
        .expect("nullable function types should compose recursively");

        let Type::Func { params, ret, .. } = &prog.functions[0].params[0].ty else {
            panic!("outer parameter type should be a function");
        };
        assert!(matches!(
            params[0],
            Type::Func { nullable: true, .. }
        ));
        assert!(matches!(
            ret.as_ref(),
            Type::Func { nullable: true, .. }
        ));
    }

    #[test]
    fn nullable_parenthesized_named_type_is_a_parse_error_not_a_panic() {
        assert!(parse("fun f(x: (Int)?): Int { return 1 }").is_err());
    }
}
