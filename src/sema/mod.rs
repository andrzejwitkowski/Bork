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

/// Peel pure nested bare-block wrappers by reference; leave nested region structure intact.
pub fn peel_blocks(block: &Block) -> (&Block, usize) {
    let mut compacted = 0;
    let mut current = block;
    while let [Stmt::Block(inner)] = current.stmts.as_slice() {
        compacted += 1;
        current = inner;
    }
    (current, compacted)
}

/// Owned wrapper around [`peel_blocks`] for callers that need a `Block` value.
pub fn collapse_block(block: Block) -> (Block, usize) {
    let (peeled, compacted) = peel_blocks(&block);
    (peeled.clone(), compacted)
}
