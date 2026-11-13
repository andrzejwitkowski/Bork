//! LLVM from MIR (RFC 0006 M5a-d, #222-#225). Covers scalars, classes and enums (with `Ref` fields), the
//! `Ref` protocol, presence observations, scalar `Vec`s, arrays and slices, text, nullables, calls with
//! the hidden arena arguments, control flow and regions. [`supported`] checks a function before it is lowered, so a
//! function is never half lowered.

use std::collections::{HashMap, HashSet};

use inkwell::basic_block::BasicBlock;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{FloatPredicate, IntPredicate};

use crate::ast::BinOp;
use crate::diag::Diagnostic;
use crate::ast::PatLit;
use crate::hir::{HirFunction, Prim, VecOp};
use crate::mir::*;
use crate::ty::{TyData, TyId};

use super::call_abi::CallAbi;
use super::context::{is_unsigned, Codegen};
use super::declare::Callees;
use super::vec;
use super::not_yet_supported;

fn trace() -> bool {
    std::env::var_os("BORK_MIR_BACKEND_TRACE").is_some()
}

fn prim(cx: &Codegen<'_>, ty: TyId) -> Option<Prim> {
    // `&i32` is represented as the `i32` itself (`Codegen::basic_type`).
    match cx.tcx.data(cx.tcx.ref_inner(ty).unwrap_or(ty)) {
        TyData::Prim(p) => Some(*p),
        _ => None,
    }
}

fn is_unit(cx: &Codegen<'_>, ty: TyId) -> bool {
    prim(cx, ty) == Some(Prim::Unit)
}

fn is_float(cx: &Codegen<'_>, ty: TyId) -> bool {
    matches!(prim(cx, ty), Some(Prim::F32 | Prim::F64))
}

/// Element type of a `Vec` (or `&Vec`) of non-unit scalars, text or `Ref`s.
fn plain_vec(cx: &Codegen<'_>, ty: TyId) -> Option<TyId> {
    let ty = cx.tcx.ref_inner(ty).unwrap_or(ty);
    let elem = cx.tcx.vec_elem(ty)?;
    ((prim(cx, elem).is_some() && !is_unit(cx, elem)) || cx.tcx.is_text(elem) || cx.tcx.is_managed_ref(elem)).then_some(elem)
}

/// Element type of an array (or `&array`) of non-unit scalars, text or `Ref`s.
fn plain_array(cx: &Codegen<'_>, ty: TyId) -> Option<TyId> {
    let elem = cx.tcx.array_elem(ty)?;
    ((prim(cx, elem).is_some() && !is_unit(cx, elem)) || cx.tcx.is_text(elem) || cx.tcx.is_managed_ref(elem)).then_some(elem)
}

/// Element type of an indexable buffer: a plain array, or `u8` for text.
fn buffer_elem(cx: &Codegen<'_>, ty: TyId) -> Option<TyId> {
    plain_array(cx, ty).or_else(|| cx.tcx.is_text(ty).then(|| cx.tcx.prim_id(Prim::U8)))
}

/// A value this lowering handles: scalars, `Ref`s, text, plain arrays and `Vec`s, and classes,
/// enums and nullables built only from those.
fn plain_value(cx: &Codegen<'_>, ty: TyId) -> bool {
    prim(cx, ty).is_some()
        || cx.tcx.is_managed_ref(ty)
        || cx.tcx.is_text(ty)
        || plain_array(cx, ty).is_some()
        || plain_vec(cx, ty).is_some()
        || plain_class(cx, ty).is_some()
        || plain_enum(cx, ty).is_some()
}

/// Answers depend only on the type, so they are memoised on `cx` (keyed with the reference
/// stripped). The entry is seeded with `None` while it is computed, so a recursive type is not plain.
fn memo_plain(
    cx: &Codegen<'_>,
    cache: &std::cell::RefCell<std::collections::HashMap<TyId, Option<String>>>,
    ty: TyId,
    compute: impl FnOnce(TyId) -> Option<String>,
) -> Option<String> {
    let ty = cx.tcx.ref_inner(ty).unwrap_or(ty);
    if let Some(hit) = cache.borrow().get(&ty) {
        return hit.clone();
    }
    cache.borrow_mut().insert(ty, None);
    let found = compute(ty);
    cache.borrow_mut().insert(ty, found.clone());
    found
}

/// A class whose fields are all plain values.
fn plain_class(cx: &Codegen<'_>, ty: TyId) -> Option<String> {
    memo_plain(cx, &cx.plain_classes, ty, |ty| {
        let name = crate::mono::class_instance_name(&cx.tcx, ty)?;
        cx.record_type(&name)?;
        let class = cx.class_by_name(&name)?;
        for i in 0..class.fields.len() {
            plain_value(cx, cx.record_field_type(&name, i)?).then_some(())?;
        }
        Some(name)
    })
}

/// An enum (or `T?`) whose variants carry only plain payloads.
fn plain_enum(cx: &Codegen<'_>, ty: TyId) -> Option<String> {
    memo_plain(cx, &cx.plain_enums, ty, |ty| {
        let name = crate::mono::enum_instance_name(&cx.tcx, ty)?;
        cx.enum_layout(&name)?;
        let e = cx.enums.iter().find(|e| e.name == name)?;
        // A `unit` payload takes no storage but keeps its field index (`enum_layout_of`).
        let plain = e.variants.iter().flat_map(|v| &v.fields).all(|f| plain_value(cx, f.ty));
        plain.then_some(name)
    })
}

/// `Some` payload type of nullable `ty`, when it is a plain `Option`.
fn nullable_payload(cx: &Codegen<'_>, ty: TyId) -> Option<TyId> {
    let ty = cx.tcx.ref_inner(ty).unwrap_or(ty);
    if !cx.tcx.is_nullable(ty) {
        return None;
    }
    plain_enum(cx, ty)?;
    cx.option_parts(ty).ok().map(|(_, payload)| payload)
}

/// Integer operand that can index a buffer: an `Int` constant or a bare integer local.
fn index_operand(cx: &Codegen<'_>, body: &Body, op: &Operand) -> Option<()> {
    match op {
        Operand::Const(Const::Int(_)) => Some(()),
        Operand::Copy(p) | Operand::Move(p) if p.is_bare() => cx.int_type(body.local(p.local)?.ty).map(|_| ()),
        _ => None,
    }
}

/// Type of `place`: a local, then fields over plain classes, enum payloads, `Some` payloads and
/// buffer elements.
fn place_ty(cx: &Codegen<'_>, body: &Body, p: &Place) -> Option<TyId> {
    let mut ty = body.local(p.local)?.ty;
    let mut variant = None;
    for pr in &p.proj {
        match (pr, variant) {
            (Proj::Variant(v), None) => {
                plain_enum(cx, ty)?;
                variant = Some(*v);
            }
            (Proj::Field(i), Some(v)) => {
                let name = plain_enum(cx, ty)?;
                ty = cx.enums.iter().find(|e| e.name == name)?.variants.get(v as usize)?.fields.get(*i as usize)?.ty;
                variant = None;
            }
            (Proj::Field(i), None) => {
                let name = plain_class(cx, ty)?;
                ty = cx.record_field_type(&name, *i as usize)?;
            }
            (Proj::Index(idx), None) => {
                index_operand(cx, body, idx)?;
                ty = buffer_elem(cx, ty)?;
            }
            (Proj::Some, None) => ty = nullable_payload(cx, ty)?,
            (Proj::Deref, None) if !cx.tcx.is_managed_ref(ty) => {}
            _ => return None,
        }
    }
    if variant.is_some() {
        return None;
    }
    Some(ty)
}

/// Region each `Ref`, text or array local was declared in; its home arena is that region's (as `declare_local`).
/// These values carry no arena of their own, unlike objects, enums and `Vec` headers.
fn ref_homes(cx: &Codegen<'_>, body: &Body) -> HashMap<Local, RegionId> {
    let mut homes: HashMap<Local, RegionId> = HashMap::new();
    let homed = |ty: TyId| cx.tcx.is_managed_ref(ty) || cx.tcx.is_text(ty) || plain_array(cx, ty).is_some();
    for (i, l) in body.locals.iter().enumerate() {
        if l.kind == LocalKind::Arg && homed(l.ty) {
            homes.insert(Local(i as u32), RegionId(0));
        }
    }
    for s in body.blocks.iter().flat_map(|b| &b.stmts) {
        if let StmtKind::Assign(dst, _) = &s.kind {
            if dst.is_bare() && body.local(dst.local).is_some_and(|l| homed(l.ty)) {
                homes.entry(dst.local).or_insert(s.region);
            }
        }
    }
    homes
}

/// Regions that some `RegionEnter` actually pushes (the others own no arena).
fn pushed_regions(body: &Body) -> HashSet<RegionId> {
    let mut set = HashSet::new();
    for block in &body.blocks {
        for s in &block.stmts {
            if let StmtKind::RegionEnter(r) = s.kind {
                set.insert(r);
            }
        }
    }
    set
}

pub(super) fn supported(cx: &Codegen<'_>, callees: &Callees<'_, '_>, function: &HirFunction, body: &Body) -> bool {
    check(cx, callees, function, body).is_some()
}

fn check(cx: &Codegen<'_>, callees: &Callees<'_, '_>, function: &HirFunction, body: &Body) -> Option<()> {
    let scalar = |ty: TyId| prim(cx, ty).map(|_| ());
    let value = |ty: TyId| plain_value(cx, ty).then_some(());
    let why = |what: &str, ty: TyId| {
        if trace() {
            eprintln!("mir-backend: unsupported {what} type `{}`", cx.tcx.display(ty));
        }
    };
    for l in &body.locals {
        match l.kind {
            LocalKind::Hidden(HiddenKind::RegionArena | HiddenKind::DynamicArena | HiddenKind::DebugScope) => {}
            LocalKind::Hidden(k) => {
                if trace() {
                    eprintln!("mir-backend: unsupported hidden local {k:?}");
                }
                return None;
            }
            _ => value(l.ty).or_else(|| {
                why("local", l.ty);
                None
            })?,
        }
    }
    for p in &function.params {
        value(p.ty).or_else(|| {
            why("param", p.ty);
            None
        })?;
    }
    value(function.return_ty).or_else(|| {
        why("return", function.return_ty);
        None
    })?;
    if body.locals.iter().filter(|l| l.kind == LocalKind::Arg).count() != function.params.len() {
        return None;
    }
    let pushed = pushed_regions(body);
    // `current_arena` needs an enclosing pushed region, and the function's own region is always pushed.
    if !pushed.contains(&RegionId(0)) {
        return None;
    }
    // Any value operand: bare or field-projected place, or a scalar constant.
    let general = |op: &Operand| -> Option<()> {
        match op {
            Operand::Copy(p) | Operand::Move(p) | Operand::Reborrow(p) => value(place_ty(cx, body, p)?),
            Operand::Const(Const::Int(_) | Const::Float(_) | Const::Bool(_) | Const::Unit | Const::Str(_)) => Some(()),
            _ => None,
        }
    };
    // Arithmetic, casts and printing need scalars (`Const::Str` is not one).
    let operand = |op: &Operand| -> Option<()> {
        general(op)?;
        match op.place() {
            Some(p) => scalar(place_ty(cx, body, p)?),
            None => (!matches!(op, Operand::Const(Const::Str(_)))).then_some(()),
        }
    };
    // Text: a string constant or a place of text type.
    let text = |op: &Operand| -> Option<()> {
        match op {
            Operand::Const(Const::Str(_)) => Some(()),
            _ => op.place().filter(|p| place_ty(cx, body, p).is_some_and(|t| cx.tcx.is_text(t))).map(|_| ()),
        }
    };
    let own_abi = CallAbi::of(cx, function);
    // Arenas lowering can name: the enclosing pushed region, the root, or a dynamic arena local.
    let homes = ref_homes(cx, body);
    let arena_ok = |a: &ArenaRef| match a {
        ArenaRef::Current | ArenaRef::Root => true,
        ArenaRef::Fresh(n) => body.local(Local(*n)).is_some_and(|l| l.kind == LocalKind::Hidden(HiddenKind::DynamicArena)),
        ArenaRef::Region(r) => pushed.contains(r),
        ArenaRef::ResultSink => own_abi.has_result_sink(),
        ArenaRef::CallerParent => own_abi.hidden_len() > 0,
        // The arena a binding lives in is readable from its handle (objects, enums, `Vec` headers);
        // text and arrays carry no arena of their own.
        ArenaRef::Home(l) if homes.contains_key(l) => true,
        ArenaRef::Home(l) | ArenaRef::ObjectArena(l) => body.local(*l).is_some_and(|d| {
            // A view of a class or enum (`&T`, observed `Ref`) is still the `{arena, object}` handle.
            plain_class(cx, d.ty).is_some() || plain_enum(cx, d.ty).is_some() || plain_vec(cx, d.ty).is_some()
        }),
    };
    for block in &body.blocks {
        for s in &block.stmts {
            let verdict = (|| -> Option<()> {
            match &s.kind {
                StmtKind::Assign(dst, rv) => {
                    let dty = place_ty(cx, body, dst)?;
                    match rv {
                        Rvalue::Use(Operand::Const(Const::None)) if cx.tcx.is_managed_ref(dty) => {}
                        Rvalue::Use(a) => {
                            general(a)?;
                            if a.place().is_none() && plain_class(cx, dty).is_some() {
                                return None;
                            }
                        }
                        _ if !dst.is_bare() => return None,
                        Rvalue::Unary(_, a) => operand(a)?,
                        Rvalue::Binary(op, a, b) => {
                            if !matches!(op, BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le | BinOp::Eq | BinOp::Ne) {
                                return None;
                            }
                            operand(a)?;
                            operand(b)?;
                            if is_unit(cx, dty) {
                                return None;
                            }
                        }
                        Rvalue::Cast(a, to) => {
                            operand(a)?;
                            cx.int_type(*to)?;
                            if let Some(p) = a.place() {
                                cx.int_type(place_ty(cx, body, p)?)?;
                            }
                        }
                        Rvalue::Len(p) => {
                            let t = place_ty(cx, body, p)?;
                            if !(cx.tcx.is_text(t) || plain_array(cx, t).is_some()) || cx.int_type(dty).is_none() {
                                return None;
                            }
                        }
                        Rvalue::Slice { base, lo, hi } => {
                            let t = place_ty(cx, body, base.place()?)?;
                            if plain_array(cx, t).is_none() || plain_array(cx, dty).is_none() || cx.tcx.array_len(dty).is_none() {
                                return None;
                            }
                            index_operand(cx, body, lo)?;
                            index_operand(cx, body, hi)?;
                        }
                        _ => return None,
                    }
                }
                StmtKind::RegionEnter(r) | StmtKind::RegionExit(r) | StmtKind::RegionReset { region: r, .. } if !pushed.contains(r) => return None,
                StmtKind::RegionEnter(_) | StmtKind::RegionExit(_) | StmtKind::RegionReset { .. } => {}
                StmtKind::StorageLive(_) | StmtKind::StorageDead(_) => {}
                StmtKind::ArenaCreateDynamic { parent, .. } if arena_ok(parent) => {}
                StmtKind::RegisterStrongRoot { owner, .. } if arena_ok(owner) => {}
                StmtKind::Borrow { dst, src, .. } | StmtKind::Move { dst, src } if dst.is_bare() && (src.is_bare() || matches!(s.kind, StmtKind::Borrow { .. })) => {
                    // Handles and descriptors copy bit for bit; a `Ref` may only move out of a temporary.
                    let (st, dt) = (place_ty(cx, body, src)?, body.local(dst.local)?.ty);
                    if !plain_value(cx, st) || !plain_value(cx, dt) || is_unit(cx, st) || is_unit(cx, dt) {
                        return None;
                    }
                    if cx.tcx.is_managed_ref(st) && matches!(s.kind, StmtKind::Move { .. }) && body.local(src.local)?.kind != LocalKind::Temp {
                        return None;
                    }
                }
                StmtKind::Ref(op) => {
                    let managed = |p: &Place| place_ty(cx, body, p).filter(|t| cx.tcx.is_managed_ref(*t)).map(|_| ());
                    let refv = |o: &Operand| match o {
                        Operand::Const(Const::None) => Some(()),
                        _ => general(o).and(managed(o.place()?)),
                    };
                    match op {
                        RefOp::Create { dst, source, target, object } => {
                            managed(dst)?;
                            arena_ok(source).then_some(())?;
                            arena_ok(target).then_some(())?;
                            general(object)?;
                            let oty = place_ty(cx, body, object.place()?)?;
                            (plain_class(cx, oty).is_some() || plain_enum(cx, oty).is_some()).then_some(())?;
                        }
                        RefOp::Clone { dst, src } => {
                            managed(dst)?;
                            refv(src)?;
                        }
                        RefOp::MoveOut { dst, src } => {
                            managed(dst)?;
                            managed(src)?;
                        }
                        RefOp::Store { slot, owner, value, .. } => {
                            managed(slot)?;
                            arena_ok(owner).then_some(())?;
                            refv(value)?;
                        }
                        RefOp::Rehome { dst, owner, value } => {
                            managed(dst)?;
                            arena_ok(owner).then_some(())?;
                            refv(value)?;
                        }
                        RefOp::Drop(v) => refv(v)?,
                        RefOp::Eq { dst, lhs, rhs } => {
                            dst.is_bare().then_some(())?;
                            refv(lhs)?;
                            refv(rhs)?;
                        }
                        RefOp::Count { dst, value, .. } => {
                            cx.int_type(place_ty(cx, body, dst)?)?;
                            refv(value)?;
                        }
                        RefOp::IsNone { dst, value } => {
                            dst.is_bare().then_some(())?;
                            refv(value)?;
                        }
                    }
                }
                StmtKind::Observe { object, absent, value, .. } => {
                    let vty = place_ty(cx, body, value.place()?)?;
                    cx.tcx.is_managed_ref(vty).then_some(())?;
                    general(value)?;
                    let oty = body.local(*object)?.ty;
                    (plain_class(cx, oty).is_some() || plain_enum(cx, oty).is_some()).then_some(())?;
                    cx.int_type(body.local(*absent)?.ty)?;
                }
                StmtKind::ObservationRegister { arena, .. } if arena_ok(arena) => {}
                StmtKind::ObservationRelease(_) => {}
                StmtKind::RegisterDrop { arena, place, glue } if arena_ok(arena) => {
                    let t = place_ty(cx, body, place)?;
                    match glue {
                        DropGlue::RefSlot => cx.tcx.is_managed_ref(t).then_some(())?,
                        DropGlue::Enum(n) => (plain_enum(cx, t)? == *n).then_some(())?,
                        DropGlue::Vec(_) => plain_vec(cx, t).map(|_| ())?,
                    }
                }
                StmtKind::Alloc { dst, kind: AllocKind::EnumSlot { enum_name, variant }, arena } if dst.is_bare() && arena_ok(arena) => {
                    let ty = body.local(dst.local)?.ty;
                    if plain_enum(cx, ty)? != *enum_name || cx.enum_layout(enum_name)?.variants.get(*variant as usize).is_none() {
                        return None;
                    }
                }
                StmtKind::Alloc { dst, kind: AllocKind::Object { class }, arena } if dst.is_bare() && arena_ok(arena) => {
                    let ty = body.local(dst.local)?.ty;
                    if plain_class(cx, ty)? != *class {
                        return None;
                    }
                }
                StmtKind::Alloc { dst, kind: AllocKind::Vec { cap }, arena } if dst.is_bare() && arena_ok(arena) => {
                    plain_vec(cx, body.local(dst.local)?.ty)?;
                    if let Some(c) = cap {
                        operand(c)?;
                    }
                }
                StmtKind::VecOp { dst, op, receiver, args, arena } => {
                    if dst.as_ref().is_some_and(|d| !d.is_bare()) {
                        return None;
                    }
                    let index = |op: &Operand| index_operand(cx, body, op);
                    // An element to store: a scalar, text, or a `Ref` (or `None`) for a `Vec` of `Ref`s.
                    let elem = plain_vec(cx, body.local(receiver.place().filter(|p| p.is_bare())?.local)?.ty)?;
                    let element = |op: &Operand| -> Option<()> {
                        if cx.tcx.is_managed_ref(elem) {
                            match op {
                                Operand::Const(Const::None) => Some(()),
                                _ => general(op).and(place_ty(cx, body, op.place()?).filter(|t| cx.tcx.is_managed_ref(*t)).map(|_| ())),
                            }
                        } else if cx.tcx.is_text(elem) {
                            text(op)
                        } else {
                            operand(op)
                        }
                    };
                    match (op, args.as_slice()) {
                        (VecOp::Len | VecOp::Capacity | VecOp::Clear, []) => {}
                        (VecOp::Pop, []) => {
                            let d = dst.as_ref()?;
                            let dty = body.local(d.local)?.ty;
                            if cx.tcx.is_managed_ref(elem) {
                                cx.tcx.is_managed_ref(dty).then_some(())?;
                            } else {
                                // The `Some` payload is the element, so the slot is a plain `Option<elem>` in an arena we can name.
                                nullable_payload(cx, dty).filter(|p| *p == elem)?;
                                arena.as_ref().filter(|a| arena_ok(a))?;
                            }
                        }
                        (VecOp::Get, [i]) => index(i)?,
                        (VecOp::Push, [v]) => element(v)?,
                        (VecOp::Set, [i, v]) => {
                            index(i)?;
                            element(v)?;
                        }
                        _ => return None,
                    }
                }
                StmtKind::Alloc { dst, kind: AllocKind::Array, arena } if dst.is_bare() && arena_ok(arena) => {
                    let t = body.local(dst.local)?.ty;
                    plain_array(cx, t)?;
                    cx.tcx.array_len(t)?;
                }
                StmtKind::CopyInto { dst, src, arena } if dst.is_bare() && arena_ok(arena) => {
                    let t = body.local(dst.local)?.ty;
                    if place_ty(cx, body, src)? != t {
                        return None;
                    }
                    let copyable = cx.tcx.is_text(t)
                        || plain_array(cx, t).is_some()
                        || plain_vec(cx, t).is_some()
                        // The payload moves with the slot; handles pass through like `materialize_owned_value`.
                        || nullable_payload(cx, t).is_some_and(|p| !is_unit(cx, p));
                    if !copyable && trace() {
                        eprintln!("mir-backend: unsupported CopyInto of `{}`", cx.tcx.display(t));
                    }
                    copyable.then_some(())?;
                }
                StmtKind::Intrinsic { dst: None, op: IntrinsicOp::PrintStr | IntrinsicOp::PrintlnStr, args, arena: None } if args.len() == 1 => text(&args[0])?,
                StmtKind::Intrinsic { dst: Some(d), op: IntrinsicOp::StringEq, args, arena: None } if d.is_bare() && args.len() == 2 => {
                    text(&args[0])?;
                    text(&args[1])?;
                }
                StmtKind::Intrinsic { dst: Some(d), op: IntrinsicOp::Concat, args, arena: Some(arena) } if d.is_bare() && args.len() == 2 && arena_ok(arena) => {
                    text(&args[0])?;
                    text(&args[1])?;
                }
                StmtKind::Intrinsic { dst: Some(d), op: IntrinsicOp::StringFromUtf8, args, arena: Some(arena) } if d.is_bare() && args.len() == 3 && arena_ok(arena) => {
                    nullable_payload(cx, body.local(d.local)?.ty).filter(|p| cx.tcx.is_text(*p))?;
                    plain_array(cx, place_ty(cx, body, args[0].place()?)?)?;
                    operand(&args[1])?;
                    operand(&args[2])?;
                }
                StmtKind::Intrinsic { dst: None, op: IntrinsicOp::PrintInt | IntrinsicOp::PrintlnInt, args, arena: None } if args.len() == 1 => {
                    operand(&args[0])?;
                    let ty = args[0].place().and_then(|p| place_ty(cx, body, p));
                    if ty.is_some_and(|t| cx.int_type(t).is_none()) {
                        return None;
                    }
                }
                _ => return None,
            }
            Some(())
            })();
            if verdict.is_none() {
                if trace() {
                    eprintln!("mir-backend: unsupported statement {:?}", s.kind);
                }
                return None;
            }
        }
        let term = block.term.as_ref()?;
        let verdict = (|| -> Option<()> {
        match &term.kind {
            TermKind::Goto(_) | TermKind::Unreachable | TermKind::Abort(_) | TermKind::Return => {}
            TermKind::Switch { value, .. } => operand(value)?,
            TermKind::Call { callee, args, hidden, dest, .. } => {
                let (_, target) = callees.get(callee.as_str())?;
                let abi = CallAbi::of(cx, target);
                if hidden.len() != abi.hidden_len() || !hidden.iter().all(&arena_ok) {
                    return None;
                }
                value(target.return_ty)?;
                if args.len() != target.params.len() || !dest.is_bare() {
                    return None;
                }
                for (a, p) in args.iter().zip(&target.params) {
                    // `None` for a `Ref` parameter is the absent handle.
                    if !(matches!(a, Operand::Const(Const::None)) && cx.tcx.is_managed_ref(p.ty)) {
                        general(a)?;
                    }
                    value(p.ty)?;
                }
            }
            TermKind::Match { scrutinee, arms } => {
                let sty = place_ty(cx, body, scrutinee)?;
                plain_value(cx, sty).then_some(())?;
                for arm in arms {
                    pat_ok(cx, body, &arm.pat, scrutinee)?;
                    for (l, src) in &arm.bindings {
                        let lt = body.local(*l)?.ty;
                        if src.local != scrutinee.local || !plain_value(cx, lt) || is_unit(cx, lt) || place_ty(cx, body, src)? != cx.tcx.ref_inner(lt).unwrap_or(lt) {
                            return None;
                        }
                    }
                }
            }
        }
            Some(())
        })();
        if verdict.is_none() {
            if trace() {
                eprintln!("mir-backend: unsupported terminator {:?}", term.kind);
            }
            return None;
        }
    }
    Some(())
}

/// `place` projected to field `field` of variant `index`.
fn variant_field(place: &Place, index: u32, field: usize) -> Place {
    let mut p = place.clone();
    p.proj.push(Proj::Variant(index));
    p.proj.push(Proj::Field(field as u32));
    p
}

/// `pat` can be tested against the value at `place`: literals of scalar, bool or text type, enum variants and
/// `Some`/`None` of a nullable, nested to any depth.
fn pat_ok(cx: &Codegen<'_>, body: &Body, pat: &MirPat, place: &Place) -> Option<()> {
    let ty = place_ty(cx, body, place)?;
    match pat {
        MirPat::Wildcard | MirPat::Bind => Some(()),
        MirPat::Lit(PatLit::Int(_)) => cx.int_type(ty).map(|_| ()),
        MirPat::Lit(PatLit::Bool(_)) => (prim(cx, ty) == Some(Prim::Bool)).then_some(()),
        MirPat::Lit(PatLit::Str(_)) => cx.tcx.is_text(ty).then_some(()),
        MirPat::Variant { index, fields, .. } => {
            let name = plain_enum(cx, ty)?;
            cx.enum_layout(&name)?.variants.get(*index as usize)?;
            fields.iter().enumerate().try_for_each(|(i, f)| match f {
                MirPat::Wildcard | MirPat::Bind => Some(()),
                _ => pat_ok(cx, body, f, &variant_field(place, *index, i)),
            })
        }
        MirPat::Some(inner) => {
            cx.tcx.is_nullable(ty).then_some(())?;
            let mut p = place.clone();
            p.proj.push(Proj::Some);
            pat_ok(cx, body, inner, &p)
        }
        MirPat::None => cx.tcx.is_nullable(ty).then_some(()),
    }
}

struct Lower<'a, 'ctx> {
    cx: &'a Codegen<'ctx>,
    callees: &'a Callees<'a, 'ctx>,
    function: &'a HirFunction,
    body: &'a Body,
    llvm_fn: FunctionValue<'ctx>,
    slots: HashMap<Local, PointerValue<'ctx>>,
    region_slots: HashMap<RegionId, PointerValue<'ctx>>,
    blocks: Vec<BasicBlock<'ctx>>,
    caller_parent: Option<PointerValue<'ctx>>,
    result_sink: Option<PointerValue<'ctx>>,
    ref_homes: HashMap<Local, RegionId>,
    /// `-g`: the DISubprogram, lexical blocks per region and the i64 token slot per `DebugScope` local.
    subprogram: Option<inkwell::debug_info::DIScope<'ctx>>,
    di_scopes: HashMap<RegionId, inkwell::debug_info::DIScope<'ctx>>,
    scope_slots: HashMap<Local, PointerValue<'ctx>>,
    /// The most recently entered `DebugScope` local per region: a loop body's own scope follows the region's.
    region_scope: HashMap<RegionId, Local>,
}

pub(super) fn lower_function<'ctx>(
    cx: &Codegen<'ctx>,
    callees: &Callees<'_, 'ctx>,
    function: &HirFunction,
    body: &Body,
) -> Result<(), Diagnostic> {
    let (llvm_fn, _) = callees[function.name.as_str()];
    let entry = cx.context.append_basic_block(llvm_fn, "entry");
    cx.position_at_end(entry);
    cx.builder.unset_current_debug_location();
    let abi = CallAbi::of(cx, function);
    let mut lower = Lower {
        cx,
        callees,
        function,
        body,
        llvm_fn,
        slots: HashMap::new(),
        region_slots: HashMap::new(),
        blocks: Vec::new(),
        caller_parent: (abi.hidden_len() > 0).then(|| llvm_fn.get_nth_param(0).expect("callee has a parent arena parameter").into_pointer_value()),
        ref_homes: ref_homes(cx, body),
        subprogram: cx.debug.as_ref().map(|d| d.function(cx, function, llvm_fn)),
        di_scopes: HashMap::new(),
        scope_slots: HashMap::new(),
        region_scope: HashMap::new(),
        result_sink: abi.has_result_sink().then(|| llvm_fn.get_nth_param(1).expect("callee has a result sink parameter").into_pointer_value()),
    };
    lower.prologue(abi.hidden_len())?;
    for i in 0..body.blocks.len() {
        let block = cx.context.append_basic_block(llvm_fn, &format!("bb{i}"));
        lower.blocks.push(block);
    }
    cx.builder.build_unconditional_branch(lower.blocks[0])?;
    for (i, block) in body.blocks.iter().enumerate() {
        cx.position_at_end(lower.blocks[i]);
        for s in &block.stmts {
            lower.locate(s.region, s.span);
            lower.stmt(s)?;
            // Arguments are declared once the function's own region exists: a `Ref` argument registers its slot in it.
            if matches!(s.kind, StmtKind::RegionEnter(RegionId(0))) {
                lower.declare_arguments()?;
            }
        }
        let term = block.term.as_ref().expect("verified MIR has terminators");
        lower.locate(term.region, term.span);
        lower.term(term)?;
    }
    Ok(())
}

impl<'a, 'ctx> Lower<'a, 'ctx> {
    /// DI scope of `region`: the subprogram for the root, a lexical block (parent = enclosing region) otherwise.
    fn di_scope(&mut self, region: RegionId) -> Option<inkwell::debug_info::DIScope<'ctx>> {
        self.cx.debug.as_ref()?;
        if let Some(scope) = self.di_scopes.get(&region) {
            return Some(*scope);
        }
        let scope = match self.body.regions[region.0 as usize].parent {
            None => self.subprogram?,
            Some(parent) => {
                let parent = self.di_scope(parent)?;
                self.cx.debug_lexical_block(parent)?
            }
        };
        self.di_scopes.insert(region, scope);
        Some(scope)
    }

    fn locate(&mut self, region: RegionId, span: crate::span::Span) {
        if let Some(scope) = self.di_scope(region) {
            self.cx.debug_at(scope, span);
        }
    }

    /// Runtime scope token and home arena `debug_label_slot` needs for a managed `Ref` local declared in `region`
    /// (the HIR emitter's innermost debug scope and current arena); `None`s for every other type.
    fn slot_context(&self, ty: TyId, region: RegionId) -> Result<(Option<inkwell::values::IntValue<'ctx>>, Option<PointerValue<'ctx>>), Diagnostic> {
        if self.cx.debug.is_none() || !self.cx.tcx.is_managed_ref(ty) {
            return Ok((None, None));
        }
        let token = match self.region_scope.get(&region) {
            Some(l) => Some(self.cx.builder.build_load(self.cx.context.i64_type(), self.scope_slots[l], "")?.into_int_value()),
            None => None,
        };
        Ok((token, Some(self.arena_ref(&ArenaRef::Current, region)?)))
    }

    /// `declare_local` for a name bound by a `when` arm or a presence guard (no `DebugLabel`: the local carries the name).
    fn declare_binding(&mut self, local: Local, span: crate::span::Span) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let decl = self.body.local(local).expect("checked");
        let (Some(name), Some(slot)) = (decl.name.clone(), self.slots.get(&local).copied()) else {
            return Ok(());
        };
        let Some(scope) = self.di_scope(decl.region) else {
            return Ok(());
        };
        let (ty, region) = (decl.ty, decl.region);
        cx.debug_at(scope, span);
        let (token, home) = self.slot_context(ty, region)?;
        cx.debug_local_in(scope, token, &name, ty, slot, home, None)?;
        let value = cx.builder.build_load(cx.basic_type(ty).expect("checked value type"), slot, "")?;
        cx.debug_declaration(value, ty, &name, span, self.function.source)
    }

    fn declare_arguments(&mut self) -> Result<(), Diagnostic> {
        let Some(scope) = self.subprogram else {
            return Ok(());
        };
        self.cx.debug_at(scope, self.function.name_span);
        let mut index = 0;
        for (i, decl) in self.body.locals.iter().enumerate() {
            if decl.kind != LocalKind::Arg {
                continue;
            }
            index += 1;
            if let (Some(name), Some(slot)) = (&decl.name, self.slots.get(&Local(i as u32))) {
                let (token, home) = self.slot_context(decl.ty, RegionId(0))?;
                self.cx.debug_local_in(scope, token, name, decl.ty, *slot, home, Some(index))?;
            }
        }
        Ok(())
    }

    fn declare_label(&mut self, stmt: &Statement, label: &DebugLabel) -> Result<(), Diagnostic> {
        let cx = self.cx;
        match &stmt.kind {
            // A bare destination is a source variable; a projected one is a record or enum payload field.
            StmtKind::Assign(dst, _) if dst.is_bare() => {
                let (Some(scope), Some(slot)) = (self.di_scope(stmt.region), self.slots.get(&dst.local).copied()) else {
                    return Ok(());
                };
                let (token, home) = self.slot_context(label.ty, stmt.region)?;
                cx.debug_local_in(scope, token, &label.name, label.ty, slot, home, label.argument)?;
                let value = cx.builder.build_load(cx.basic_type(label.ty).expect("checked value type"), slot, "")?;
                cx.debug_declaration(value, label.ty, &label.name, label.span, label.source)
            }
            StmtKind::Assign(dst, _) => {
                if is_unit(cx, label.ty) {
                    return Ok(());
                }
                let (ptr, _) = self.place_ptr(dst)?;
                let value = cx.builder.build_load(cx.basic_type(label.ty).expect("checked value type"), ptr, "")?;
                cx.debug_declaration(value, label.ty, &label.name, label.span, label.source)
            }
            StmtKind::Ref(RefOp::Store { slot, owner, .. }) => {
                let (ptr, _) = self.place_ptr(slot)?;
                let owner = self.arena_ref(owner, stmt.region)?;
                let word = cx.context.i64_type();
                // Array elements carry their runtime index; fields and locals use the all-ones "no index".
                let index = match &label.index {
                    Some(i) => self.index_i64(i)?,
                    None => word.const_all_ones(),
                };
                cx.debug_label_slot(owner, ptr, &label.name, label.ty, word.const_zero(), index)
            }
            _ => Ok(()),
        }
    }

    fn prologue(&mut self, hidden: usize) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let mut params = self.llvm_fn.get_param_iter().skip(hidden);
        let observations: HashSet<Local> = self.body.blocks.iter().flat_map(|b| &b.stmts).filter_map(|s| if let StmtKind::Observe { obs, .. } = s.kind { Some(obs) } else { None }).collect();
        for (i, decl) in self.body.locals.iter().enumerate() {
            let local = Local(i as u32);
            if decl.kind == LocalKind::Hidden(HiddenKind::DebugScope) {
                if cx.debug.is_some() {
                    self.scope_slots.insert(local, cx.builder.build_alloca(cx.context.i64_type(), "debug.scope")?);
                }
                continue;
            }
            if decl.kind == LocalKind::Hidden(HiddenKind::DynamicArena) {
                self.slots.insert(local, cx.builder.build_alloca(cx.context.ptr_type(inkwell::AddressSpace::default()), "dynamic.arena")?);
                continue;
            }
            if observations.contains(&local) {
                self.slots.insert(local, cx.builder.build_alloca(cx.ref_observation_type(), "ref.guard")?);
                continue;
            }
            if decl.kind == LocalKind::Hidden(HiddenKind::RegionArena) || is_unit(cx, decl.ty) {
                continue;
            }
            let ty = cx.basic_type(decl.ty).ok_or_else(|| not_yet_supported("local type", None))?;
            let slot = cx.builder.build_alloca(ty, "local")?;
            if decl.kind == LocalKind::Arg {
                let value = params.next().expect("one parameter per argument local");
                cx.builder.build_store(slot, value)?;
            }
            self.slots.insert(local, slot);
        }
        let ptr = cx.context.ptr_type(inkwell::AddressSpace::default());
        for r in pushed_regions(self.body) {
            self.region_slots.insert(r, cx.builder.build_alloca(ptr, "arena")?);
        }
        Ok(())
    }

    /// The arena of the nearest pushed region at or above `start`, if any.
    fn arena_from(&self, start: Option<RegionId>) -> Result<Option<PointerValue<'ctx>>, Diagnostic> {
        let mut at = start;
        while let Some(r) = at {
            if let Some(slot) = self.region_slots.get(&r) {
                return self.load_ptr(*slot).map(Some);
            }
            at = self.body.regions[r.0 as usize].parent;
        }
        Ok(None)
    }

    /// The arena allocations and calls in `region` see: the nearest enclosing pushed region.
    fn arena_of(&self, region: RegionId) -> Result<PointerValue<'ctx>, Diagnostic> {
        let found = self.arena_from(Some(region))?;
        found.or(self.caller_parent).ok_or_else(|| not_yet_supported("call outside any open arena", None))
    }

    fn load_ptr(&self, slot: PointerValue<'ctx>) -> Result<PointerValue<'ctx>, Diagnostic> {
        let ptr = self.cx.context.ptr_type(inkwell::AddressSpace::default());
        Ok(self.cx.builder.build_load(ptr, slot, "arena")?.into_pointer_value())
    }

    fn op_ty(&self, op: &Operand) -> Option<TyId> {
        op.place().and_then(|p| place_ty(self.cx, self.body, p))
    }

    fn operand(&self, op: &Operand, expect: TyId) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        Ok(match op {
            Operand::Copy(p) | Operand::Move(p) | Operand::Reborrow(p) => {
                let (ptr, pty) = self.place_ptr(p)?;
                cx.builder.build_load(cx.basic_type(pty).expect("checked value type"), ptr, "")?
            }
            Operand::Const(Const::Int(v)) => match cx.int_type(expect) {
                Some(t) => t.const_int(*v as u64, true).into(),
                None => cx.basic_type(expect).expect("float constant type").into_float_type().const_float(*v as f64).into(),
            },
            Operand::Const(Const::Float(v)) => cx.basic_type(expect).expect("float type").into_float_type().const_float(*v).into(),
            Operand::Const(Const::None) if cx.tcx.is_managed_ref(expect) => cx.ref_handle_type().const_zero().into(),
            Operand::Const(Const::Str(text)) => cx.str_literal(text).into(),
            Operand::Const(Const::Bool(b)) => cx.context.bool_type().const_int(*b as u64, false).into(),
            _ => return Err(not_yet_supported("operand", None)),
        })
    }

    fn int_as(&self, value: BasicValueEnum<'ctx>, ty: TyId) -> Result<inkwell::values::IntValue<'ctx>, Diagnostic> {
        let target = self.cx.int_type(ty).ok_or_else(|| not_yet_supported("integer type", None))?;
        let int = value.into_int_value();
        if int.get_type() == target {
            return Ok(int);
        }
        Ok(self.cx.builder.build_int_cast_sign_flag(int, target, !is_unsigned(&self.cx.tcx, ty), "cast")?)
    }

    /// The arena an op names (`supported` only lets `Current`, `Root` and `Fresh` through).
    fn arena_ref(&self, a: &ArenaRef, region: RegionId) -> Result<PointerValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        match a {
            ArenaRef::Current => self.arena_of(region),
            ArenaRef::Region(r) => self.arena_of(*r),
            ArenaRef::ResultSink => Ok(self.result_sink.expect("checked result sink")),
            ArenaRef::CallerParent => Ok(self.caller_parent.expect("checked caller parent")),
            // The arena inside the handle (`{arena, object}`) or the `Vec` header the binding holds.
            ArenaRef::Home(l) if self.ref_homes.contains_key(l) => self.arena_of(self.ref_homes[l]),
            ArenaRef::Home(l) | ArenaRef::ObjectArena(l) => {
                let ty = self.body.local(*l).expect("checked local").ty;
                if plain_vec(cx, ty).is_some() {
                    let header = cx.builder.build_load(cx.context.ptr_type(inkwell::AddressSpace::default()), self.slots[l], "vec")?.into_pointer_value();
                    cx.vec_load_ptr(header, vec::ARENA)
                } else {
                    let handle = cx.builder.build_load(cx.object_handle_type(), self.slots[l], "home")?.into_struct_value();
                    Ok(cx.builder.build_extract_value(handle, 0, "home.arena")?.into_pointer_value())
                }
            }
            ArenaRef::Fresh(n) => self.load_ptr(self.slots[&Local(*n)]),
            ArenaRef::Root => Ok(cx
                .builder
                .build_call(cx.arena_root_fn(), &[], "arena.root")?
                .try_as_basic_value()
                .basic()
                .expect("bork_arena_root returns a pointer")
                .into_pointer_value()),
        }
    }

    /// Address and type of `p`: the local's slot, then one GEP per `Field` (the object handle in
    /// the slot is `{arena, object}`; fields live behind `object`, enum payloads behind a `Variant`).
    fn place_ptr(&self, p: &Place) -> Result<(PointerValue<'ctx>, TyId), Diagnostic> {
        let cx = self.cx;
        let mut ty = self.body.locals[p.local.0 as usize].ty;
        let mut ptr = self.slots[&p.local];
        let mut variant = None;
        for pr in &p.proj {
            match pr {
                Proj::Variant(v) => variant = Some(*v),
                Proj::Field(i) => {
                    let object = self.object_ptr(ptr)?;
                    if let Some(v) = variant.take() {
                        let name = plain_enum(cx, ty).expect("checked plain enum");
                        let record = cx.enum_layout(&name).expect("checked layout").variants[v as usize];
                        ptr = cx.builder.build_struct_gep(record, object, *i + 1, "payload")?;
                        ty = cx.enums.iter().find(|e| e.name == name).expect("checked enum").variants[v as usize].fields[*i as usize].ty;
                    } else {
                        let name = plain_class(cx, ty).expect("checked plain class");
                        let record = cx.record_type(&name).expect("checked record");
                        ptr = cx.builder.build_struct_gep(record, object, *i, "field")?;
                        ty = cx.record_field_type(&name, *i as usize).expect("checked field");
                    }
                }
                Proj::Some => {
                    let object = self.object_ptr(ptr)?;
                    let name = plain_enum(cx, ty).expect("checked nullable");
                    let (_, tag) = cx.enum_variant(&name, "Some").expect("prelude Some");
                    let record = cx.enum_layout(&name).expect("checked layout").variants[tag as usize];
                    ptr = cx.builder.build_struct_gep(record, object, 1, "some.payload")?;
                    ty = nullable_payload(cx, ty).expect("checked nullable");
                }
                Proj::Index(idx) => {
                    let elem = buffer_elem(cx, ty).expect("checked buffer");
                    let (_, stride, _) = cx.elem_storage(elem)?;
                    let desc = cx.builder.build_load(cx.buffer_descriptor_type(), ptr, "buf")?.into_struct_value();
                    let base = cx.builder.build_extract_value(desc, 0, "buf.base")?.into_pointer_value();
                    let i = self.index_i64(idx)?;
                    let offset = cx.builder.build_int_mul(i, cx.word(stride as u64), "elem.off")?;
                    // SAFETY: MIR guards every index with a `Len` comparison and `Abort(Bounds)`.
                    ptr = unsafe { cx.builder.build_gep(cx.context.i8_type(), base, &[offset], "elem")? };
                    ty = elem;
                }
                Proj::Deref => {}
            }
        }
        Ok((ptr, ty))
    }

    /// An index operand as the `i32` HIR indexes with, sign-extended to `i64`.
    fn index_i64(&self, op: &Operand) -> Result<inkwell::values::IntValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        let i32_ty = cx.i32_ty();
        let idx = self.int_as(self.operand(op, i32_ty)?, i32_ty)?;
        Ok(cx.builder.build_int_s_extend(idx, cx.context.i64_type(), "idx")?)
    }

    /// The object pointer inside the `{arena, object}` handle stored at `slot`.
    fn object_ptr(&self, slot: PointerValue<'ctx>) -> Result<PointerValue<'ctx>, Diagnostic> {
        self.object_ptr_with(slot, true)
    }

    /// `check` is the `-g` handle validation; the HIR emitter skips it where it only registers drop glue.
    fn object_ptr_with(&self, slot: PointerValue<'ctx>, check: bool) -> Result<PointerValue<'ctx>, Diagnostic> {
        let handle = self.cx.builder.build_load(self.cx.object_handle_type(), slot, "object")?.into_struct_value();
        let ptr = self.cx.builder.build_extract_value(handle, 1, "object.ptr")?.into_pointer_value();
        if check && self.cx.debug.is_some() {
            let owner = self.cx.builder.build_extract_value(handle, 0, "object.arena")?.into_pointer_value();
            self.cx.debug_check_handle(owner, ptr)?;
        }
        Ok(ptr)
    }

    /// `bork_arena_alloc` of `size`/`align` bytes, wrapped as the `{arena, object}` handle and stored in `dst`.
    fn alloc_handle(&self, dst: &Place, arena: PointerValue<'ctx>, size: inkwell::values::IntValue<'ctx>, align: inkwell::values::IntValue<'ctx>, tag: Option<(inkwell::types::StructType<'ctx>, u32)>, label: &str) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let object = cx
            .builder
            .build_call(cx.arena_alloc_fn(), &[arena.into(), size.into(), align.into()], "object")?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_alloc returns a pointer")
            .into_pointer_value();
        cx.debug_allocation(object, "", label, false)?;
        if let Some((variant_ty, tag)) = tag {
            let slot = cx.builder.build_struct_gep(variant_ty, object, 0, "enum.tag")?;
            cx.builder.build_store(slot, cx.context.i32_type().const_int(u64::from(tag), false))?;
        }
        let handle = cx.object_handle_type().const_named_struct(&[]);
        let handle = cx.builder.build_insert_value(handle, arena, 0, "object.arena")?;
        let handle = cx.builder.build_insert_value(handle, object, 1, "object.ptr")?;
        self.store(dst, handle.as_basic_value_enum())
    }

    fn store(&self, dst: &Place, value: BasicValueEnum<'ctx>) -> Result<(), Diagnostic> {
        if dst.is_bare() && !self.slots.contains_key(&dst.local) {
            return Ok(());
        }
        let (ptr, _) = self.place_ptr(dst)?;
        self.cx.builder.build_store(ptr, value)?;
        Ok(())
    }

    fn stmt(&mut self, stmt: &Statement) -> Result<(), Diagnostic> {
        self.stmt_inner(stmt)?;
        match &stmt.debug {
            Some(label) if self.cx.debug.is_some() => self.declare_label(stmt, label),
            _ => Ok(()),
        }
    }

    fn stmt_inner(&mut self, stmt: &Statement) -> Result<(), Diagnostic> {
        let cx = self.cx;
        match &stmt.kind {
            StmtKind::ArenaCreateDynamic { dst, parent, scoped } => {
                let parent = self.arena_ref(parent, stmt.region)?;
                let arena = cx
                    .builder
                    .build_call(cx.arena_create_dynamic_fn(), &[parent.into(), cx.context.i8_type().const_int(*scoped as u64, false).into()], "arena.dynamic")?
                    .try_as_basic_value()
                    .basic()
                    .expect("bork_arena_create_dynamic returns a pointer");
                cx.builder.build_store(self.slots[dst], arena)?;
            }
            StmtKind::RegisterStrongRoot { owner, arena } => {
                let owner = self.arena_ref(owner, stmt.region)?;
                let arena = self.load_ptr(self.slots[arena])?;
                cx.builder.build_call(cx.arena_register_strong_root_fn(), &[owner.into(), arena.into()], "")?;
            }
            StmtKind::Borrow { dst, src, .. } | StmtKind::Move { dst, src } => {
                let (ptr, ty) = self.place_ptr(src)?;
                let v = cx.builder.build_load(cx.basic_type(ty).expect("checked value type"), ptr, "")?;
                self.store(dst, v)?;
            }
            StmtKind::Ref(op) => {
                self.ref_op(op, stmt.region)?;
                if let (true, RefOp::Clone { dst, .. }) = (cx.debug.is_some(), op) {
                    if dst.is_bare() {
                        self.declare_binding(dst.local, stmt.span)?;
                    }
                }
            }
            StmtKind::Observe { obs, object, absent, value } => {
                let reference = self.ref_operand(value)?;
                let (observation, ptr, is_absent) = cx.ref_observe(self.llvm_fn, reference)?;
                cx.builder.build_store(self.slots[obs], observation)?;
                // As `bind_observation`: the view's arena slot holds the Ref's control.
                let control = cx.builder.build_extract_value(reference.into_struct_value(), 0, "ref.control")?;
                let handle = cx.object_handle_type().const_named_struct(&[]);
                let handle = cx.builder.build_insert_value(handle, control, 0, "borrow.arena")?;
                let handle = cx.builder.build_insert_value(handle, ptr, 1, "borrow.ptr")?;
                self.store(&Place::local(*object), handle.as_basic_value_enum())?;
                let flag = cx.builder.build_int_z_extend_or_bit_cast(is_absent, cx.int_type(self.body.local(*absent).expect("checked").ty).expect("checked"), "absent")?;
                self.store(&Place::local(*absent), flag.into())?;
                if cx.debug.is_some() {
                    self.declare_binding(*object, stmt.span)?;
                }
            }
            StmtKind::ObservationRegister { arena, obs } => {
                let owner = self.arena_ref(arena, stmt.region)?;
                cx.builder.build_call(cx.arena_register_observation_drop_fn(), &[owner.into(), self.slots[obs].into()], "")?;
            }
            StmtKind::ObservationRelease(obs) => {
                cx.builder.build_call(cx.ref_release_observation_fn(), &[self.slots[obs].into()], "")?;
            }
            StmtKind::RegisterDrop { arena, place, glue } => {
                let owner = self.arena_ref(arena, stmt.region)?;
                let (ptr, _) = self.place_ptr(place)?;
                match glue {
                    DropGlue::RefSlot => cx.ref_register_drop(owner, ptr)?,
                    DropGlue::Enum(name) => {
                        if let Some(f) = cx.enum_drop_glue(name) {
                            let object = self.object_ptr_with(ptr, false)?;
                            cx.builder.build_call(cx.arena_register_drop_fn(), &[owner.into(), object.into(), f.as_global_value().as_pointer_value().into()], "")?;
                        }
                    }
                    // `vec_alloc` registers the glue with the header it allocates; the op only carries it for the plan.
                    DropGlue::Vec(_) => {}
                }
            }
            StmtKind::Alloc { dst, kind: AllocKind::EnumSlot { enum_name, variant }, arena } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                if stmt.debug.is_some() {
                    cx.debug_label_arena(arena)?;
                }
                let layout = cx.enum_layout(enum_name).expect("checked layout");
                let i64 = cx.context.i64_type();
                let (size, align) = (i64.const_int(layout.size, false), i64.const_int(layout.align, false));
                self.alloc_handle(dst, arena, size, align, Some((layout.variants[*variant as usize], *variant)), enum_name)?;
            }
            StmtKind::Alloc { dst, kind: AllocKind::Vec { cap }, arena } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let elem = plain_vec(cx, self.body.local(dst.local).expect("checked").ty).expect("checked vec");
                let cap = match cap {
                    Some(c) => Some(self.int_as(self.operand(c, cx.i32_ty())?, cx.i32_ty())?.into()),
                    None => None,
                };
                let header = cx.vec_new(self.llvm_fn, arena, elem, cap)?;
                self.store(dst, header.into())?;
            }
            StmtKind::VecOp { dst, op, receiver, args, arena } => {
                let rty = self.op_ty(receiver).expect("checked");
                let elem = plain_vec(cx, rty).expect("checked vec");
                let header = self.operand(receiver, rty)?.into_pointer_value();
                let result = if *op == VecOp::Pop {
                    let arena = arena.as_ref().map(|a| self.arena_ref(a, stmt.region)).transpose()?;
                    let dty = place_ty(cx, self.body, dst.as_ref().expect("checked pop result")).expect("checked place");
                    Some(self.vec_pop(header, elem, dty, arena)?)
                } else {
                    self.vec_op(*op, header, elem, args)?
                };
                if let (Some(d), Some(v)) = (dst, result) {
                    self.store(d, v)?;
                }
            }
            StmtKind::Alloc { dst, kind: AllocKind::Object { class }, arena } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let record = cx.record_type(class).expect("checked record");
                let size = record.size_of().ok_or_else(|| not_yet_supported("unsized record", None))?;
                // The construction arena is labelled once per source-level construct (`begin_object_construct`).
                if stmt.debug.is_some() {
                    cx.debug_label_arena(arena)?;
                }
                self.alloc_handle(dst, arena, size, record.get_alignment(), None, class)?;
            }
            StmtKind::Assign(dst, rv) => {
                let dty = place_ty(cx, self.body, dst).expect("checked place");
                if is_unit(cx, dty) {
                    return Ok(());
                }
                let value = match rv {
                    Rvalue::Use(a) => self.operand(a, dty)?,
                    Rvalue::Unary(_, a) => cx.builder.build_not(self.operand(a, dty)?.into_int_value(), "not")?.into(),
                    Rvalue::Binary(op, a, b) => self.binary(op, a, b, dty)?,
                    Rvalue::Cast(a, to) => {
                        let from = self.op_ty(a).unwrap_or(*to);
                        let v = self.operand(a, from)?;
                        let int = self.int_as(v, from)?;
                        let target = cx.int_type(*to).expect("checked integer cast");
                        cx.builder.build_int_cast_sign_flag(int, target, !is_unsigned(&cx.tcx, from), "cast")?.into()
                    }
                    Rvalue::Len(p) => {
                        let (ptr, _) = self.place_ptr(p)?;
                        let desc = cx.builder.build_load(cx.buffer_descriptor_type(), ptr, "buf")?.into_struct_value();
                        let len = cx.builder.build_extract_value(desc, 1, "buf.len")?.into_int_value();
                        cx.builder.build_int_cast_sign_flag(len, cx.int_type(dty).expect("checked length type"), true, "len")?.into()
                    }
                    Rvalue::Slice { base, lo, .. } => {
                        // As `emit_slice_values`: no runtime check of `lo`/`hi`; the length is part of the type.
                        let bty = self.op_ty(base).expect("checked slice base");
                        let desc = self.operand(base, bty)?.into_struct_value();
                        let ptr = cx.builder.build_extract_value(desc, 0, "slice.src")?.into_pointer_value();
                        let (_, stride, _) = cx.elem_storage(plain_array(cx, dty).expect("checked slice type"))?;
                        let offset = cx.builder.build_int_mul(self.index_i64(lo)?, cx.word(stride as u64), "off")?;
                        let new_base = unsafe { cx.builder.build_gep(cx.context.i8_type(), ptr, &[offset], "slice.base")? };
                        let new_len = cx.tcx.array_len(dty).expect("checked slice length");
                        cx.descriptor(new_base, cx.word(u64::from(new_len))).into()
                    }
                    _ => unreachable!("filtered by supported"),
                };
                self.store(dst, value)?;
            }
            StmtKind::RegionEnter(r) => {
                let parent = self.arena_from(self.body.regions[r.0 as usize].parent)?.or(self.caller_parent);
                let call = match parent {
                    Some(parent) => cx.builder.build_call(cx.arena_push_child_fn(), &[parent.into()], "arena")?,
                    None => cx.builder.build_call(cx.arena_push_fn(), &[], "arena")?,
                };
                let handle = call.try_as_basic_value().basic().expect("bork_arena_push returns a pointer");
                cx.builder.build_store(self.region_slots[r], handle)?;
                cx.debug_label_arena(handle.into_pointer_value())?;
            }
            StmtKind::RegionExit(r) => {
                let handle = self.load_ptr(self.region_slots[r])?;
                cx.builder.build_call(cx.arena_pop_fn(), &[handle.into()], "")?;
            }
            StmtKind::RegionReset { region, .. } => {
                let handle = self.load_ptr(self.region_slots[region])?;
                let next = cx.builder.build_call(cx.arena_reset_fn(), &[handle.into()], "arena.reset")?;
                let next = next.try_as_basic_value().basic().expect("bork_arena_reset returns a pointer");
                cx.builder.build_store(self.region_slots[region], next)?;
                cx.debug_label_arena(next.into_pointer_value())?;
            }
            StmtKind::StorageLive(l) => {
                if let Some(slot) = self.scope_slots.get(l).copied() {
                    // Creates the region's lexical block here, at the scope's own position.
                    let region = self.body.local(*l).expect("checked").region;
                    self.di_scope(region);
                    self.region_scope.insert(region, *l);
                    let f = cx.runtime_fn("bork_debug_scope_enter", || cx.context.i64_type().fn_type(&[], false));
                    let token = cx.builder.build_call(f, &[], "debug.scope")?.try_as_basic_value().basic().expect("scope token");
                    cx.builder.build_store(slot, token)?;
                }
            }
            StmtKind::StorageDead(l) => {
                if let Some(slot) = self.scope_slots.get(l).copied() {
                    let f = cx.runtime_fn("bork_debug_scope_exit", || cx.context.void_type().fn_type(&[cx.context.i64_type().into()], false));
                    let token = cx.builder.build_load(cx.context.i64_type(), slot, "")?;
                    cx.builder.build_call(f, &[token.into()], "")?;
                }
            }
            StmtKind::Alloc { dst, kind: AllocKind::Array, arena } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let ty = self.body.local(dst.local).expect("checked").ty;
                let elem = plain_array(cx, ty).expect("checked array");
                let (_, stride, align) = cx.elem_storage(elem)?;
                let len = u64::from(cx.tcx.array_len(ty).expect("checked array length"));
                let base = cx.arena_alloc(arena, cx.word(len * stride as u64), align)?;
                cx.debug_allocation(base, "", &cx.tcx.display(ty).to_string(), false)?;
                self.store(dst, cx.descriptor(base, cx.word(len)).into())?;
            }
            StmtKind::CopyInto { dst, src, arena } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let (ptr, ty) = self.place_ptr(src)?;
                let value = cx.builder.build_load(cx.basic_type(ty).expect("checked value type"), ptr, "copy.src")?;
                let copied = cx.copy_value_into(self.llvm_fn, arena, ty, value)?;
                self.store(dst, copied)?;
            }
            StmtKind::Intrinsic { op: op @ (IntrinsicOp::PrintStr | IntrinsicOp::PrintlnStr), args, .. } => {
                let desc = self.operand(&args[0], cx.i32_ty())?.into_struct_value();
                let bytes = cx.builder.build_extract_value(desc, 0, "print.ptr")?;
                let len = cx.builder.build_extract_value(desc, 1, "print.len")?;
                cx.builder.build_call(cx.print_str_fn(*op == IntrinsicOp::PrintlnStr), &[bytes.into(), len.into()], "")?;
            }
            StmtKind::Intrinsic { dst: Some(d), op: IntrinsicOp::StringEq, args, .. } => {
                let (l, r) = (self.operand(&args[0], cx.i32_ty())?.into_struct_value(), self.operand(&args[1], cx.i32_ty())?.into_struct_value());
                let equal = cx.string_equal(l, r)?;
                self.store(d, equal.into())?;
            }
            StmtKind::Intrinsic { dst: Some(d), op: IntrinsicOp::Concat, args, arena: Some(arena) } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let (l, r) = (self.operand(&args[0], cx.i32_ty())?.into_struct_value(), self.operand(&args[1], cx.i32_ty())?.into_struct_value());
                let out = cx.concat_into(arena, l, r)?;
                self.store(d, out.into())?;
            }
            StmtKind::Intrinsic { dst: Some(d), op: IntrinsicOp::StringFromUtf8, args, arena: Some(arena) } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let bty = self.op_ty(&args[0]).expect("checked buffer");
                let buffer = self.operand(&args[0], bty)?.into_struct_value();
                let i32_ty = cx.i32_ty();
                let (start, count) = (self.int_as(self.operand(&args[1], i32_ty)?, i32_ty)?, self.int_as(self.operand(&args[2], i32_ty)?, i32_ty)?);
                let dty = self.body.local(d.local).expect("checked").ty;
                let out = cx.string_from_utf8(self.llvm_fn, arena, buffer, start, count, dty)?;
                self.store(d, out.into())?;
            }
            StmtKind::Intrinsic { op, args, .. } => {
                let ty = self.op_ty(&args[0]).unwrap_or_else(|| cx.i32_ty());
                let int = self.operand(&args[0], ty)?.into_int_value();
                let i64_ty = cx.context.i64_type();
                // Same widening as `emit_print_value`: sign-extend narrower integers, even unsigned ones.
                let wide = if int.get_type().get_bit_width() < 64 {
                    cx.builder.build_int_s_extend(int, i64_ty, "print.wide")?
                } else {
                    cx.builder.build_int_truncate(int, i64_ty, "print.wide")?
                };
                cx.builder.build_call(cx.print_i64_fn(*op == IntrinsicOp::PrintlnInt), &[wide.into()], "")?;
            }
            _ => unreachable!("filtered by supported"),
        }
        Ok(())
    }

    /// A `Ref` operand: a loaded handle, or the `None` handle (all zero).
    fn ref_operand(&self, op: &Operand) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        match op {
            Operand::Const(Const::None) => Ok(self.cx.ref_handle_type().const_zero().into()),
            _ => self.operand(op, self.cx.unit_ty()),
        }
    }

    /// The `Ref` protocol (RFC 0006 3.2); every op is one runtime call sequence.
    fn ref_op(&self, op: &RefOp, region: RegionId) -> Result<(), Diagnostic> {
        let (cx, f) = (self.cx, self.llvm_fn);
        match op {
            RefOp::Create { dst, source, target, object } => {
                let (source, target) = (self.arena_ref(source, region)?, self.arena_ref(target, region)?);
                let handle = self.ref_operand(object)?;
                let ptr = cx.builder.build_extract_value(handle.into_struct_value(), 1, "object.ptr")?.into_pointer_value();
                if cx.debug.is_some() {
                    let owner = cx.builder.build_extract_value(handle.into_struct_value(), 0, "object.arena")?.into_pointer_value();
                    cx.debug_check_handle(owner, ptr)?;
                }
                let r = cx.ref_create(f, source, target, ptr)?;
                self.store(dst, r)?;
            }
            RefOp::Clone { dst, src } => {
                let r = cx.ref_clone(f, self.ref_operand(src)?)?;
                self.store(dst, r)?;
            }
            RefOp::MoveOut { dst, src } => {
                let (slot, _) = self.place_ptr(src)?;
                let r = cx.ref_move_out(f, slot)?;
                self.store(dst, r)?;
            }
            RefOp::Store { slot, owner, value, fresh } => {
                let owner = self.arena_ref(owner, region)?;
                let value = self.ref_operand(value)?;
                let (ptr, _) = self.place_ptr(slot)?;
                if *fresh {
                    cx.builder.build_store(ptr, cx.ref_handle_type().const_zero())?;
                }
                cx.ref_store(f, ptr, owner, value)?;
            }
            RefOp::Rehome { dst, owner, value } => {
                let owner = self.arena_ref(owner, region)?;
                let r = cx.ref_rehome(f, owner, self.ref_operand(value)?)?;
                self.store(dst, r)?;
            }
            RefOp::Drop(v) => cx.ref_drop(f, self.ref_operand(v)?)?,
            RefOp::Eq { dst, lhs, rhs } => {
                let eq = cx.ref_eq(f, self.ref_operand(lhs)?, self.ref_operand(rhs)?, false)?;
                self.store(dst, eq.into())?;
            }
            RefOp::Count { dst, value, weak } => {
                let n = cx.ref_count(f, self.ref_operand(value)?, *weak)?;
                self.store(dst, n)?;
            }
            RefOp::IsNone { dst, value } => {
                let none = cx.ref_is_none(f, self.ref_operand(value)?)?;
                self.store(dst, none.into())?;
            }
        }
        Ok(())
    }

    /// Built-in `Vec` method; `Ref` elements are cloned out and stored with
    /// ownership, text is copied into the vector's arena.
    fn vec_op(&self, op: VecOp, header: PointerValue<'ctx>, elem: TyId, args: &[Operand]) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let cx = self.cx;
        let i32_ty = cx.i32_ty();
        let (llvm_elem, stride, _) = cx.elem_storage(elem)?;
        let is_ref = cx.tcx.is_managed_ref(elem);
        // Bounds-checked address of element `args[0]`.
        let slot = || -> Result<PointerValue<'ctx>, Diagnostic> {
            let idx = self.int_as(self.operand(&args[0], i32_ty)?, i32_ty)?;
            let idx = cx.builder.build_int_s_extend(idx, cx.context.i64_type(), "vec.idx")?;
            cx.guard_index(self.llvm_fn, idx, cx.vec_load_word(header, vec::LEN)?)?;
            cx.vec_slot(cx.vec_load_ptr(header, vec::BUF)?, idx, stride)
        };
        let store = |slot: PointerValue<'ctx>, value: &Operand, fresh: bool| -> Result<(), Diagnostic> {
            let arena = cx.vec_load_ptr(header, vec::ARENA)?;
            if is_ref {
                if fresh {
                    cx.builder.build_store(slot, cx.ref_handle_type().const_zero())?;
                }
                return cx.ref_store(self.llvm_fn, slot, arena, self.ref_operand(value)?);
            }
            let value = self.operand(value, elem)?;
            let value = if cx.tcx.is_text(elem) { cx.copy_string_into(arena, value.into_struct_value())?.into() } else { value };
            cx.builder.build_store(slot, value)?;
            Ok(())
        };
        Ok(match op {
            VecOp::Len | VecOp::Capacity => {
                let count = cx.vec_load_word(header, if op == VecOp::Len { vec::LEN } else { vec::CAP })?;
                Some(cx.builder.build_int_truncate(count, cx.context.i32_type(), "vec.count")?.into())
            }
            VecOp::Get => {
                let loaded = cx.builder.build_load(llvm_elem, slot()?, "vec.get")?;
                Some(if is_ref { cx.ref_clone(self.llvm_fn, loaded)? } else { loaded })
            }
            VecOp::Set => {
                store(slot()?, &args[1], false)?;
                None
            }
            VecOp::Push => {
                cx.vec_reserve_one(self.llvm_fn, header, elem)?;
                let len = cx.vec_load_word(header, vec::LEN)?;
                let slot = cx.vec_slot(cx.vec_load_ptr(header, vec::BUF)?, len, stride)?;
                store(slot, &args[0], true)?;
                cx.vec_store(header, vec::LEN, cx.builder.build_int_add(len, cx.word(1), "vec.len.new")?)?;
                None
            }
            VecOp::Clear => {
                if is_ref {
                    self.release_elements(header, llvm_elem, stride)?;
                }
                cx.vec_store(header, vec::LEN, cx.word(0))?;
                None
            }
            VecOp::Pop => unreachable!("`vec_pop`"),
        })
    }

    /// `pop`: the last element, or none. A `Ref` moves out of its slot; anything else becomes `Some(elem)` in `arena`.
    fn vec_pop(&self, header: PointerValue<'ctx>, elem: TyId, result_ty: TyId, arena: Option<PointerValue<'ctx>>) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let ctx = cx.context;
        let len = cx.vec_load_word(header, vec::LEN)?;
        let empty = cx.builder.build_int_compare(IntPredicate::EQ, len, cx.word(0), "vec.pop.empty")?;
        let (present, absent, merge) = (ctx.append_basic_block(self.llvm_fn, "vec.pop.some"), ctx.append_basic_block(self.llvm_fn, "vec.pop.none"), ctx.append_basic_block(self.llvm_fn, "vec.pop.done"));
        cx.builder.build_conditional_branch(empty, absent, present)?;

        cx.position_at_end(present);
        let last = cx.builder.build_int_sub(len, cx.word(1), "vec.last")?;
        cx.vec_store(header, vec::LEN, last)?;
        let (llvm_elem, stride, _) = cx.elem_storage(elem)?;
        let slot = cx.vec_slot(cx.vec_load_ptr(header, vec::BUF)?, last, stride)?;
        let (name, some_tag, none_tag) = if cx.tcx.is_managed_ref(elem) {
            (String::new(), 0, 0)
        } else {
            let (name, _) = cx.option_parts(result_ty)?;
            let tag = |v: &str| cx.enum_variant(&name, v).map(|(_, t)| t).ok_or_else(|| not_yet_supported("the prelude `Option`", None));
            let tags = (tag("Some")?, tag("None")?);
            (name, tags.0, tags.1)
        };
        let popped = if cx.tcx.is_managed_ref(elem) {
            // The slot is past `len` now, so the handle moves out and the slot becomes `None`.
            cx.ref_move_out(self.llvm_fn, slot)?
        } else {
            let loaded = cx.builder.build_load(llvm_elem, slot, "vec.pop")?;
            cx.option_slot(arena.expect("checked pop arena"), &name, some_tag, Some(loaded))?.into()
        };
        let present_end = cx.builder.get_insert_block().expect("positioned");
        cx.builder.build_unconditional_branch(merge)?;

        cx.position_at_end(absent);
        let none: BasicValueEnum<'ctx> = if cx.tcx.is_managed_ref(elem) {
            cx.ref_handle_type().const_zero().into()
        } else {
            cx.option_slot(arena.expect("checked pop arena"), &name, none_tag, None)?.into()
        };
        let absent_end = cx.builder.get_insert_block().expect("positioned");
        cx.builder.build_unconditional_branch(merge)?;

        cx.position_at_end(merge);
        let phi = cx.builder.build_phi(popped.get_type(), "vec.pop.value")?;
        phi.add_incoming(&[(&popped, present_end), (&none, absent_end)]);
        Ok(phi.as_basic_value())
    }

    fn release_elements(&self, header: PointerValue<'ctx>, llvm_elem: BasicTypeEnum<'ctx>, stride: usize) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let (len, buf) = (cx.vec_load_word(header, vec::LEN)?, cx.vec_load_ptr(header, vec::BUF)?);
        cx.counted_loop(self.llvm_fn, len, |i| {
            let handle = cx.builder.build_load(llvm_elem, cx.vec_slot(buf, i, stride)?, "vec.drop")?;
            cx.ref_drop(self.llvm_fn, handle)
        })
    }

    fn binary(&self, op: &BinOp, a: &Operand, b: &Operand, dty: TyId) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let (ta, tb) = (self.op_ty(a), self.op_ty(b));
        let compare = matches!(op, BinOp::Gt | BinOp::Lt | BinOp::Ge | BinOp::Le | BinOp::Eq | BinOp::Ne);
        // A comparison's result is `bool`, so its operands decide the type; two constants compare as `i64`.
        let operand_ty = if compare {
            let width = |t: TyId| cx.int_type(t).map_or(0, |t| t.get_bit_width());
            match (ta, tb) {
                (Some(x), Some(y)) => if width(y) > width(x) { y } else { x },
                (Some(x), None) | (None, Some(x)) => x,
                (None, None) => cx.tcx.prim_id(Prim::I64),
            }
        } else {
            dty
        };
        let builder = &cx.builder;
        if is_float(cx, operand_ty) {
            let (l, r) = (self.operand(a, operand_ty)?.into_float_value(), self.operand(b, operand_ty)?.into_float_value());
            let cmp = |p| builder.build_float_compare(p, l, r, "fcmp").map(Into::into);
            return Ok(match op {
                BinOp::Add => builder.build_float_add(l, r, "fadd")?.into(),
                BinOp::Sub => builder.build_float_sub(l, r, "fsub")?.into(),
                BinOp::Mul => builder.build_float_mul(l, r, "fmul")?.into(),
                BinOp::Div => builder.build_float_div(l, r, "fdiv")?.into(),
                BinOp::Gt => cmp(FloatPredicate::OGT)?,
                BinOp::Lt => cmp(FloatPredicate::OLT)?,
                BinOp::Ge => cmp(FloatPredicate::OGE)?,
                BinOp::Le => cmp(FloatPredicate::OLE)?,
                BinOp::Eq => cmp(FloatPredicate::OEQ)?,
                _ => cmp(FloatPredicate::ONE)?,
            });
        }
        let l = self.operand(a, operand_ty)?;
        let r = self.operand(b, operand_ty)?;
        let (l, r) = (self.int_as(l, operand_ty)?, self.int_as(r, operand_ty)?);
        let unsigned = is_unsigned(&cx.tcx, operand_ty);
        let pick = |s, u| if unsigned { u } else { s };
        let cmp = |p| builder.build_int_compare(p, l, r, "cmp").map(Into::into);
        Ok(match op {
            BinOp::Add => builder.build_int_add(l, r, "add")?.into(),
            BinOp::Sub => builder.build_int_sub(l, r, "sub")?.into(),
            BinOp::Mul => builder.build_int_mul(l, r, "mul")?.into(),
            // Zero and `MIN / -1` are guarded by the MIR that precedes this op.
            BinOp::Div if unsigned => builder.build_int_unsigned_div(l, r, "div")?.into(),
            BinOp::Div => builder.build_int_signed_div(l, r, "div")?.into(),
            BinOp::Gt => cmp(pick(IntPredicate::SGT, IntPredicate::UGT))?,
            BinOp::Lt => cmp(pick(IntPredicate::SLT, IntPredicate::ULT))?,
            BinOp::Ge => cmp(pick(IntPredicate::SGE, IntPredicate::UGE))?,
            BinOp::Le => cmp(pick(IntPredicate::SLE, IntPredicate::ULE))?,
            BinOp::Eq => cmp(IntPredicate::EQ)?,
            _ => cmp(IntPredicate::NE)?,
        })
    }

    fn pat_test(&self, pat: &MirPat, place: &Place, fail: BasicBlock<'ctx>) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let unless = |hit: inkwell::values::IntValue<'ctx>| -> Result<(), Diagnostic> {
            let ok = cx.context.append_basic_block(self.llvm_fn, "match.ok");
            cx.builder.build_conditional_branch(hit, ok, fail)?;
            cx.position_at_end(ok);
            Ok(())
        };
        let tag_is = |index: u32| -> Result<inkwell::values::IntValue<'ctx>, Diagnostic> {
            let (slot, ty) = self.place_ptr(place)?;
            let name = plain_enum(cx, ty).expect("checked plain enum");
            let object = self.object_ptr(slot)?;
            let first = cx.enum_layout(&name).expect("checked layout").variants[0];
            let tag_ptr = cx.builder.build_struct_gep(first, object, 0, "enum.tag")?;
            let tag = cx.builder.build_load(cx.context.i32_type(), tag_ptr, "tag")?.into_int_value();
            Ok(cx.builder.build_int_compare(IntPredicate::EQ, tag, cx.context.i32_type().const_int(u64::from(index), false), "arm.hit")?)
        };
        let variant_of = |name: &str| cx.enum_variant(&self.enum_name_at(place), name).expect("prelude Option").1;
        match pat {
            MirPat::Wildcard | MirPat::Bind => {}
            MirPat::Lit(lit) => {
                let (ptr, ty) = self.place_ptr(place)?;
                let value = cx.builder.build_load(cx.basic_type(ty).expect("checked value type"), ptr, "lit.value")?;
                let hit = match lit {
                    PatLit::Int(n) => {
                        let int = value.into_int_value();
                        cx.builder.build_int_compare(IntPredicate::EQ, int, int.get_type().const_int(*n as u64, true), "lit.eq")?
                    }
                    PatLit::Bool(b) => {
                        let flag = value.into_int_value();
                        cx.builder.build_int_compare(IntPredicate::EQ, flag, flag.get_type().const_int(u64::from(*b), false), "lit.eq")?
                    }
                    PatLit::Str(text) => cx.string_equal(value.into_struct_value(), cx.str_literal(text))?,
                };
                unless(hit)?;
            }
            MirPat::Variant { index, fields, .. } => {
                unless(tag_is(*index)?)?;
                for (i, f) in fields.iter().enumerate() {
                    self.pat_test(f, &variant_field(place, *index, i), fail)?;
                }
            }
            MirPat::Some(inner) => {
                unless(tag_is(variant_of("Some"))?)?;
                let mut p = place.clone();
                p.proj.push(Proj::Some);
                self.pat_test(inner, &p, fail)?;
            }
            MirPat::None => unless(tag_is(variant_of("None"))?)?,
        }
        Ok(())
    }

    fn enum_name_at(&self, place: &Place) -> String {
        let (_, ty) = self.place_ptr(place).expect("checked place");
        plain_enum(self.cx, ty).expect("checked plain enum")
    }

    fn term(&mut self, term: &Terminator) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let builder = &cx.builder;
        match &term.kind {
            TermKind::Goto(b) => {
                builder.build_unconditional_branch(self.blocks[b.0 as usize])?;
            }
            TermKind::Switch { value, cases, otherwise } => {
                let ty = self.op_ty(value).unwrap_or_else(|| cx.i32_ty());
                let v = self.operand(value, ty)?.into_int_value();
                let arms: Vec<_> = cases.iter().map(|(c, b)| (v.get_type().const_int(*c as u64, true), self.blocks[b.0 as usize])).collect();
                builder.build_switch(v, self.blocks[otherwise.0 as usize], &arms)?;
            }
            TermKind::Call { callee, args, dest, target, hidden, .. } => {
                let (callee_fn, function) = self.callees[callee.as_str()];
                let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
                for h in hidden {
                    call_args.push(self.arena_ref(h, term.region)?.into());
                }
                for (a, p) in args.iter().zip(&function.params) {
                    let v = self.operand(a, p.ty)?;
                    call_args.push(if cx.int_type(p.ty).is_some() { self.int_as(v, p.ty)?.into() } else { v.into() });
                }
                let call = builder.build_call(callee_fn, &call_args, "call")?;
                if let Some(value) = call.try_as_basic_value().basic() {
                    self.store(dest, value)?;
                }
                match target {
                    Some(b) => builder.build_unconditional_branch(self.blocks[b.0 as usize])?,
                    None => builder.build_unreachable()?,
                };
            }
            TermKind::Return => {
                let ret = match is_unit(cx, self.function.return_ty) {
                    true => None,
                    false => Some(self.operand(&Operand::Copy(Place::local(self.body.ret)), self.function.return_ty)?),
                };
                if self.function.name == "main" {
                    builder.build_return(Some(&ret.unwrap_or_else(|| cx.context.i32_type().const_zero().into())))?;
                } else {
                    builder.build_return(ret.as_ref().map(|v| v as &dyn inkwell::values::BasicValue))?;
                }
            }
            TermKind::Abort(_) => {
                builder.build_call(cx.abort_function(), &[], "abort")?;
                builder.build_unreachable()?;
            }
            TermKind::Unreachable => {
                builder.build_unreachable()?;
            }
            TermKind::Match { scrutinee, arms } => {
                for arm in arms {
                    let bind = cx.context.append_basic_block(self.llvm_fn, "arm");
                    let next = cx.context.append_basic_block(self.llvm_fn, "arm.next");
                    self.pat_test(&arm.pat, scrutinee, next)?;
                    builder.build_unconditional_branch(bind)?;
                    cx.position_at_end(bind);
                    for (l, src) in &arm.bindings {
                        let (ptr, ty) = self.place_ptr(src)?;
                        let v = builder.build_load(cx.basic_type(ty).expect("checked value type"), ptr, "bound")?;
                        self.store(&Place::local(*l), v)?;
                        if cx.debug.is_some() {
                            self.declare_binding(*l, term.span)?;
                        }
                    }
                    builder.build_unconditional_branch(self.blocks[arm.target.0 as usize])?;
                    cx.position_at_end(next);
                }
                builder.build_unreachable()?;
            }
        }
        Ok(())
    }
}
