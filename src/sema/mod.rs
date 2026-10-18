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

pub use analyze::{analyze, analyze_with_members, analyze_with_span_tys};
pub use report::{ArenaNode, ArenaReport, BindingInfo, Ownership, SemaError};

pub use crate::ast::peel_blocks;
