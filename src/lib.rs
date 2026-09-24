pub mod arena;
pub mod ast;
mod builtins;
#[cfg(feature = "codegen")]
pub mod codegen;
pub mod diag;
pub mod dump;
mod escape;
pub mod frontend;
pub mod hoist;
pub mod hir;
mod layout;
#[cfg(feature = "lsp")]
pub mod lsp;
pub mod sema;
pub mod span;
pub mod typeck;

use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub parser);

pub use ast::*;
pub use span::{Span, SpannedName};

use lalrpop_util::ParseError;
use layout::normalize_parenthesized_newlines;

pub type Error = ParseError<usize, String, &'static str>;

pub fn parse(source: &str) -> Result<Program, Error> {
    let normalized = normalize_parenthesized_newlines(source);
    let input = normalized.as_deref().unwrap_or(source);
    parser::ProgramParser::new()
        .parse(input)
        .map_err(|err| err.map_token(|token| token.to_string()))
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
