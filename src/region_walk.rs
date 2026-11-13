//! The kinds of region a report child can stand for, and the label each one has in the sema tree.
//! Sema builds the tree from the AST (`sema::peel_blocks`); MIR building takes the children in source order.

use crate::hir::{HirPat, HirPatKind};
use crate::ty::TyCtx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionSite {
    Block,
    MoveBlock,
    For,
    While,
    IfThen,
    IfElse,
    PresenceSome,
    PresenceNone,
    MatchArm,
    Closure,
}

impl RegionSite {
    pub fn matches(self, label: &str) -> bool {
        match self {
            RegionSite::Block => label == "Block",
            RegionSite::MoveBlock => label == "MoveBlock (move)",
            RegionSite::For => label.starts_with("ForLoop ("),
            RegionSite::While => label == "WhileLoop",
            RegionSite::IfThen => label == "IfThen",
            RegionSite::IfElse => label == "IfElse",
            RegionSite::PresenceSome => label == "IfValSome" || label == "WhenSome",
            RegionSite::PresenceNone => label == "IfValNone" || label == "WhenNone",
            // A `when` with `Some(x)` then `None` over an `Option` is checked as a `when` but ownership walks it as `if val`.
            RegionSite::MatchArm => matches!(label, "MatchArm" | "IfValSome" | "IfValNone"),
            RegionSite::Closure => label.starts_with("Closure"),
        }
    }
}

/// Whether a pattern binds a `Ref` by value: the binding owns a retained count, so its arm
/// needs an arena of its own to release it in.
pub(crate) fn pat_binds_owned_ref(tcx: &TyCtx, pat: &HirPat) -> bool {
    match &pat.kind {
        HirPatKind::Bind { binding_ty, .. } => tcx.is_managed_ref(*binding_ty),
        HirPatKind::Variant { fields, .. } => fields.iter().any(|f| pat_binds_owned_ref(tcx, f)),
        HirPatKind::Some(inner) => pat_binds_owned_ref(tcx, inner),
        HirPatKind::Wildcard | HirPatKind::Lit(_) | HirPatKind::None => false,
    }
}
