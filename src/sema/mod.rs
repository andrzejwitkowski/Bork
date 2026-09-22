//! Compile-time region / ownership analysis.

mod analyze;
mod env;
mod free_vars;
mod report;

#[cfg(test)]
mod tests;

pub use analyze::analyze;
pub use report::{ArenaNode, ArenaReport, BindingInfo, Ownership, SemaError};

use crate::ast::{Block, Stmt};

/// Peel pure nested bare-block wrappers; leave nested region structure intact.
pub fn collapse_block(block: Block) -> (Block, usize) {
    let mut compacted = 0;
    let mut current = block;
    while let [Stmt::Block(inner)] = current.stmts.as_slice() {
        compacted += 1;
        current = inner.clone();
    }
    (current, compacted)
}
