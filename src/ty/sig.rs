//! Function signatures. `FnSig<TyId>` is the interned form keyed by `DefId`; typeck still uses
//! `FnSig<hir::Ty>` (as `FunSig`) keyed by name until it migrates (#36).

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

    #[test]
    fn signatures_are_keyed_by_def_id_and_round_trip() {
        let program = crate::parse("fun add(a: i32, b: i32): i64 {\n return 1\n}\nfun main() {}\n").unwrap();
        let defs = DefTable::build(&program);
        let mut c = crate::ty::TyCtx::new(&defs);
        let mut sigs = FnSigs::new();
        for (i, f) in program.functions.iter().enumerate() {
            let params = f
                .params
                .iter()
                .map(|p| c.intern(&crate::hir::Ty::from_ast(&p.ty)).unwrap())
                .collect();
            let ret = c.intern(&crate::hir::Ty::from_ast(&f.return_type)).unwrap();
            sigs.insert(defs.fn_id(i), FnSig::new(params, ret));
        }
        let add = &sigs[&defs.lookup_fn("add").unwrap()];
        assert_eq!(add.params.len(), 2);
        assert_eq!(c.to_ty(add.ret).to_string(), "i64");
        assert!(add.generics.is_empty());
    }
}
