//! Canonical types: `TyId` plus a hash-consing interner (RFC 0001 section 6).
//!
//! **Equality is id equality**, and ids are only comparable within the `TyCtx` that made them.
//! The context is owned by the compilation session; it is plain `Vec`/`HashMap`, never arena
//! memory. Typeck works in `TyId` (#36); later passes still read `hir::Ty` through the
//! `to_ty` bridge until they migrate (#37).
//!
//! Choices that differ from a naive mirror of `hir::TyKind`:
//! - `T?` is a constructor, `TyData::Nullable`, not a flag. `T??` is rejected (`TyCtx::nullable`).
//! - `Error` replaces `hir`'s `Unknown`; `Error?` is `Error` (an error absorbs, no cascade), so a
//!   hir `Unknown?` does not round trip. `Param` and `Infer` have no `hir::Ty` form: `to_ty` panics.
//! - `String` is its own variant, not a class: it has no `DefId`.
//! - `Named(name)` becomes `Class(DefId)`; a name that is not a class cannot be interned.

mod fold;
mod print;
mod query;
mod sig;

use std::collections::HashMap;

use crate::defs::{DefId, DefTable};
use crate::hir::{Prim, Ty, TyKind};

pub use fold::{normalize, subst, super_fold, walk, TypeFolder, TypeVisitor};
pub use print::TyPrinter;
pub use sig::{FnSig, FnSigs};

/// Index of an interned type; only meaningful within the `TyCtx` that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyId(u32);

/// The shape of a type; children are ids, so equal shapes compare in O(1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyData {
    Prim(Prim),
    String,
    Class(DefId),
    Array { elem: TyId, len: u32 },
    Func { params: Vec<TyId>, ret: TyId },
    Ref(TyId),
    ManagedRef(TyId),
    Vec(TyId),
    Range(TyId),
    /// `T?`; the inner type is never `Nullable` or `Error`.
    Nullable(TyId),
    /// A generic parameter (Phase 3); never reaches HIR.
    Param(ParamIdx),
    /// An inference variable, local to one body; never reaches HIR.
    Infer(InferVar),
    /// A type that already has a diagnostic; checks against it stay silent.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ParamIdx(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InferVar(pub u32);

/// `T?` of a type that is already nullable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NestedNullable;

/// `hir::Ty` named a class the definition table does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownClass(pub String);

const ALL_PRIMS: [Prim; 12] = [
    Prim::I8, Prim::I16, Prim::I32, Prim::I64, Prim::U8, Prim::U16,
    Prim::U32, Prim::U64, Prim::F32, Prim::F64, Prim::Bool, Prim::Unit,
];

/// The session-owned interner: one id per distinct `TyData`.
#[derive(Clone)]
pub struct TyCtx {
    types: Vec<TyData>,
    ids: HashMap<TyData, TyId>,
    class_ids: HashMap<String, DefId>,
    class_names: HashMap<DefId, String>,
}

impl TyCtx {
    /// A context that resolves class names through a snapshot of `defs`.
    pub fn new(defs: &DefTable) -> TyCtx {
        let class_ids: HashMap<String, DefId> =
            defs.classes().map(|(name, id)| (name.to_string(), id)).collect();
        let class_names = class_ids.iter().map(|(name, id)| (*id, name.clone())).collect();
        let mut ctx = TyCtx { types: Vec::new(), ids: HashMap::new(), class_ids, class_names };
        // Read-only consumers (codegen) look these up with `prim_id` and cannot intern.
        for prim in ALL_PRIMS {
            ctx.add(TyData::Prim(prim));
        }
        ctx
    }

    /// The canonical id of `ty`. On `Err` nothing was added to the context.
    pub fn intern(&mut self, ty: &Ty) -> Result<TyId, UnknownClass> {
        // Validate first so a failure leaves no half-interned children behind.
        self.check_classes(ty)?;
        Ok(self.intern_checked(ty))
    }

    /// Fails on the first class name `defs` does not know.
    fn check_classes(&self, ty: &Ty) -> Result<(), UnknownClass> {
        match &ty.kind {
            TyKind::Named(name) if name != "String" && !self.class_ids.contains_key(name) => {
                Err(UnknownClass(name.clone()))
            }
            TyKind::Array { elem, .. } => self.check_classes(elem),
            TyKind::Ref(inner) | TyKind::ManagedRef(inner) | TyKind::Vec(inner) | TyKind::Range(inner) => {
                self.check_classes(inner)
            }
            TyKind::Func { params, ret } => {
                params.iter().try_for_each(|p| self.check_classes(p))?;
                self.check_classes(ret)
            }
            _ => Ok(()),
        }
    }

    /// Interns bottom-up; every class name must already be validated.
    fn intern_checked(&mut self, ty: &Ty) -> TyId {
        let data = match &ty.kind {
            TyKind::Prim(prim) => TyData::Prim(*prim),
            TyKind::Named(name) if name == "String" => TyData::String,
            TyKind::Named(name) => TyData::Class(self.class_ids[name]),
            TyKind::Array { elem, len } => TyData::Array { elem: self.intern_checked(elem), len: *len },
            TyKind::Func { params, ret } => TyData::Func {
                params: params.iter().map(|p| self.intern_checked(p)).collect(),
                ret: self.intern_checked(ret),
            },
            TyKind::Ref(inner) => TyData::Ref(self.intern_checked(inner)),
            TyKind::ManagedRef(inner) => TyData::ManagedRef(self.intern_checked(inner)),
            TyKind::Vec(inner) => TyData::Vec(self.intern_checked(inner)),
            TyKind::Range(inner) => TyData::Range(self.intern_checked(inner)),
            TyKind::Unknown => TyData::Error,
        };
        let id = self.add(data);
        // hir's flag is never set on an already-nullable type, so this cannot fail.
        if ty.nullable { self.nullable(id).expect("hir types are never nested nullable") } else { id }
    }

    fn add(&mut self, data: TyData) -> TyId {
        if let Some(&id) = self.ids.get(&data) {
            return id;
        }
        let id = TyId(self.types.len() as u32);
        self.types.push(data.clone());
        self.ids.insert(data, id);
        id
    }

    /// `T?`. `T??` is an error (RFC 0001 section 3); `Error?` is `Error`.
    pub fn nullable(&mut self, id: TyId) -> Result<TyId, NestedNullable> {
        match self.data(id) {
            TyData::Nullable(_) => Err(NestedNullable),
            TyData::Error => Ok(id),
            _ => Ok(self.add(TyData::Nullable(id))),
        }
    }

    pub fn param(&mut self, param: ParamIdx) -> TyId {
        self.add(TyData::Param(param))
    }

    pub fn infer(&mut self, var: InferVar) -> TyId {
        self.add(TyData::Infer(var))
    }

    pub fn error(&mut self) -> TyId {
        self.add(TyData::Error)
    }

    pub fn is_error(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Error)
    }

    /// Mirrors `hir::Ty::is_copy`: a non-nullable primitive.
    pub fn is_copy(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Prim(_))
    }

    /// Mirrors `hir::Ty::is_presence_capable`: `T?` or a managed reference.
    pub fn is_presence_capable(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Nullable(_) | TyData::ManagedRef(_))
    }

    /// The type data of `id`, which must come from this context: a foreign id
    /// either panics (out of range) or silently names an unrelated type.
    pub fn data(&self, id: TyId) -> &TyData {
        &self.types[id.0 as usize]
    }

    /// Rebuilds the `hir::Ty` for `id`, which must come from this context (see [`TyCtx::data`]).
    pub fn to_ty(&self, id: TyId) -> Ty {
        let data = self.data(id);
        let boxed = |inner: &TyId| Box::new(self.to_ty(*inner));
        let kind = match data {
            TyData::Prim(prim) => TyKind::Prim(*prim),
            TyData::String => TyKind::Named("String".into()),
            TyData::Class(def) => TyKind::Named(self.class_names[def].clone()),
            TyData::Array { elem, len } => TyKind::Array { elem: boxed(elem), len: *len },
            TyData::Func { params, ret } => TyKind::Func {
                params: params.iter().map(|p| self.to_ty(*p)).collect(),
                ret: boxed(ret),
            },
            TyData::Ref(inner) => TyKind::Ref(boxed(inner)),
            TyData::ManagedRef(inner) => TyKind::ManagedRef(boxed(inner)),
            TyData::Vec(inner) => TyKind::Vec(boxed(inner)),
            TyData::Range(inner) => TyKind::Range(boxed(inner)),
            TyData::Error => TyKind::Unknown,
            TyData::Nullable(inner) => {
                let mut ty = self.to_ty(*inner);
                ty.nullable = true;
                return ty;
            }
            TyData::Param(_) | TyData::Infer(_) => panic!("Param/Infer are unreachable in HIR after typeck"),
        };
        Ty::new(kind, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn ctx() -> TyCtx {
        let program = crate::parse("class Point {\n x: i32\n}\nclass Node {\n y: i32\n}\nfun main() {}\n").unwrap();
        TyCtx::new(&DefTable::build(&program))
    }

    pub(super) fn named(n: &str) -> Ty {
        Ty::new(TyKind::Named(n.into()), false)
    }

    const PRIMS: [Prim; 12] = [
        Prim::I8, Prim::I16, Prim::I32, Prim::I64, Prim::U8, Prim::U16,
        Prim::U32, Prim::U64, Prim::F32, Prim::F64, Prim::Bool, Prim::Unit,
    ];

    /// One of each shape, nested to depth 3 where it makes sense.
    pub(super) fn samples() -> Vec<Ty> {
        let leaves: Vec<Ty> = PRIMS
            .iter()
            .map(|p| Ty::prim(*p))
            .chain([Ty::string(false), named("Point"), named("Node"), Ty::unknown()])
            .collect();
        let mut all = leaves.clone();
        for leaf in &leaves {
            let b = || Box::new(leaf.clone());
            all.extend([
                Ty::new(TyKind::Array { elem: b(), len: 3 }, false),
                Ty::new(TyKind::Ref(b()), false),
                Ty::new(TyKind::ManagedRef(b()), false),
                Ty::new(TyKind::Vec(b()), false),
                Ty::new(TyKind::Range(b()), false),
                Ty::new(TyKind::Func { params: vec![], ret: b() }, false),
                Ty::new(TyKind::Func { params: vec![leaf.clone(), Ty::i32()], ret: b() }, false),
            ]);
        }
        let deep = Ty::new(
            TyKind::Func {
                params: vec![Ty::new(TyKind::Array { elem: Box::new(Ty::new(TyKind::Ref(Box::new(named("Point"))), false)), len: 2 }, false)],
                ret: Box::new(Ty::new(TyKind::Vec(Box::new(Ty::string(true))), true)),
            },
            false,
        );
        all.push(deep);
        // every shape again with the nullable flag set
        // `Unknown?` is the one lossy case (Error absorbs nullable), so it is not a sample
        let nullable: Vec<Ty> = all
            .iter()
            .filter(|t| t.kind != TyKind::Unknown)
            .cloned()
            .map(|mut t| { t.nullable = true; t })
            .collect();
        all.extend(nullable);
        all
    }

    #[test]
    fn round_trip_is_lossless_for_every_shape() {
        let mut ctx = ctx();
        for ty in samples() {
            let id = ctx.intern(&ty).unwrap();
            assert_eq!(ctx.to_ty(id), ty, "{ty}");
        }
    }

    #[test]
    fn interning_is_idempotent_and_equal_types_share_an_id() {
        let mut ctx = ctx();
        for ty in samples() {
            let first = ctx.intern(&ty).unwrap();
            assert_eq!(ctx.intern(&ty.clone()).unwrap(), first);
        }
        let count = ctx.types.len();
        for ty in samples() {
            ctx.intern(&ty).unwrap();
        }
        assert_eq!(ctx.types.len(), count, "re-interning added types");
    }

    #[test]
    fn distinct_types_get_distinct_ids() {
        let mut ctx = ctx();
        let arr = |len| Ty::new(TyKind::Array { elem: Box::new(Ty::i32()), len }, false);
        let func = |params: Vec<Ty>| Ty::new(TyKind::Func { params, ret: Box::new(Ty::unit()) }, false);
        let distinct = [
            Ty::i32(),
            Ty::prim(Prim::I64),
            Ty::new(TyKind::Prim(Prim::I32), true),
            arr(2),
            arr(3),
            Ty::new(TyKind::Ref(Box::new(Ty::i32())), false),
            Ty::new(TyKind::ManagedRef(Box::new(Ty::i32())), false),
            Ty::new(TyKind::Vec(Box::new(Ty::i32())), false),
            Ty::range(Ty::i32()),
            named("Point"),
            named("Node"),
            Ty::string(false),
            Ty::string(true),
            func(vec![Ty::i32(), Ty::prim(Prim::I64)]),
            func(vec![Ty::prim(Prim::I64), Ty::i32()]),
            Ty::unknown(),
        ];
        let ids: std::collections::HashSet<_> = distinct.iter().map(|t| ctx.intern(t).unwrap()).collect();
        assert_eq!(ids.len(), distinct.len());
    }

    #[test]
    fn unknown_class_is_an_error_and_leaves_the_context_unchanged() {
        let mut ctx = ctx();
        ctx.intern(&Ty::i32()).unwrap();
        let before = ctx.types.len();
        let nested = Ty::new(TyKind::Vec(Box::new(Ty::new(TyKind::Ref(Box::new(named("Nope"))), false))), false);
        assert_eq!(ctx.intern(&nested), Err(UnknownClass("Nope".into())));
        assert_eq!(ctx.types.len(), before);
    }

    #[test]
    fn nested_nullable_is_rejected_and_error_absorbs_nullable() {
        let mut ctx = ctx();
        let t = ctx.intern(&Ty::new(TyKind::Prim(Prim::I32), true)).unwrap();
        let before = ctx.types.len();
        assert_eq!(ctx.nullable(t), Err(NestedNullable));
        assert_eq!(ctx.types.len(), before);
        let err = ctx.error();
        assert_eq!(ctx.nullable(err), Ok(err));
        assert!(ctx.is_error(err) && !ctx.is_error(t));
    }

    #[test]
    fn nullable_ref_and_managed_ref_are_distinct_types() {
        let mut ctx = ctx();
        let i = ctx.intern(&Ty::i32()).unwrap();
        let opt = ctx.nullable(i).unwrap();
        let r = ctx.intern(&Ty::new(TyKind::Ref(Box::new(Ty::i32())), false)).unwrap();
        let m = ctx.intern(&Ty::new(TyKind::ManagedRef(Box::new(Ty::i32())), false)).unwrap();
        let ids = [i, opt, r, m];
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 4);
        assert_eq!(ctx.nullable(i).unwrap(), opt);
    }

    #[test]
    fn copy_and_presence_queries_match_hir() {
        let mut ctx = ctx();
        for ty in samples() {
            let id = ctx.intern(&ty).unwrap();
            assert_eq!(ctx.is_copy(id), ty.is_copy(), "{ty}");
            assert_eq!(ctx.is_presence_capable(id), ty.is_presence_capable(), "{ty}");
        }
    }

    #[test]
    fn error_round_trips_to_unknown_and_unknown_nullable_is_absorbed() {
        let mut ctx = ctx();
        let id = ctx.intern(&Ty::new(TyKind::Unknown, true)).unwrap();
        assert!(ctx.is_error(id));
        assert_eq!(ctx.to_ty(id), Ty::unknown());
    }

    #[test]
    #[should_panic(expected = "unreachable in HIR")]
    fn param_has_no_hir_form() {
        let mut ctx = ctx();
        let id = ctx.param(ParamIdx(0));
        ctx.to_ty(id);
    }

    #[test]
    #[should_panic(expected = "unreachable in HIR")]
    fn infer_has_no_hir_form() {
        let mut ctx = ctx();
        let id = ctx.infer(InferVar(0));
        ctx.to_ty(id);
    }
}
