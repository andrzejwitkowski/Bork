//! Canonical types: `TyId` plus a hash-consing interner (RFC 0001 section 6).
//!
//! **Equality is id equality**, and ids are only comparable within the `TyCtx` that made them.
//! The context is owned by the compilation session; it is plain `Vec`/`HashMap`, never arena
//! memory. Nothing outside this module uses it yet: passes keep `hir::Ty` and migrate through
//! the `intern` / `to_ty` bridge (#36, #37).
//!
//! Choices that differ from a naive mirror of `hir::TyKind`:
//! - `nullable` stays a flag next to the data, as in `hir::Ty` (first-class nullable is #33).
//! - `String` is its own variant, not a class: it has no `DefId`.
//! - `Named(name)` becomes `Class(DefId)`; a name that is not a class cannot be interned.

use std::collections::HashMap;

use crate::defs::{DefId, DefTable};
use crate::hir::{Prim, Ty, TyKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyId(u32);

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
    Unknown,
}

/// `hir::Ty` named a class the definition table does not know.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownClass(pub String);

pub struct TyCtx {
    types: Vec<(TyData, bool)>,
    ids: HashMap<(TyData, bool), TyId>,
    class_ids: HashMap<String, DefId>,
    class_names: HashMap<DefId, String>,
}

impl TyCtx {
    pub fn new(defs: &DefTable) -> TyCtx {
        let class_ids: HashMap<String, DefId> =
            defs.classes().map(|(name, id)| (name.to_string(), id)).collect();
        let class_names = class_ids.iter().map(|(name, id)| (*id, name.clone())).collect();
        TyCtx { types: Vec::new(), ids: HashMap::new(), class_ids, class_names }
    }

    /// The canonical id of `ty`. On `Err` nothing was added to the context.
    pub fn intern(&mut self, ty: &Ty) -> Result<TyId, UnknownClass> {
        // Validate first so a failure leaves no half-interned children behind.
        self.check_classes(ty)?;
        Ok(self.intern_checked(ty))
    }

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
            TyKind::Unknown => TyData::Unknown,
        };
        let key = (data, ty.nullable);
        if let Some(&id) = self.ids.get(&key) {
            return id;
        }
        let id = TyId(self.types.len() as u32);
        self.types.push(key.clone());
        self.ids.insert(key, id);
        id
    }

    /// The type data and nullable flag of `id`. Panics on an id from another context.
    pub fn data(&self, id: TyId) -> (&TyData, bool) {
        let (data, nullable) = &self.types[id.0 as usize];
        (data, *nullable)
    }

    /// Rebuilds the `hir::Ty` for `id`. Panics on an id from another context.
    pub fn to_ty(&self, id: TyId) -> Ty {
        let (data, nullable) = self.data(id);
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
            TyData::Unknown => TyKind::Unknown,
        };
        Ty::new(kind, nullable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> TyCtx {
        let program = crate::parse("class Point {\n x: i32\n}\nclass Node {\n y: i32\n}\nfun main() {}\n").unwrap();
        TyCtx::new(&DefTable::build(&program))
    }

    fn named(n: &str) -> Ty {
        Ty::new(TyKind::Named(n.into()), false)
    }

    const PRIMS: [Prim; 12] = [
        Prim::I8, Prim::I16, Prim::I32, Prim::I64, Prim::U8, Prim::U16,
        Prim::U32, Prim::U64, Prim::F32, Prim::F64, Prim::Bool, Prim::Unit,
    ];

    /// One of each shape, nested to depth 3 where it makes sense.
    fn samples() -> Vec<Ty> {
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
        let nullable: Vec<Ty> = all.iter().cloned().map(|mut t| { t.nullable = true; t }).collect();
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
}
