use std::collections::HashMap;

use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::codegen::regions::{RegionEmitter, RegionSite};
use crate::diag::Diagnostic;
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, Ty};

use super::arena::ArenaCalls;
use super::context::Codegen;
use super::{not_yet_supported, schedule_error};

pub type Callees<'h, 'ctx> = HashMap<&'h str, (FunctionValue<'ctx>, &'h HirFunction)>;

pub type Regions<'r, 'a, 'ctx> = RegionEmitter<'r, ArenaCalls<'a, 'ctx>>;

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

pub fn emit_function<'a, 'ctx>(
    cx: &'a Codegen<'ctx>,
    callees: &Callees<'_, 'ctx>,
    regions: &mut Regions<'_, 'a, 'ctx>,
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
    };
    for (param, value) in function.params.iter().zip(llvm_fn.get_param_iter()) {
        emitter.declare_local(&param.name, &param.ty, value)?;
    }

    let body = emitter
        .regions
        .enter_function(&function.name, &function.body)
        .map_err(schedule_error)?;
    emitter.check_arena()?;
    emitter.emit_stmts(body, None)?;
    emitter.exit_region()?;
    if !cx.current_block_terminated() {
        emitter.build_fallthrough()?;
    }
    Ok(())
}

pub(super) struct Slot<'ctx> {
    pub ptr: PointerValue<'ctx>,
    pub ty: Ty,
}

pub(super) struct FnEmitter<'s, 'r, 'a, 'ctx> {
    pub cx: &'a Codegen<'ctx>,
    pub callees: &'s Callees<'s, 'ctx>,
    regions: &'s mut Regions<'r, 'a, 'ctx>,
    function: &'s HirFunction,
    pub llvm_fn: FunctionValue<'ctx>,
    scopes: Vec<HashMap<String, Slot<'ctx>>>,
}

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub fn emit_stmts(
        &mut self,
        block: &HirBlock,
        value_ty: Option<&Ty>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let Some((last, prefix)) = block.stmts.split_last() else {
            return Ok(None);
        };
        for stmt in prefix {
            self.emit_stmt(stmt)?;
        }
        match (last, value_ty) {
            (HirStmt::Expr(expr), Some(ty)) => {
                self.ensure_open_block();
                Ok(Some(self.emit_value(expr, ty)?))
            }
            _ => self.emit_stmt(last).map(|()| None),
        }
    }

    pub fn emit_region(
        &mut self,
        site: RegionSite,
        block: &HirBlock,
        value_ty: Option<&Ty>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let body = self.regions.enter(site, block).map_err(schedule_error)?;
        self.check_arena()?;
        self.scopes.push(HashMap::new());
        let value = self.emit_stmts(body, value_ty);
        self.scopes.pop();
        let value = value?;
        self.exit_region()?;
        Ok(value)
    }

    pub fn current_arena(&mut self) -> PointerValue<'ctx> {
        self.regions
            .sink_mut()
            .current()
            .expect("function bodies always have an open arena")
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

    fn emit_stmt(&mut self, stmt: &HirStmt) -> Result<(), Diagnostic> {
        self.ensure_open_block();
        match stmt {
            HirStmt::Return { value } => self.emit_return(value.as_ref()),
            HirStmt::Block(body) => self.emit_region(RegionSite::Block, body, None).map(drop),
            // MoveBlock captures are enforced by sema; codegen only opens the move region.
            HirStmt::MoveBlock { body, .. } => self
                .emit_region(RegionSite::MoveBlock, body, None)
                .map(drop),
            HirStmt::VarDecl {
                name, ty, value, ..
            } => {
                let value = self.emit_value(value, ty)?;
                self.declare_local(name, ty, value)
            }
            HirStmt::Assign { name, value } => {
                let slot = self.lookup(name).ok_or_else(|| {
                    not_yet_supported(&format!("assignment to `{name}`"), value.span)
                })?;
                let (ptr, ty) = (slot.ptr, slot.ty.clone());
                let value = self.emit_value(value, &ty)?;
                self.cx.builder.build_store(ptr, value)?;
                Ok(())
            }
            HirStmt::Expr(expr) => self.emit_expr(expr).map(drop),
            HirStmt::For { name, iter, body } => {
                self.scopes.push(HashMap::new());
                let result = self.emit_for(name, iter, body);
                self.scopes.pop();
                result
            }
        }
    }

    /// `for (name in lo..hi)` over the half-open range, bounds evaluated once. The loop arena
    /// is pushed before the header, reset at the latch after every iteration, and popped on exit.
    fn emit_for(&mut self, name: &str, iter: &HirExpr, body: &HirBlock) -> Result<(), Diagnostic> {
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

        let body = self
            .regions
            .enter(RegionSite::For, body)
            .map_err(schedule_error)?;
        self.check_arena()?;

        let cx = self.cx;
        let (context, builder) = (cx.context, &cx.builder);
        let i32_type = context.i32_type();
        let header_bb = context.append_basic_block(self.llvm_fn, "for.header");
        let body_bb = context.append_basic_block(self.llvm_fn, "for.body");
        let latch_bb = context.append_basic_block(self.llvm_fn, "for.latch");
        let exit_bb = context.append_basic_block(self.llvm_fn, "for.exit");
        builder.build_unconditional_branch(header_bb)?;

        builder.position_at_end(header_bb);
        let current = builder.build_load(i32_type, index, name)?.into_int_value();
        let in_range = builder.build_int_compare(IntPredicate::SLT, current, end, "for.cond")?;
        builder.build_conditional_branch(in_range, body_bb, exit_bb)?;

        builder.position_at_end(body_bb);
        self.scopes.push(HashMap::new());
        let emitted = self.emit_stmts(body, None);
        self.scopes.pop();
        emitted?;
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
        self.exit_region()
    }

    fn emit_return(&mut self, value: Option<&HirExpr>) -> Result<(), Diagnostic> {
        let return_ty = &self.function.return_ty;
        let value = match value {
            Some(value) if *return_ty != Ty::unit() => Some(self.emit_value(value, return_ty)?),
            Some(value) => {
                self.emit_expr(value)?;
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

    fn declare_local(
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
        self.scopes.last_mut().expect("function scope").insert(
            name.to_owned(),
            Slot {
                ptr,
                ty: ty.clone(),
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

    fn exit_region(&mut self) -> Result<(), Diagnostic> {
        self.regions.exit().map_err(schedule_error)?;
        self.check_arena()
    }

    fn check_arena(&mut self) -> Result<(), Diagnostic> {
        Ok(self.regions.sink_mut().take_error()?)
    }
}
