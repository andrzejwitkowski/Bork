mod region_walk;
pub mod arena;
pub mod ast;
pub mod ast_number;
pub mod constants;
mod builtins;
pub mod defs;
mod codegen_gate;
#[cfg(feature = "codegen")]
pub mod codegen;
pub mod diag;
pub mod dump;
mod escape;
mod return_rules;
pub mod frontend;
pub mod hir;
pub mod infer;
mod layout;
pub mod mir;
mod prelude;
pub mod mono;
#[cfg(feature = "lsp")]
pub mod lsp;
pub mod sema;
pub mod span;
pub mod ty;
pub mod typeck;

use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub parser);

pub use ast::*;
pub use span::{Span, SpannedName};

use lalrpop_util::ParseError;
use layout::normalize_parenthesized_newlines;

pub type Error = ParseError<usize, String, &'static str>;

/// Parses and numbers the AST ([`ast_number::number_nodes`]).
pub fn parse(source: &str) -> Result<Program, Error> {
    parse_indexed(source).map(|(program, _)| program)
}

/// Parses without numbering the nodes (the prelude is numbered after the user's program).
pub(crate) fn parse_unnumbered(source: &str) -> Result<Program, Error> {
    let normalized = normalize_parenthesized_newlines(source);
    let input = normalized.as_deref().unwrap_or(source);
    parser::ProgramParser::new()
        .parse(input)
        .map_err(|err| err.map_token(|token| token.to_string()))
}

/// Like [`parse`], plus the `(Span, AstId)` index of every numbered node (for the LSP).
pub fn parse_indexed(source: &str) -> Result<(Program, Vec<(span::Span, span::AstId)>), Error> {
    let normalized = normalize_parenthesized_newlines(source);
    let input = normalized.as_deref().unwrap_or(source);
    let mut program = parser::ProgramParser::new()
        .parse(input)
        .map_err(|err| err.map_token(|token| token.to_string()))?;
    let intrinsic = |f: &Function| f.intrinsic;
    if program.functions.iter().any(intrinsic)
        || program.classes.iter().flat_map(|c| &c.methods).any(intrinsic)
        || program.impls.iter().flat_map(|i| &i.methods).any(intrinsic)
    {
        return Err(ParseError::User { error: "`intrinsic fun` is only allowed in the prelude" });
    }
    let index = ast_number::number_nodes(&mut program);
    Ok((program, index))
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

pub const PROCESS_USER_SAMPLE: &str = r#"
fun processUser(name: String?, score: i32): i32 {
    val fallbackName: String = name ?: "Guest"
    val finalScore = score

    {
        val verifiedUser: String? = Some(fallbackName)
        val emptyMiddle: String? = None

        val verifiedLength: i32 = verifiedUser?.length ?: 0
        if (verifiedLength > 0) {
            return finalScore
        }
    }

    return 0
}
"#;

#[cfg(test)]
mod intrinsic_syntax_tests {
    #[test]
    fn intrinsic_fun_parses_without_body_only_unnumbered() {
        let src = "class Bag<T> { intrinsic fun len(): i32 }";
        assert!(super::parse(src).is_err(), "user source must reject `intrinsic fun`");
        let program = super::parse_unnumbered(src).unwrap();
        assert!(program.classes[0].methods[0].intrinsic);
    }
}
