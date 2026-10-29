//! Trait predicates (#56): `Self: Trait<args>`. They are substituted like the rest of a signature.

use super::{subst, NestedNullable, TyCtx, TyId};
use crate::defs::DefId;

/// `Trait<args>`; `args` exclude the implicit `Self`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraitRef<T> {
    pub trait_id: DefId,
    pub args: Vec<T>,
}

/// The requirement that `self_ty` implements `trait_ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate<T> {
    pub self_ty: T,
    pub trait_ref: TraitRef<T>,
}

impl Predicate<TyId> {
    /// This predicate with `Param(i)` replaced by `args[i]`.
    pub fn subst(&self, tcx: &mut TyCtx, args: &[TyId]) -> Result<Predicate<TyId>, NestedNullable> {
        let trait_args = self.trait_ref.args.iter().map(|&a| subst(tcx, a, args)).collect::<Result<_, _>>()?;
        Ok(Predicate {
            self_ty: subst(tcx, self.self_ty, args)?,
            trait_ref: TraitRef { trait_id: self.trait_ref.trait_id, args: trait_args },
        })
    }
}
