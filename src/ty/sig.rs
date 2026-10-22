//! Function signatures. `FnSig<TyId>` is the interned form keyed by `DefId`; typeck and
//! `builtins::signatures` build one per definition.

use std::collections::HashMap;

use super::ParamIdx;
use crate::defs::DefId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FnSig<T> {
    /// Type parameters this signature is generic over (none until Phase 3 syntax).
    pub generics: Vec<ParamIdx>,
    pub params: Vec<T>,
    pub ret: T,
}

impl<T> FnSig<T> {
    pub fn new(params: Vec<T>, ret: T) -> FnSig<T> {
        FnSig { generics: Vec::new(), params, ret }
    }
}

/// Signatures by definition.
pub type FnSigs = HashMap<DefId, FnSig<super::TyId>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::DefTable;
    use crate::hir::Prim;

    #[test]
    fn signatures_are_keyed_by_def_id() {
        let program = crate::parse("fun add(a: i32, b: i32): i64 {\n return 1\n}\nfun main() {}\n").unwrap();
        let defs = DefTable::build(&program);
        let mut c = crate::ty::TyCtx::new(&defs);
        let (i32_ty, i64_ty) = (c.i32(), c.prim(Prim::I64));
        let mut sigs = FnSigs::new();
        sigs.insert(defs.fn_id(0), FnSig::new(vec![i32_ty, i32_ty], i64_ty));
        let add = &sigs[&defs.lookup_fn("add").unwrap()];
        assert_eq!(add.params.len(), 2);
        assert_eq!(c.display(add.ret).to_string(), "i64");
        assert!(add.generics.is_empty());
    }
}
