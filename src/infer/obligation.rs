//! Pending trait obligations (#56). Inference holds the list so snapshots roll it back with the
//! variables; selecting from impls and bounds is the checker's job (`typeck::solve`), which keeps
//! unification free of traits.

use super::InferCtxt;
use crate::span::Span;
use crate::ty::{Predicate, TyId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Obligation {
    pub predicate: Predicate<TyId>,
    pub span: Option<Span>,
    /// How many impl bounds deep this requirement is; the solver stops at a fixed limit.
    pub depth: u32,
    /// The callee whose bound this is, for diagnostics.
    pub origin: String,
}

impl InferCtxt {
    pub fn register(&mut self, obligation: Obligation) {
        self.obligations.push(obligation);
    }

    /// Removes and returns every pending obligation.
    pub fn take_obligations(&mut self) -> Vec<Obligation> {
        std::mem::take(&mut self.obligations)
    }

    pub fn pending_obligations(&self) -> usize {
        self.obligations.len()
    }
}
