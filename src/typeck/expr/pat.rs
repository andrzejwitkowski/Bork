//! Pattern checking for `when`: each pattern is checked against the type of the value it matches.
//!
//! Binding mode: over a place (or a `&T`, such as `this`) a bound name is a borrow, `&T`, except
//! for `Copy` scalars, which bind by value. Over a temporary it binds by move. Nothing moves out
//! of a place.

use std::collections::HashSet;

use crate::ast::{BindingKind, PatLit, Pattern};
use crate::hir::{HirPat, HirPatKind, Prim};
use crate::span::Span;
use crate::ty::{TyData, TyId};

use super::super::env::{EnumInfo, Env};
use super::object::instantiate_field;

pub(super) fn check_pat(
    pat: &Pattern,
    ty: TyId,
    by_borrow: bool,
    names: &mut HashSet<String>,
    env: &mut Env<'_>,
) -> HirPat {
    let ty = env.settle(ty);
    let span = pat.span();
    let kind = match pat {
        Pattern::Wildcard { .. } => HirPatKind::Wildcard,
        Pattern::Binding(name) => match unit_variant(&name.name, ty, env) {
            Some((enum_name, index)) => HirPatKind::Variant { enum_name, variant: name.name.clone(), index, fields: Vec::new() },
            None => bind(name, ty, by_borrow, names, env),
        },
        Pattern::Literal { value, .. } => literal(value, ty, span, env),
        Pattern::Variant { .. } => return check_variant(pat, ty, by_borrow, names, env),
        Pattern::Some { inner, .. } => {
            let (inner_ty, inner_borrow) = match (env.tcx.managed_ref_inner(ty), env.tcx.is_nullable(ty)) {
                (Some(target), _) => (target, true),
                (None, true) => (env.tcx.with_nullable(ty, false), by_borrow),
                _ => {
                    if !env.tcx.is_unknown(ty) {
                        env.error(format!("`Some` pattern requires `Ref<T>` or `T?`, got {}", env.show(ty)), Some(span));
                    }
                    (env.tcx.error(), false)
                }
            };
            HirPatKind::Some(Box::new(check_pat(inner, inner_ty, inner_borrow, names, env)))
        }
        Pattern::None { .. } => {
            if !env.tcx.is_managed_ref(ty) && !env.tcx.is_nullable(ty) && !env.tcx.is_unknown(ty) {
                env.error(format!("`None` pattern requires `Ref<T>` or `T?`, got {}", env.show(ty)), Some(span));
            }
            HirPatKind::None
        }
    };
    HirPat { kind, ty, span }
}

/// A bare name that spells a payload-free variant of the matched enum is that variant.
fn unit_variant(name: &str, ty: TyId, env: &Env<'_>) -> Option<(String, usize)> {
    let (def, _) = env.tcx.enum_args(ty)?;
    let e = env.enums.get(&def)?;
    let index = e.variants.iter().position(|v| v.name == name)?;
    Some((e.name.clone(), index))
}

fn is_copy(ty: TyId, env: &Env<'_>) -> bool {
    env.tcx.is_numeric(ty) || matches!(env.tcx.data(ty), TyData::Prim(Prim::Bool))
}

fn bind(
    name: &crate::span::SpannedName,
    ty: TyId,
    by_borrow: bool,
    names: &mut HashSet<String>,
    env: &mut Env<'_>,
) -> HirPatKind {
    if !names.insert(name.name.clone()) {
        env.error(format!("duplicate pattern binding '{}'", name.name), Some(name.span));
    }
    let borrowed = by_borrow && !is_copy(ty, env) && !env.tcx.is_unknown(ty);
    let binding_ty = if borrowed { env.tcx.ref_to(ty) } else { ty };
    env.node_tys.insert(name.id.numbered(), binding_ty);
    env.bind(name.name.clone(), BindingKind::Val, binding_ty, borrowed);
    HirPatKind::Bind { name: name.clone(), binding_ty }
}

fn literal(value: &PatLit, ty: TyId, span: Span, env: &mut Env<'_>) -> HirPatKind {
    let found = match value {
        PatLit::Int(_) if env.tcx.is_integer(ty) || env.is_literal_var(ty, crate::infer::VarKind::Int) => ty,
        PatLit::Int(_) => env.tcx.i32(),
        PatLit::Bool(_) => env.tcx.bool(),
        PatLit::Str(_) => env.tcx.string(false),
    };
    env.mismatch("literal pattern", found, ty, Some(span));
    HirPatKind::Lit(value.clone())
}

fn check_variant(pat: &Pattern, ty: TyId, by_borrow: bool, names: &mut HashSet<String>, env: &mut Env<'_>) -> HirPat {
    let Pattern::Variant { enum_name, name, args, span } = pat else { unreachable!("caller matched a variant pattern") };
    let (enum_name, span) = (enum_name.as_ref(), *span);
    let rejected = |args: &[Pattern], names: &mut HashSet<String>, env: &mut Env<'_>| {
        // Sub-patterns still bind their names, so their uses in the arm do not cascade.
        let unknown = env.tcx.error();
        for arg in args {
            check_pat(arg, unknown, false, names, env);
        }
        HirPat { kind: HirPatKind::Wildcard, ty: unknown, span }
    };
    if env.tcx.is_unknown(ty) {
        return rejected(args, names, env);
    }
    let Some((def, type_args)) = env.tcx.enum_args(ty).map(|(def, a)| (def, a.to_vec())) else {
        env.error(format!("pattern `{}` is an enum variant, but the value has type {}", name.name, env.show(ty)), Some(span));
        return rejected(args, names, env);
    };
    let e: EnumInfo = env.enums.get(&def).expect("an enum type has a table entry").clone();
    if let Some(written) = enum_name.filter(|written| written.name != e.name) {
        env.error(format!("pattern names enum `{}`, but the value has type {}", written.name, env.show(ty)), Some(written.span));
        return rejected(args, names, env);
    }
    let Some(index) = e.variants.iter().position(|v| v.name == name.name) else {
        env.error(format!("enum `{}` has no variant `{}`", e.name, name.name), Some(name.span));
        return rejected(args, names, env);
    };
    let variant = &e.variants[index];
    if args.len() != variant.fields.len() {
        env.error(
            format!("variant `{}.{}` has {} field(s), the pattern has {}", e.name, name.name, variant.fields.len(), args.len()),
            Some(span),
        );
        return rejected(args, names, env);
    }
    let mut fields = Vec::with_capacity(args.len());
    for (field, arg) in variant.fields.iter().zip(args) {
        let field_ty = instantiate_field(field.ty, &type_args, &e.name, arg.span(), env);
        fields.push(check_pat(arg, field_ty, by_borrow, names, env));
    }
    HirPat {
        kind: HirPatKind::Variant { enum_name: e.name.clone(), variant: name.name.clone(), index, fields },
        ty,
        span,
    }
}
