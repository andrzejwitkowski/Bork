//! Compile-time region / ownership analysis.

mod analyze;
mod env;
mod free_vars;
mod policy;
mod region;
mod report;
mod walk;

#[cfg(test)]
mod tests;

pub use analyze::{analyze, analyze_with_decl_tys};
pub use report::{ArenaNode, ArenaReport, BindingInfo, Ownership, SemaError};

use crate::ast::{Block, Stmt};

/// Peel nested bare-block wrappers by reference (must match `hir::peel_blocks`).
pub fn peel_blocks(block: &Block) -> (&Block, usize) {
    let mut compacted = 0;
    let mut current = block;
    while let [Stmt::Block(inner)] = current.stmts.as_slice() {
        compacted += 1;
        current = inner;
    }
    (current, compacted)
}
