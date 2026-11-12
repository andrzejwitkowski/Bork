//! Mono MIR (RFC 0006). Steps so far: [`layout`] (M1) and the data types, printer and verifier (M2a).
//! Consumed by the default backend (`codegen/llvm/lower.rs`) and the plan analyses.

pub mod body;
pub mod borrows;
pub mod build;
pub mod capability;
pub mod emit;
pub mod escape;
pub mod layout;
pub mod plan;
pub mod print;
pub mod verify;

pub use body::*;

#[cfg(test)]
mod tests;
