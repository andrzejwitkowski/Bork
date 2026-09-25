use std::collections::HashMap;

use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::basic_block::BasicBlock;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::codegen::regions::{RegionEmitter, RegionSite};
use crate::region_walk::{RegionWalkRef, WalkDriver};
use crate::diag::Diagnostic;
use crate::hir::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, Ty};

use super::arena::ArenaCalls;
use super::context::Codegen;
use super::{not_yet_supported, region_walk_codegen_error, schedule_error};

pub type Callees<'h, 'ctx> = HashMap<&'h str, (FunctionValue<'ctx>, &'h HirFunction)>;

pub type Regions<'report, 'a, 'ctx> = RegionEmitter<'report, ArenaCalls<'a, 'ctx>>;

/// `main` keeps its C name and returns `i32` even when declared `unit`; other functions get
/// internal `bork.`-prefixed symbols so they cannot collide with libc or the runtime.
pub fn declare_function<'ctx>(
    cx: &Codegen<'ctx>,
    function: &HirFunction,
) -> Result<FunctionValue<'ctx>, Diagnostic> {
    let params = function
        .params
        .iter()
        .map(|param| {
            cx.basic_type(&param.ty)
                .map(BasicMetadataTypeEnum::from)
                .ok_or_else(|| not_yet_supported(&format!("parameter type `{}`", param.ty), None))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if function.name == "main" {
        if !params.is_empty() {
            return Err(not_yet_supported("parameters on `main`", None));
        }
        if function.return_ty != Ty::i32() && function.return_ty != Ty::unit() {
            return Err(not_yet_supported(
                &format!("`main` returning `{}`", function.return_ty),
                None,
            ));
        }
        let fn_type = cx.context.i32_type().fn_type(&[], false);
        return Ok(cx.module.add_function("main", fn_type, None));
    }

    let fn_type = if function.return_ty == Ty::unit() {
        cx.context.void_type().fn_type(&params, false)
    } else {
        cx.basic_type(&function.return_ty)
            .ok_or_else(|| {
                not_yet_supported(&format!("return type `{}`", function.return_ty), None)
            })?
            .fn_type(&params, false)
    };
    Ok(cx.module.add_function(
        &format!("bork.{}", function.name),
        fn_type,
        Some(Linkage::Internal),
    ))
}

pub fn emit_function<'report, 'a, 'ctx>(
    cx: &'a Codegen<'ctx>,
    callees: &Callees<'_, 'ctx>,
    regions: &mut Regions<'report, 'a, 'ctx>,
    roots: &'report [crate::sema::ArenaNode],
    function: &HirFunction,
) -> Result<(), Diagnostic> {
    let (llvm_fn, _) = callees[function.name.as_str()];
    let entry = cx.context.append_basic_block(llvm_fn, "entry");
    cx.builder.position_at_end(entry);

    let mut emitter = FnEmitter {
        cx,
        callees,
        regions,
        function,
        llvm_fn,
        scopes: vec![HashMap::new()],
        alloc_sink: None,
        function_arena: None,
        loop_stack: Vec::new(),
        walk: CodegenWalkState::new(),
    };
    super::region_emit::emit_function_body(&mut emitter, roots, function)?;
    if !cx.current_block_terminated() {
        emitter.build_fallthrough()?;
    }
    Ok(())
}

pub(super) struct Slot<'ctx> {
    pub ptr: PointerValue<'ctx>,
    pub ty: Ty,
    pub home_arena: PointerValue<'ctx>,
}

pub(super) struct FnEmitter<'s, 'report, 'a, 'ctx> {
    pub cx: &'a Codegen<'ctx>,
    pub callees: &'s Callees<'s, 'ctx>,
    pub(super) regions: &'s mut Regions<'report, 'a, 'ctx>,
    pub(super) function: &'s HirFunction,
    pub llvm_fn: FunctionValue<'ctx>,
    pub(super) scopes: Vec<HashMap<String, Slot<'ctx>>>,
    alloc_sink: Option<PointerValue<'ctx>>,
    /// Function-body arena handle; stable for the whole emit even when sibling branches return.
    pub(super) function_arena: Option<PointerValue<'ctx>>,
    loop_stack: Vec<LoopLabels<'ctx>>,
    pub(super) walk: CodegenWalkState<'ctx>,
}

#[derive(Clone, Copy)]
struct LoopLabels<'ctx> {
    exit: BasicBlock<'ctx>,
    continue_target: BasicBlock<'ctx>,
}

/// Driver pin and trailing-value slot while `region_walk` drives a function body.
pub(super) struct CodegenWalkState<'ctx> {
    pub trailing: Option<BasicValueEnum<'ctx>>,
    /// Operand values produced by child `after_expr` calls (post-order walk).
    pub eval_stack: Vec<BasicValueEnum<'ctx>>,
    driver: Option<*mut ()>,
}

impl<'ctx> CodegenWalkState<'ctx> {
    fn new() -> Self {
        Self {
            trailing: None,
            eval_stack: Vec::new(),
            driver: None,
        }
    }

    fn active(&self) -> bool {
        self.driver.is_some()
    }

    fn driver_ptr(&self) -> *mut () {
        self.driver
            .expect("emit_expr region sync requires an active region walk driver")
    }

    fn driver_mut<'a>(ptr: *mut ()) -> &'a mut WalkDriver<'a, RegionWalkRef<'a>> {
        unsafe { &mut *ptr.cast() }
    }
}

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn pin_walk_driver<C: crate::region_walk::ArenaCursor, R>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        f: impl FnOnce(&mut Self) -> R,
    ) -> R {
        let prev = self.walk.driver;
        self.walk.driver = Some(std::ptr::from_mut(driver).cast());
        let out = f(self);
        self.walk.driver = prev;
        out
    }

    pub(super) fn walk_driver_active(&self) -> bool {
        self.walk.active()
    }

    pub(crate) fn codegen_driver_ptr(&self) -> *mut () {
        self.walk.driver_ptr()
    }

    pub(super) fn codegen_driver_mut<'a>(ptr: *mut ()) -> &'a mut WalkDriver<'a, RegionWalkRef<'a>> {
        CodegenWalkState::driver_mut(ptr)
    }

    pub(super) fn value_from_walk(
        &mut self,
        expr: &HirExpr,
        missing: impl FnOnce() -> Diagnostic,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        Self::codegen_driver_mut(self.walk.driver_ptr())
            .walk_expr(self, expr)
            .map_err(region_walk_codegen_error)?;
        self.walk.trailing.take().ok_or_else(|| missing())
    }

    pub fn current_arena(&mut self) -> PointerValue<'ctx> {
        self.regions
            .sink_mut()
            .current()
            .expect("function bodies always have an open arena")
    }

    pub(super) fn sink_arena(&mut self) -> PointerValue<'ctx> {
        self.alloc_sink.unwrap_or_else(|| self.current_arena())
    }

    /// Evaluate sub-expressions without treating them as the assign/return/`concat` sink.
    pub(super) fn without_alloc_sink<R>(
        &mut self,
        f: impl FnOnce(&mut Self) -> Result<R, Diagnostic>,
    ) -> Result<R, Diagnostic> {
        let saved = self.alloc_sink.take();
        let result = f(self);
        self.alloc_sink = saved;
        result
    }

    pub fn lookup(&self, name: &str) -> Option<&Slot<'ctx>> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    /// Code after a terminator still has to be walked to keep regions in step; it goes into
    /// a fresh block with no predecessors.
    pub fn ensure_open_block(&self) {
        if self.cx.current_block_terminated() {
            let dead = self.cx.context.append_basic_block(self.llvm_fn, "dead");
            self.cx.builder.position_at_end(dead);
        }
    }

    pub(super) fn emit_stmt(&mut self, stmt: &HirStmt) -> Result<(), Diagnostic> {
        self.ensure_open_block();
        match stmt {
            HirStmt::Return { value } => self.emit_return(value.as_ref()),
            HirStmt::Block(_) | HirStmt::MoveBlock { .. } | HirStmt::For { .. } | HirStmt::While { .. } => {
                Err(not_yet_supported("region statement outside region walk", None))
            }
            HirStmt::VarDecl {
                name,
                ty,
                value,
                alloc_in_binding,
                ..
            } => {
                let prev = self.alloc_sink;
                if let Some(target) = alloc_in_binding {
                    let slot = self.lookup(target).ok_or_else(|| {
                        not_yet_supported(
                            &format!("hoisted alloc target `{target}` is not in scope"),
                            value.span,
                        )
                    })?;
                    self.alloc_sink = Some(slot.home_arena);
                }
                let value = self.value_from_walk(
                    value,
                    || not_yet_supported("initializer produced no value", value.span),
                )?;
                self.alloc_sink = prev;
                self.declare_local(name, ty, value)
            }
            HirStmt::Assign { target, value } => {
                let name = target.name();
                let slot = self.lookup(name).ok_or_else(|| {
                    not_yet_supported(&format!("assignment to `{name}`"), value.span)
                })?;
                let (ptr, home, array_ty) = (slot.ptr, slot.home_arena, slot.ty.clone());
                let prev = self.alloc_sink;
                self.alloc_sink = Some(home);
                let stored = self.value_from_walk(
                    value,
                    || not_yet_supported("assignment value missing", value.span),
                )?;
                self.alloc_sink = prev;
                match target {
                    HirAssignTarget::Name { .. } => {
                        self.cx.builder.build_store(ptr, stored)?;
                        Ok(())
                    }
                    HirAssignTarget::Index { index, .. } => {
                        self.emit_index_store(ptr, &array_ty, index, stored, value.span)
                    }
                }
            }
            HirStmt::Expr(expr) => {
                Self::codegen_driver_mut(self.codegen_driver_ptr())
                    .walk_expr(self, expr)
                    .map_err(region_walk_codegen_error)?;
                self.walk.trailing.take();
                Ok(())
            }
            HirStmt::Break { .. } => self.emit_break(),
            HirStmt::Continue { .. } => self.emit_continue(),
        }
    }

    fn emit_break(&mut self) -> Result<(), Diagnostic> {
        let labels = *self
            .loop_stack
            .last()
            .ok_or_else(|| not_yet_supported("`break` outside of a loop", None))?;
        self.cx.builder.build_unconditional_branch(labels.exit)?;
        Ok(())
    }

    fn emit_continue(&mut self) -> Result<(), Diagnostic> {
        let labels = *self
            .loop_stack
            .last()
            .ok_or_else(|| not_yet_supported("`continue` outside of a loop", None))?;
        self.cx.builder.build_unconditional_branch(labels.continue_target)?;
        Ok(())
    }

    /// `for (name in lo..hi)` — iterator expression is already evaluated by `region_walk`.
    pub(super) fn emit_for_with_driver<C: crate::region_walk::ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        name: &str,
        iter: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), Diagnostic> {
        self.pin_walk_driver(driver, |emitter| {
            emitter.emit_for_with_driver_inner(name, iter, body)
        })
    }

    fn emit_for_with_driver_inner(
        &mut self,
        name: &str,
        iter: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), Diagnostic> {
        let HirExprKind::Binary {
            op: BinOp::RangeTo,
            lhs,
            rhs,
        } = &iter.kind
        else {
            return Err(not_yet_supported("this `for` iterator", iter.span));
        };
        let index_ty = Ty::i32();
        let start = self.emit_int(lhs, &index_ty)?;
        let end = self.emit_int(rhs, &index_ty)?;
        self.declare_local(name, &index_ty, start.into())?;
        let index = self.lookup(name).expect("loop index was just declared").ptr;

        let ptr = self.codegen_driver_ptr();
        let driver = Self::codegen_driver_mut(ptr);
        crate::region_walk::region_enter(driver, self, RegionSite::For, body)
            .map_err(region_walk_codegen_error)?;
        self.check_arena()?;

        let cx = self.cx;
        let (context, builder) = (cx.context, &cx.builder);
        let i32_type = context.i32_type();
        let header_bb = context.append_basic_block(self.llvm_fn, "for.header");
        let body_bb = context.append_basic_block(self.llvm_fn, "for.body");
        let latch_bb = context.append_basic_block(self.llvm_fn, "for.latch");
        let exit_bb = context.append_basic_block(self.llvm_fn, "for.exit");
        builder.build_unconditional_branch(header_bb)?;

        self.loop_stack.push(LoopLabels {
            exit: exit_bb,
            continue_target: latch_bb,
        });

        builder.position_at_end(header_bb);
        let current = builder.build_load(i32_type, index, name)?.into_int_value();
        let in_range = builder.build_int_compare(IntPredicate::SLT, current, end, "for.cond")?;
        builder.build_conditional_branch(in_range, body_bb, exit_bb)?;

        builder.position_at_end(body_bb);
        self.scopes.push(HashMap::new());
        let (peeled, _) = crate::hir::peel_blocks(body);
        driver.walk_block(self, peeled, None).map_err(|err| {
            region_walk_codegen_error(err)
        })?;
        self.scopes.pop();
        if !cx.current_block_terminated() {
            builder.build_unconditional_branch(latch_bb)?;
        }

        builder.position_at_end(latch_bb);
        self.regions.latch().map_err(schedule_error)?;
        self.check_arena()?;
        let current = builder.build_load(i32_type, index, name)?.into_int_value();
        let next = builder.build_int_add(current, i32_type.const_int(1, false), "for.next")?;
        builder.build_store(index, next)?;
        builder.build_unconditional_branch(header_bb)?;

        builder.position_at_end(exit_bb);
        self.loop_stack.pop();
        crate::region_walk::region_exit(driver, self, RegionSite::For)
            .map_err(region_walk_codegen_error)?;
        Ok(())
    }

    pub(super) fn emit_while_with_driver<C: crate::region_walk::ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        cond: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), Diagnostic> {
        self.pin_walk_driver(driver, |emitter| {
            emitter.emit_while_with_driver_inner(cond, body)
        })
    }

    fn emit_while_with_driver_inner(
        &mut self,
        cond: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), Diagnostic> {
        let ptr = self.codegen_driver_ptr();
        let driver = Self::codegen_driver_mut(ptr);
        crate::region_walk::region_enter(driver, self, RegionSite::While, body)
            .map_err(region_walk_codegen_error)?;
        self.check_arena()?;

        let cx = self.cx;
        let (context, builder) = (cx.context, &cx.builder);
        let header_bb = context.append_basic_block(self.llvm_fn, "while.header");
        let body_bb = context.append_basic_block(self.llvm_fn, "while.body");
        let latch_bb = context.append_basic_block(self.llvm_fn, "while.latch");
        let exit_bb = context.append_basic_block(self.llvm_fn, "while.exit");
        builder.build_unconditional_branch(header_bb)?;

        self.loop_stack.push(LoopLabels {
            exit: exit_bb,
            continue_target: latch_bb,
        });

        builder.position_at_end(header_bb);
        let keep_going = self.emit_bool(cond)?;
        builder.build_conditional_branch(keep_going, body_bb, exit_bb)?;

        builder.position_at_end(body_bb);
        self.scopes.push(HashMap::new());
        let (peeled, _) = crate::hir::peel_blocks(body);
        driver.walk_block(self, peeled, None).map_err(|err| {
            region_walk_codegen_error(err)
        })?;
        self.scopes.pop();
        if !cx.current_block_terminated() {
            builder.build_unconditional_branch(latch_bb)?;
        }

        builder.position_at_end(latch_bb);
        self.regions.latch().map_err(schedule_error)?;
        self.check_arena()?;
        builder.build_unconditional_branch(header_bb)?;

        builder.position_at_end(exit_bb);
        self.loop_stack.pop();
        crate::region_walk::region_exit(driver, self, RegionSite::While)
            .map_err(region_walk_codegen_error)?;
        Ok(())
    }

    fn emit_return(&mut self, value: Option<&HirExpr>) -> Result<(), Diagnostic> {
        let return_ty = &self.function.return_ty;
        let value = match value {
            Some(value) if *return_ty != Ty::unit() => Some(self.value_from_walk(
                value,
                || not_yet_supported("return value missing", value.span),
            )?),
            Some(value) => {
                Self::codegen_driver_mut(self.codegen_driver_ptr())
                    .walk_expr(self, value)
                    .map_err(region_walk_codegen_error)?;
                self.walk.trailing.take();
                None
            }
            None => None,
        };
        self.regions.sink_mut().unwind()?;
        match value {
            Some(value) => self.cx.builder.build_return(Some(&value)).map(drop)?,
            None => self.build_unit_return()?,
        }
        Ok(())
    }

    /// Typeck guarantees non-`unit` functions return on every path, so falling off their end
    /// is unreachable.
    fn build_fallthrough(&self) -> Result<(), Diagnostic> {
        if self.function.return_ty == Ty::unit() {
            self.build_unit_return()?;
        } else {
            self.cx.builder.build_unreachable()?;
        }
        Ok(())
    }

    fn build_unit_return(&self) -> Result<(), inkwell::builder::BuilderError> {
        let builder = &self.cx.builder;
        if self.function.name == "main" {
            builder.build_return(Some(&self.cx.context.i32_type().const_zero()))?;
        } else {
            builder.build_return(None)?;
        }
        Ok(())
    }

    pub(super) fn declare_local(
        &mut self,
        name: &str,
        ty: &Ty,
        value: BasicValueEnum<'ctx>,
    ) -> Result<(), Diagnostic> {
        let llvm_ty = self
            .cx
            .basic_type(ty)
            .ok_or_else(|| not_yet_supported(&format!("local of type `{ty}`"), None))?;
        let ptr = self.entry_alloca(llvm_ty, name)?;
        self.cx.builder.build_store(ptr, value)?;
        let home_arena = if ty.uses_arena_storage() {
            self.sink_arena()
        } else {
            self.function_arena
                .expect("function arena is set before locals are declared")
        };
        self.scopes.last_mut().expect("function scope").insert(
            name.to_owned(),
            Slot {
                ptr,
                ty: ty.clone(),
                home_arena,
            },
        );
        Ok(())
    }

    /// Allocas go at the top of the entry block so each slot is allocated once per call.
    fn entry_alloca(
        &self,
        ty: impl BasicType<'ctx>,
        name: &str,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let entry = self
            .llvm_fn
            .get_first_basic_block()
            .expect("entry block exists");
        let builder = self.cx.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => builder.position_before(&first),
            None => builder.position_at_end(entry),
        }
        Ok(builder.build_alloca(ty, name)?)
    }

    pub(super) fn pop_region(&mut self) -> Result<(), Diagnostic> {
        self.regions.exit().map_err(schedule_error)?;
        self.check_arena()
    }

    pub(super) fn check_arena(&mut self) -> Result<(), Diagnostic> {
        Ok(self.regions.sink_mut().take_error()?)
    }

}
