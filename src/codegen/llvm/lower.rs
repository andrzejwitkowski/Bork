//! LLVM from MIR (RFC 0006 M5a, #222). Covers scalar locals, arithmetic and comparisons, calls with
//! the hidden arena argument, `Switch`/`Goto` control flow and region enter/exit/reset. [`supported`]
//! decides per function; anything else stays on the HIR emitter, so a function is never half lowered.

use std::collections::{HashMap, HashSet};

use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicType;
use inkwell::values::{BasicMetadataValueEnum, BasicValue, BasicValueEnum, FunctionValue, PointerValue};
use inkwell::{FloatPredicate, IntPredicate};

use crate::ast::BinOp;
use crate::diag::Diagnostic;
use crate::hir::{HirFunction, Prim};
use crate::mir::*;
use crate::ty::{TyData, TyId};

use super::call_abi::CallAbi;
use super::context::{is_unsigned, Codegen};
use super::emit_fn::Callees;
use super::not_yet_supported;

fn prim(cx: &Codegen<'_>, ty: TyId) -> Option<Prim> {
    match cx.tcx.data(ty) {
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

/// A class whose fields are all scalars or such classes: no `Ref`, `Vec`, text or nullable
/// fields, so it needs no drop registration (those stay on the HIR emitter until #224/#225).
fn plain_class(cx: &Codegen<'_>, ty: TyId) -> Option<String> {
    if let Some(hit) = cx.plain_classes.borrow().get(&ty) {
        return hit.clone();
    }
    let found = plain_class_at(cx, ty, 0);
    cx.plain_classes.borrow_mut().insert(ty, found.clone());
    found
}

fn plain_class_at(cx: &Codegen<'_>, ty: TyId, depth: u32) -> Option<String> {
    let ty = cx.tcx.ref_inner(ty).unwrap_or(ty);
    let name = crate::mono::class_instance_name(&cx.tcx, ty).filter(|_| depth < 8)?;
    cx.record_type(&name)?;
    let class = cx.class_by_name(&name)?;
    for i in 0..class.fields.len() {
        let f = cx.record_field_type(&name, i)?;
        prim(cx, f).map(|_| ()).or_else(|| plain_class_at(cx, f, depth + 1).map(|_| ()))?;
    }
    Some(name)
}

/// An enum whose variants carry only non-unit scalar payloads: no drop glue, no nested slots.
fn plain_enum(cx: &Codegen<'_>, ty: TyId) -> Option<String> {
    let ty = cx.tcx.ref_inner(ty).unwrap_or(ty);
    let name = crate::mono::enum_instance_name(&cx.tcx, ty)?;
    cx.enum_layout(&name)?;
    let e = cx.enums.iter().find(|e| e.name == name)?;
    for v in &e.variants {
        for f in &v.fields {
            if f.ty == cx.unit_ty() || prim(cx, f.ty).is_none() {
                return None;
            }
        }
    }
    Some(name)
}

/// Type of `place` when it is a bare local or a chain of fields over plain classes.
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
            _ => return None,
        }
    }
    if variant.is_some() {
        return None;
    }
    Some(ty)
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
    let value = |ty: TyId| scalar(ty).or_else(|| plain_class(cx, ty).or_else(|| plain_enum(cx, ty)).map(|_| ()));
    for l in &body.locals {
        match l.kind {
            LocalKind::Hidden(HiddenKind::RegionArena | HiddenKind::DynamicArena) => {}
            LocalKind::Hidden(_) => return None,
            _ => value(l.ty)?,
        }
    }
    for p in &function.params {
        value(p.ty)?;
    }
    value(function.return_ty)?;
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
            Operand::Const(Const::Int(_) | Const::Float(_) | Const::Bool(_) | Const::Unit) => Some(()),
            _ => None,
        }
    };
    // Arithmetic, casts and printing read the local's own type, so they need bare scalars.
    let operand = |op: &Operand| -> Option<()> {
        if op.place().is_some_and(|p| !p.is_bare()) {
            return None;
        }
        general(op)?;
        match op.place() {
            Some(p) => scalar(body.local(p.local)?.ty),
            None => Some(()),
        }
    };
    // Arenas lowering can name: the enclosing pushed region, the root, or a dynamic arena local.
    let arena_ok = |a: &ArenaRef| match a {
        ArenaRef::Current | ArenaRef::Root => true,
        ArenaRef::Fresh(n) => body.local(Local(*n)).is_some_and(|l| l.kind == LocalKind::Hidden(HiddenKind::DynamicArena)),
        _ => false,
    };
    for block in &body.blocks {
        for s in &block.stmts {
            match &s.kind {
                StmtKind::Assign(dst, rv) => {
                    let dty = place_ty(cx, body, dst)?;
                    match rv {
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
                                cx.int_type(body.local(p.local)?.ty)?;
                            }
                        }
                        _ => return None,
                    }
                }
                StmtKind::RegionEnter(r) | StmtKind::RegionExit(r) | StmtKind::RegionReset { region: r, .. } if !pushed.contains(r) => return None,
                StmtKind::RegionEnter(_) | StmtKind::RegionExit(_) | StmtKind::RegionReset { .. } => {}
                StmtKind::StorageLive(_) | StmtKind::StorageDead(_) => {}
                // Debug builds label every allocation; that stays on the HIR emitter until #226.
                StmtKind::ArenaCreateDynamic { parent, .. } if cx.debug.is_none() && arena_ok(parent) => {}
                StmtKind::RegisterStrongRoot { owner, .. } if cx.debug.is_none() && arena_ok(owner) => {}
                StmtKind::Borrow { dst, src, .. } | StmtKind::Move { dst, src } if dst.is_bare() && src.is_bare() => {
                    plain_class(cx, body.local(src.local)?.ty)?;
                    plain_class(cx, body.local(dst.local)?.ty)?;
                }
                StmtKind::Alloc { dst, kind: AllocKind::EnumSlot { enum_name, variant }, arena } if dst.is_bare() && cx.debug.is_none() && arena_ok(arena) => {
                    let ty = body.local(dst.local)?.ty;
                    if plain_enum(cx, ty)? != *enum_name || cx.enum_layout(enum_name)?.variants.get(*variant as usize).is_none() {
                        return None;
                    }
                }
                StmtKind::Alloc { dst, kind: AllocKind::Object { class }, arena } if dst.is_bare() && cx.debug.is_none() && arena_ok(arena) => {
                    let ty = body.local(dst.local)?.ty;
                    if plain_class(cx, ty)? != *class {
                        return None;
                    }
                }
                StmtKind::Intrinsic { dst: None, op: IntrinsicOp::PrintInt | IntrinsicOp::PrintlnInt, args, arena: None } if args.len() == 1 => {
                    operand(&args[0])?;
                    let ty = args[0].place().and_then(|p| body.local(p.local)).map(|l| l.ty);
                    if ty.is_some_and(|t| cx.int_type(t).is_none()) {
                        return None;
                    }
                }
                _ => return None,
            }
        }
        match &block.term.as_ref()?.kind {
            TermKind::Goto(_) | TermKind::Unreachable | TermKind::Abort(_) | TermKind::Return => {}
            TermKind::Switch { value, .. } => operand(value)?,
            TermKind::Call { callee, args, hidden, dest, .. } => {
                let (_, target) = callees.get(callee.as_str())?;
                let abi = CallAbi::of(cx, target);
                if abi.has_result_sink() || hidden.len() != abi.hidden_len() || hidden.iter().any(|h| *h != ArenaRef::Current) {
                    return None;
                }
                value(target.return_ty)?;
                if args.len() != target.params.len() || !dest.is_bare() {
                    return None;
                }
                for (a, p) in args.iter().zip(&target.params) {
                    general(a)?;
                    value(p.ty)?;
                }
            }
            TermKind::Match { scrutinee, arms } => {
                if !scrutinee.is_bare() {
                    return None;
                }
                let name = plain_enum(cx, body.local(scrutinee.local)?.ty)?;
                for arm in arms {
                    match &arm.pat {
                        MirPat::Wildcard | MirPat::Bind => {}
                        MirPat::Variant { enum_name, index, fields } if *enum_name == name && fields.iter().all(|f| matches!(f, MirPat::Wildcard | MirPat::Bind)) => {
                            cx.enum_layout(&name)?.variants.get(*index as usize)?;
                        }
                        _ => return None,
                    }
                    for (l, src) in &arm.bindings {
                        let lt = body.local(*l)?.ty;
                        if src.local != scrutinee.local || scalar(lt).is_none() || place_ty(cx, body, src)? != lt {
                            return None;
                        }
                    }
                }
            }
        }
    }
    Some(())
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
            lower.stmt(s)?;
        }
        let term = block.term.as_ref().expect("verified MIR has terminators");
        lower.term(term)?;
    }
    Ok(())
}

impl<'a, 'ctx> Lower<'a, 'ctx> {
    fn prologue(&mut self, hidden: usize) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let mut params = self.llvm_fn.get_param_iter().skip(hidden);
        for (i, decl) in self.body.locals.iter().enumerate() {
            let local = Local(i as u32);
            if decl.kind == LocalKind::Hidden(HiddenKind::DynamicArena) {
                self.slots.insert(local, cx.builder.build_alloca(cx.context.ptr_type(inkwell::AddressSpace::default()), "dynamic.arena")?);
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
        op.place().and_then(|p| self.body.local(p.local)).map(|l| l.ty)
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
            ArenaRef::Fresh(n) => self.load_ptr(self.slots[&Local(*n)]),
            ArenaRef::Root => Ok(cx
                .builder
                .build_call(cx.arena_root_fn(), &[], "arena.root")?
                .try_as_basic_value()
                .basic()
                .expect("bork_arena_root returns a pointer")
                .into_pointer_value()),
            _ => unreachable!("filtered by supported"),
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
                _ => unreachable!("filtered by supported"),
            }
        }
        Ok((ptr, ty))
    }

    /// The object pointer inside the `{arena, object}` handle stored at `slot`.
    fn object_ptr(&self, slot: PointerValue<'ctx>) -> Result<PointerValue<'ctx>, Diagnostic> {
        let handle = self.cx.builder.build_load(self.cx.object_handle_type(), slot, "object")?.into_struct_value();
        Ok(self.cx.builder.build_extract_value(handle, 1, "object.ptr")?.into_pointer_value())
    }

    /// `bork_arena_alloc` of `size`/`align` bytes, wrapped as the `{arena, object}` handle and stored in `dst`.
    fn alloc_handle(&self, dst: &Place, arena: PointerValue<'ctx>, size: inkwell::values::IntValue<'ctx>, align: inkwell::values::IntValue<'ctx>, tag: Option<(inkwell::types::StructType<'ctx>, u32)>) -> Result<(), Diagnostic> {
        let cx = self.cx;
        let object = cx
            .builder
            .build_call(cx.arena_alloc_fn(), &[arena.into(), size.into(), align.into()], "object")?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_alloc returns a pointer")
            .into_pointer_value();
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
            // shortcut: a plain handle copy is enough while `Ref`/drop state is excluded; #225 must clear the source on `Move`.
            StmtKind::Borrow { dst, src, .. } | StmtKind::Move { dst, src } => {
                let (ptr, ty) = self.place_ptr(src)?;
                let v = cx.builder.build_load(cx.basic_type(ty).expect("checked value type"), ptr, "")?;
                self.store(dst, v)?;
            }
            StmtKind::Alloc { dst, kind: AllocKind::EnumSlot { enum_name, variant }, arena } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let layout = cx.enum_layout(enum_name).expect("checked layout");
                let i64 = cx.context.i64_type();
                let (size, align) = (i64.const_int(layout.size, false), i64.const_int(layout.align, false));
                self.alloc_handle(dst, arena, size, align, Some((layout.variants[*variant as usize], *variant)))?;
            }
            StmtKind::Alloc { dst, kind: AllocKind::Object { class }, arena } => {
                let arena = self.arena_ref(arena, stmt.region)?;
                let record = cx.record_type(class).expect("checked record");
                let size = record.size_of().ok_or_else(|| not_yet_supported("unsized record", None))?;
                self.alloc_handle(dst, arena, size, record.get_alignment(), None)?;
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
            }
            StmtKind::StorageLive(_) | StmtKind::StorageDead(_) => {}
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
            TermKind::Call { callee, args, dest, target, .. } => {
                let (callee_fn, function) = self.callees[callee.as_str()];
                let abi = CallAbi::of(cx, function);
                let mut call_args: Vec<BasicMetadataValueEnum<'ctx>> = Vec::new();
                if abi.hidden_len() > 0 {
                    call_args.push(self.arena_of(term.region)?.into());
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
                let name = plain_enum(cx, self.body.locals[scrutinee.local.0 as usize].ty).expect("checked plain enum");
                let object = self.object_ptr(self.slots[&scrutinee.local])?;
                let first = cx.enum_layout(&name).expect("checked layout").variants[0];
                let tag_ptr = builder.build_struct_gep(first, object, 0, "enum.tag")?;
                let tag = builder.build_load(cx.context.i32_type(), tag_ptr, "tag")?.into_int_value();
                for arm in arms {
                    let bind = cx.context.append_basic_block(self.llvm_fn, "arm");
                    let next = cx.context.append_basic_block(self.llvm_fn, "arm.next");
                    match &arm.pat {
                        MirPat::Variant { index, .. } => {
                            let hit = builder.build_int_compare(IntPredicate::EQ, tag, cx.context.i32_type().const_int(u64::from(*index), false), "arm.hit")?;
                            builder.build_conditional_branch(hit, bind, next)?;
                        }
                        _ => {
                            builder.build_unconditional_branch(bind)?;
                        }
                    }
                    cx.position_at_end(bind);
                    for (l, src) in &arm.bindings {
                        let (ptr, ty) = self.place_ptr(src)?;
                        let v = builder.build_load(cx.basic_type(ty).expect("checked scalar"), ptr, "bound")?;
                        self.store(&Place::local(*l), v)?;
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
