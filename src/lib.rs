pub mod ast;

use lalrpop_util::lalrpop_mod;
lalrpop_mod!(pub parser);

pub use ast::*;

#[cfg(test)]
mod tests {
    #[test]
    fn empty_input_parses() {
        assert!(crate::parser::ProgramParser::new().parse("").is_ok());
    }
}
