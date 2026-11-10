//! Mono MIR (RFC 0006). Steps so far: [`layout`] (M1) and the data types, printer and verifier (M2a).
//! Nothing consumes MIR yet.

pub mod body;
pub mod build;
pub mod capability;
pub mod emit;
pub mod escape;
pub mod layout;
pub mod plan;
pub mod print;
pub mod shadow;
pub mod verify;

pub use body::*;

#[cfg(test)]
mod tests;
