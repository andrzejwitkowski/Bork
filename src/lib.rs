pub mod ast;

use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub parser);

pub use ast::*;

use lalrpop_util::ParseError;
use parser::Token;

pub type Error<'input> = ParseError<usize, Token<'input>, &'static str>;

pub fn parse(source: &str) -> Result<Program, Error<'_>> {
    match normalize_parenthesized_newlines(source) {
        Some(normalized) => parser::ProgramParser::new()
            .parse(&normalized)
            .map_err(|_| ParseError::User {
                error: "parse error in parenthesized expression",
            }),
        None => parser::ProgramParser::new().parse(source),
    }
}

fn normalize_parenthesized_newlines(source: &str) -> Option<String> {
    let mut bytes = source.as_bytes().to_vec();
    let mut paren_brace_depths = Vec::new();
    let mut brace_depth = 0usize;
    let mut in_line_comment = false;
    let mut changed = false;
    let mut index = 0;

    while index < bytes.len() {
        match bytes[index] {
            b'\n' => {
                if paren_brace_depths.last() == Some(&brace_depth) {
                    bytes[index] = b'\r';
                    changed = true;
                }
                in_line_comment = false;
            }
            b'\r' => in_line_comment = false,
            b'/' if !in_line_comment && bytes.get(index + 1).copied() == Some(b'/') => {
                in_line_comment = true;
                index += 1;
            }
            b'(' if !in_line_comment => paren_brace_depths.push(brace_depth),
            b')' if !in_line_comment => {
                paren_brace_depths.pop();
            }
            b'{' if !in_line_comment => brace_depth += 1,
            b'}' if !in_line_comment => brace_depth = brace_depth.saturating_sub(1),
            _ => {}
        }
        index += 1;
    }

    changed.then(|| String::from_utf8(bytes).expect("source was valid UTF-8"))
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
    fn same_line_statements_require_a_separator() {
        assert!(parse("fun main(): Unit { val x = 1 var y = 2 }").is_err());
    }

    #[test]
    fn parses_final_expression_after_declaration() {
        let prog = parse("fun main() { val x = 1\n x }")
            .expect("newline should separate declaration and final expression");

        assert!(matches!(
            prog.functions[0].body.stmts.as_slice(),
            [Stmt::VarDecl { .. }, Stmt::Expr(Expr::Ident(name))] if name == "x"
        ));
    }

    #[test]
    fn parses_return_after_declaration() {
        let prog = parse("fun main() { val x = 1\n return x }")
            .expect("newline should separate declaration and return");

        assert!(matches!(
            prog.functions[0].body.stmts.as_slice(),
            [Stmt::VarDecl { .. }, Stmt::Return(Some(Expr::Ident(name)))] if name == "x"
        ));
    }

    #[test]
    fn parses_else_on_following_line() {
        parse("fun main() { if (c) { a }\n else { b } }")
            .expect("else may follow the if block on the next line");
    }

    #[test]
    fn parses_newlines_inside_parentheses() {
        parse(
            "fun apply(\n f: (Int,\n Int) -> Int,\n x: Int\n) {\n\
             val y = f(\n x,\n x\n)\n\
             if (\n y >\n x\n) { y } else { x }\n\
             for (\n i\n in\n 0..\n y\n) { i }\n}",
        )
        .expect("newlines inside parentheses should be insignificant");
    }

    #[test]
    fn newlines_in_blocks_nested_inside_parentheses_still_end_statements() {
        let prog = parse("fun main() { f(if (c) {\n val x = 1\n x\n} else { 0 }) }")
            .expect("a block nested in call arguments keeps statement newlines");

        let Stmt::Expr(Expr::Call { args, .. }) = &prog.functions[0].body.stmts[0] else {
            panic!("body should contain a call");
        };
        let Expr::If { then_block, .. } = &args[0] else {
            panic!("call argument should be an if expression");
        };
        assert_eq!(then_block.stmts.len(), 2);
    }

    #[test]
    fn newline_before_call_parenthesis_ends_statement() {
        let prog = parse("fun main() { val x = f\n (y) }")
            .expect("parenthesized expression should start the next statement");

        assert!(matches!(
            prog.functions[0].body.stmts.as_slice(),
            [Stmt::VarDecl { value: Expr::Ident(name), .. }, Stmt::Expr(Expr::Ident(y))]
                if name == "f" && y == "y"
        ));
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
