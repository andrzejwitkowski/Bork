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
}
