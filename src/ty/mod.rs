//! Canonical types: `TyId` plus a hash-consing interner (RFC 0001 section 6).
//!
//! **Equality is id equality**, and ids are only comparable within the `TyCtx` that made them.
//! The context is owned by the compilation session; it is plain `Vec`/`HashMap`, never arena
//! memory. Every pass reads `TyId` through the `TyCtx` the frontend keeps (#36, #37).
//!
//! Shape choices:
//! - `T?` is the prelude's `Option<T>` (`TyCtx::nullable`), not a flag; `T??` is `Option<Option<T>>`.
//! - `Error` marks a type that already has a diagnostic; `Error?` is `Error` (an error absorbs, no
//!   cascade), so an unknown type stays unknown when made nullable. `Param` and `Infer` are for generics (Phase 3).
//! - `String` is its own variant, not a class: it has no `DefId`.
//! - `Named(name)` becomes `Class(DefId, args)`; a name that is not a class cannot be interned.

mod fold;
mod predicate;
mod print;
mod query;
mod sig;

use std::collections::HashMap;

use crate::defs::{DefId, DefTable};
use crate::hir::Prim;

pub use fold::{normalize, subst, super_fold, walk, TypeFolder, TypeVisitor};
pub use print::TyPrinter;
pub use predicate::{Predicate, TraitRef};
pub use sig::{FnSig, FnSigs};

/// Index of an interned type; only meaningful within the `TyCtx` that issued it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TyId(u32);

/// The shape of a type; children are ids, so equal shapes compare in O(1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TyData {
    Prim(Prim),
    String,
    /// A class with its type arguments (empty for a non-generic class).
    Class(DefId, Vec<TyId>),
    /// An enum with its type arguments (empty for a non-generic enum).
    Enum(DefId, Vec<TyId>),
    Array { elem: TyId, len: u32 },
    Func { params: Vec<TyId>, ret: TyId },
    Ref(TyId),
    ManagedRef(TyId),
    Vec(TyId),
    Range(TyId),
    /// A generic parameter, opaque inside its own item; reaches HIR only in generic items.
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

/// The error of folding a type. Nothing produces it any more (`T??` is an `Option<Option<T>>`); the
/// folder and resolver signatures keep it until they are simplified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NestedNullable;

const ALL_PRIMS: [Prim; 12] = [
    Prim::I8, Prim::I16, Prim::I32, Prim::I64, Prim::U8, Prim::U16,
    Prim::U32, Prim::U64, Prim::F32, Prim::F64, Prim::Bool, Prim::Unit,
];

/// The session-owned interner: one id per distinct `TyData`.
#[derive(Clone)]
pub struct TyCtx {
    types: Vec<TyData>,
    ids: HashMap<TyData, TyId>,
    class_names: HashMap<DefId, String>,
    /// Names of the type parameters of the item being checked, so messages say `T`, not `T0`.
    param_names: Vec<String>,
    /// The prelude's `Option`: `T?` is `Option<T>` (#54), so this is the enum every nullable type is.
    option_def: Option<DefId>,
}

impl TyCtx {
    pub fn set_param_names(&mut self, names: Vec<String>) {
        self.param_names = names;
    }

    /// A context that resolves class and enum names through a snapshot of `defs`.
    pub fn new(defs: &DefTable) -> TyCtx {
        let class_ids: HashMap<String, DefId> =
            defs.classes().chain(defs.enums()).map(|(name, id)| (name.to_string(), id)).collect();
        let class_names = class_ids.iter().map(|(name, id)| (*id, name.clone())).collect();
        let option_def = defs.lookup_option();
        let mut ctx = TyCtx { types: Vec::new(), ids: HashMap::new(), class_names, param_names: Vec::new(), option_def };
        // Read-only consumers (codegen) look these up with `prim_id` and cannot intern.
        for prim in ALL_PRIMS {
            ctx.add(TyData::Prim(prim));
        }
        ctx
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

    /// `T?`, which is `Option<T>`; `Error?` is `Error`. `T??` is `Option<Option<T>>` and is allowed
    /// (RFC 0001 forbade it only while `T?` was a flag on `T`).
    pub fn nullable(&mut self, id: TyId) -> Result<TyId, NestedNullable> {
        let Some(def) = self.option_def else { return Ok(self.add(TyData::Error)) };
        match self.data(id) {
            TyData::Error => Ok(id),
            _ => Ok(self.add(TyData::Enum(def, vec![id]))),
        }
    }

    /// The prelude's `Option` enum, when the program has one.
    pub fn option_def(&self) -> Option<DefId> {
        self.option_def
    }

    /// `Param(0)..Param(arity)`: the arguments that make a generic item refer to itself.
    pub fn identity_args(&mut self, arity: usize) -> Vec<TyId> {
        (0..arity as u32).map(|i| self.param(ParamIdx(i))).collect()
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

    /// A non-nullable primitive.
    pub fn is_copy(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Prim(_))
    }

    /// `T?` or a managed reference.
    pub fn is_presence_capable(&self, id: TyId) -> bool {
        self.is_nullable(id) || matches!(self.data(id), TyData::ManagedRef(_))
    }

    /// The type data of `id`, which must come from this context: a foreign id
    /// either panics (out of range) or silently names an unrelated type.
    pub fn data(&self, id: TyId) -> &TyData {
        &self.types[id.0 as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn ctx() -> TyCtx {
        let program = crate::parse("class Point {\n x: i32\n}\nclass Node {\n y: i32\n}\nenum Option<T> {\n Some(value: T)\n None\n}\nfun main() {}\n").unwrap();
        TyCtx::new(&DefTable::build(&program))
    }

    pub(super) fn named(c: &mut TyCtx, name: &str) -> TyId {
        let def = c.class_names.iter().find(|(_, n)| n.as_str() == name).map(|(d, _)| *d).unwrap();
        c.class(def, Vec::new())
    }

    const PRIMS: [Prim; 12] = [
        Prim::I8, Prim::I16, Prim::I32, Prim::I64, Prim::U8, Prim::U16,
        Prim::U32, Prim::U64, Prim::F32, Prim::F64, Prim::Bool, Prim::Unit,
    ];

    /// One of each shape, nested to depth 3 where it makes sense.
    pub(super) fn samples(c: &mut TyCtx) -> Vec<TyId> {
        let mut leaves: Vec<TyId> = PRIMS.iter().map(|p| c.prim(*p)).collect();
        let string = c.string(false);
        let (point, node, error) = (named(c, "Point"), named(c, "Node"), c.error());
        leaves.extend([string, point, node, error]);
        let mut all = leaves.clone();
        let i32_ty = c.i32();
        for &leaf in &leaves {
            let shapes = [
                c.array(leaf, 3),
                c.ref_to(leaf),
                c.managed_ref(leaf),
                c.vec_of(leaf),
                c.range(leaf),
                c.func(vec![], leaf),
                c.func(vec![leaf, i32_ty], leaf),
            ];
            all.extend(shapes);
        }
        let (point_ref, opt_string) = (c.ref_to(point), c.string(true));
        let (arr, vec_ty) = (c.array(point_ref, 2), c.vec_of(opt_string));
        let ret = c.with_nullable(vec_ty, true);
        all.push(c.func(vec![arr], ret));
        // every shape again nullable; `Error?` is `Error`, so it adds nothing
        let nullable: Vec<TyId> = all.clone().into_iter().map(|t| c.with_nullable(t, true)).collect();
        all.extend(nullable);
        all
    }

    #[test]
    fn interning_is_idempotent_and_equal_types_share_an_id() {
        let mut ctx = ctx();
        let first = samples(&mut ctx);
        let count = ctx.types.len();
        assert_eq!(samples(&mut ctx), first);
        assert_eq!(ctx.types.len(), count, "rebuilding added types");
    }

    #[test]
    fn distinct_types_get_distinct_ids() {
        let mut c = ctx();
        let (i32_ty, i64_ty, unit) = (c.i32(), c.prim(Prim::I64), c.unit());
        let (opt_i32, point, node) = (c.with_nullable(i32_ty, true), named(&mut c, "Point"), named(&mut c, "Node"));
        let (string, opt_string, unknown) = (c.string(false), c.string(true), c.error());
        let distinct = [
            i32_ty,
            i64_ty,
            opt_i32,
            c.array(i32_ty, 2),
            c.array(i32_ty, 3),
            c.ref_to(i32_ty),
            c.managed_ref(i32_ty),
            c.vec_of(i32_ty),
            c.range(i32_ty),
            point,
            node,
            string,
            opt_string,
            c.func(vec![i32_ty, i64_ty], unit),
            c.func(vec![i64_ty, i32_ty], unit),
            unknown,
        ];
        let ids: std::collections::HashSet<_> = distinct.iter().collect();
        assert_eq!(ids.len(), distinct.len());
    }

    #[test]
    fn nested_nullable_is_an_option_of_an_option_and_error_absorbs_nullable() {
        let mut ctx = ctx();
        let i = ctx.i32();
        let t = ctx.nullable(i).unwrap();
        let nested = ctx.nullable(t).unwrap();
        assert_ne!(nested, t);
        assert_eq!(ctx.base(nested), t);
        let err = ctx.error();
        assert_eq!(ctx.nullable(err), Ok(err));
        assert!(ctx.is_error(err) && !ctx.is_error(t));
    }

    #[test]
    fn nullable_ref_and_managed_ref_are_distinct_types() {
        let mut ctx = ctx();
        let i = ctx.i32();
        let opt = ctx.nullable(i).unwrap();
        let (r, m) = (ctx.ref_to(i), ctx.managed_ref(i));
        let ids = [i, opt, r, m];
        assert_eq!(ids.iter().collect::<std::collections::HashSet<_>>().len(), 4);
        assert_eq!(ctx.nullable(i).unwrap(), opt);
    }

    #[test]
    fn copy_and_presence_queries() {
        let mut ctx = ctx();
        let i = ctx.i32();
        let opt = ctx.nullable(i).unwrap();
        let (managed, string, point) = (ctx.managed_ref(i), ctx.string(false), named(&mut ctx, "Point"));
        assert!(ctx.is_copy(i) && !ctx.is_copy(opt) && !ctx.is_copy(string) && !ctx.is_copy(point));
        assert!(ctx.is_presence_capable(opt) && ctx.is_presence_capable(managed));
        assert!(!ctx.is_presence_capable(i) && !ctx.is_presence_capable(string));
    }

    #[test]
    fn primitives_are_available_without_interning() {
        let mut ctx = ctx();
        for prim in PRIMS {
            let id = ctx.prim_id(prim);
            assert_eq!(ctx.prim(prim), id);
        }
    }

    #[test]
    fn enum_arguments_are_visible_to_walks_and_substitution() {
        let program = crate::parse("enum Opt<T> {\n Empty\n Has(v: T)\n}\nfun main() {}\n").unwrap();
        let mut ctx = TyCtx::new(&DefTable::build(&program));
        let def = ctx.class_names.keys().copied().next().unwrap();
        let (param, i32_ty) = (ctx.param(ParamIdx(0)), ctx.i32());
        let generic = ctx.enum_ty(def, vec![param]);
        let concrete = ctx.enum_ty(def, vec![i32_ty]);
        assert_eq!(ctx.data(generic).children(), vec![param]);
        assert!(ctx.has_param(generic) && !ctx.has_param(concrete));
        assert_ne!(generic, concrete);
        assert_eq!(ctx.enum_args(concrete), Some((def, &[i32_ty][..])));
        assert_eq!(ctx.record_def(concrete), None);
        assert_eq!(ctx.display(concrete).to_string(), "Opt<i32>");
        assert_eq!(crate::ty::fold::subst(&mut ctx, generic, &[i32_ty]), Ok(concrete));
    }
}
