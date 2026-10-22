//! Traversal and substitution over `TyData`. Rebuilding goes through the `TyCtx` constructors,
//! so results stay canonical (`T??` impossible, `Error?` is `Error`).

use super::{NestedNullable, TyCtx, TyData, TyId};

impl TyData {
    /// The direct child types, in source order.
    pub fn children(&self) -> Vec<TyId> {
        match self {
            TyData::Array { elem, .. } => vec![*elem],
            TyData::Func { params, ret } => params.iter().copied().chain([*ret]).collect(),
            TyData::Ref(t) | TyData::ManagedRef(t) | TyData::Vec(t) | TyData::Range(t) | TyData::Nullable(t) => {
                vec![*t]
            }
            _ => Vec::new(),
        }
    }
}

pub trait TypeVisitor {
    /// Override to inspect a node; call [`walk`] to continue into its children.
    fn visit(&mut self, ctx: &TyCtx, id: TyId) {
        walk(self, ctx, id);
    }
}

pub fn walk<V: TypeVisitor + ?Sized>(visitor: &mut V, ctx: &TyCtx, id: TyId) {
    for child in ctx.data(id).children() {
        visitor.visit(ctx, child);
    }
}

pub trait TypeFolder {
    /// Override to rewrite a node; call [`super_fold`] to fold its children.
    fn fold(&mut self, ctx: &mut TyCtx, id: TyId) -> Result<TyId, NestedNullable> {
        super_fold(self, ctx, id)
    }
}

/// Folds the children of `id` and rebuilds the node; leaves are returned as they are.
pub fn super_fold<F: TypeFolder + ?Sized>(folder: &mut F, ctx: &mut TyCtx, id: TyId) -> Result<TyId, NestedNullable> {
    Ok(match ctx.data(id).clone() {
        TyData::Array { elem, len } => {
            let elem = folder.fold(ctx, elem)?;
            ctx.add(TyData::Array { elem, len })
        }
        TyData::Func { params, ret } => {
            let params = params.into_iter().map(|p| folder.fold(ctx, p)).collect::<Result<_, _>>()?;
            let ret = folder.fold(ctx, ret)?;
            ctx.add(TyData::Func { params, ret })
        }
        TyData::Ref(t) => {
            let t = folder.fold(ctx, t)?;
            ctx.add(TyData::Ref(t))
        }
        TyData::ManagedRef(t) => {
            let t = folder.fold(ctx, t)?;
            ctx.add(TyData::ManagedRef(t))
        }
        TyData::Vec(t) => {
            let t = folder.fold(ctx, t)?;
            ctx.add(TyData::Vec(t))
        }
        TyData::Range(t) => {
            let t = folder.fold(ctx, t)?;
            ctx.add(TyData::Range(t))
        }
        TyData::Nullable(t) => {
            let t = folder.fold(ctx, t)?;
            ctx.nullable(t)?
        }
        TyData::Prim(_) | TyData::String | TyData::Class(_) | TyData::Param(_) | TyData::Infer(_) | TyData::Error => id,
    })
}

struct Subst<'a>(&'a [TyId]);

impl TypeFolder for Subst<'_> {
    fn fold(&mut self, ctx: &mut TyCtx, id: TyId) -> Result<TyId, NestedNullable> {
        match ctx.data(id) {
            TyData::Param(idx) => Ok(*self
                .0
                .get(idx.0 as usize)
                .unwrap_or_else(|| panic!("no substitution for type parameter {}", idx.0))),
            _ => super_fold(self, ctx, id),
        }
    }
}

/// Replaces `Param(i)` by `args[i]`. There are no binders in `TyData` and `args` are closed
/// types, so nothing can be captured. `Err` when an instantiation would make `T??`.
/// Panics if `id` mentions a parameter beyond `args` (a caller bug).
pub fn subst(ctx: &mut TyCtx, id: TyId, args: &[TyId]) -> Result<TyId, NestedNullable> {
    Subst(args).fold(ctx, id)
}

struct Normalize;
impl TypeFolder for Normalize {}

/// Re-canonicalises `id` through the constructors. Interned types are already canonical, so
/// this is the identity today; it is where a future rewrite rule would go.
pub fn normalize(ctx: &mut TyCtx, id: TyId) -> TyId {
    Normalize.fold(ctx, id).expect("a canonical type stays canonical")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::Prim;
    use crate::ty::tests::{ctx, samples};
    use crate::ty::ParamIdx;

    fn p(ctx: &mut TyCtx, n: u32) -> TyId {
        ctx.param(ParamIdx(n))
    }

    fn int(ctx: &mut TyCtx) -> TyId {
        ctx.i32()
    }

    #[test]
    fn subst_reaches_through_array_func_ref_and_nullable() {
        let mut c = ctx();
        let (t, u, i) = (p(&mut c, 0), p(&mut c, 1), int(&mut c));
        let b = c.prim(Prim::Bool);
        let arr = c.add(TyData::Array { elem: t, len: 3 });
        let func = c.add(TyData::Func { params: vec![t, u], ret: t });
        let r = c.add(TyData::Ref(t));
        let m = c.add(TyData::ManagedRef(t));
        let opt = c.nullable(t).unwrap();
        let vec_opt = c.add(TyData::Vec(opt));
        let text = |c: &TyCtx, id| c.display(id).to_string();
        for (ty, want) in [
            (arr, "[i32; 3]"),
            (func, "(i32, bool)->i32"),
            (r, "&i32"),
            (m, "Ref<i32>"),
            (opt, "i32?"),
            (vec_opt, "Vec<i32?>"),
        ] {
            let got = subst(&mut c, ty, &[i, b]).unwrap();
            assert_eq!(text(&c, got), want);
        }
    }

    #[test]
    fn subst_is_simultaneous_and_leaves_closed_types_alone() {
        let mut c = ctx();
        let (t, u, i) = (p(&mut c, 0), p(&mut c, 1), int(&mut c));
        let func = c.add(TyData::Func { params: vec![t], ret: u });
        // T := U, U := i32 must not chain: T becomes U (param 1), not i32
        let got = subst(&mut c, func, &[u, i]).unwrap();
        assert_eq!(got, c.add(TyData::Func { params: vec![u], ret: i }));
        assert_eq!(subst(&mut c, i, &[]).unwrap(), i);
    }

    #[test]
    fn subst_into_nullable_param_rejects_t_nested_and_absorbs_error() {
        let mut c = ctx();
        let t = p(&mut c, 0);
        let opt_t = c.nullable(t).unwrap();
        let i = int(&mut c);
        let opt_i = c.nullable(i).unwrap();
        assert_eq!(subst(&mut c, opt_t, &[opt_i]), Err(NestedNullable));
        assert_eq!(subst(&mut c, opt_t, &[i]).unwrap(), opt_i);
        let err = c.error();
        assert_eq!(subst(&mut c, opt_t, &[err]).unwrap(), err);
    }

    #[test]
    #[should_panic(expected = "no substitution for type parameter 1")]
    fn subst_out_of_range_panics() {
        let mut c = ctx();
        let u = p(&mut c, 1);
        let _ = subst(&mut c, u, &[]);
    }

    #[test]
    fn identity_fold_and_normalize_keep_every_sample() {
        let mut c = ctx();
        for id in samples(&mut c) {
            let shown = c.display(id).to_string();
            assert_eq!(Normalize.fold(&mut c, id), Ok(id), "{shown}");
            assert_eq!(normalize(&mut c, id), id, "{shown}");
        }
    }

    #[test]
    fn visitor_sees_every_node_occurrence() {
        struct Count(usize);
        impl TypeVisitor for Count {
            fn visit(&mut self, ctx: &TyCtx, id: TyId) {
                self.0 += 1;
                walk(self, ctx, id);
            }
        }
        let mut c = ctx();
        // (i32, [i32; 2]?) -> &i32 : func, i32, nullable, array, i32, ref, i32
        let i32_ty = c.i32();
        let array = c.array(i32_ty, 2);
        let opt_array = c.with_nullable(array, true);
        let ret = c.ref_to(i32_ty);
        let id = c.func(vec![i32_ty, opt_array], ret);
        let mut count = Count(0);
        count.visit(&c, id);
        assert_eq!(count.0, 7);
    }
}
