//! LLVM emission driven by `region_walk` (same visitor as schedule / stamp).

use std::collections::HashMap;
use std::iter;

use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicType;
use inkwell::values::{BasicValue, BasicValueEnum};

use crate::codegen::regions::RegionSite;
use crate::diag::Diagnostic;
use crate::hir::{HirBlock, HirExpr, HirFunction, HirStmt, Ty};
use crate::region_walk::{
    region_enter, region_exit, ArenaCursor, RegionVisitor, WalkDriver, WalkError,
};
use crate::sema::ArenaNode;

use super::emit_fn::FnEmitter;
use super::schedule_error;
use super::{not_yet_supported, region_walk_codegen_error};

fn map_emit_err(result: Result<(), Diagnostic>) -> Result<(), WalkError> {
    result.map_err(WalkError::from_diagnostic)
}

pub(super) enum IfKind<'a> {
    Unit,
    Value(&'a Ty),
}

impl IfKind<'_> {
    pub(super) fn from_ty(ty: &Ty) -> IfKind<'_> {
        if *ty == Ty::unit() {
            IfKind::Unit
        } else {
            IfKind::Value(ty)
        }
    }
}

impl<'s, 'report, 'a, 'ctx> FnEmitter<'s, 'report, 'a, 'ctx> {
    fn emit_region_with_driver(
        &mut self,
        site: RegionSite,
        block: &HirBlock,
        value_ty: Option<&Ty>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, WalkError> {
        let ptr = self.codegen_driver_ptr();
        let driver = FnEmitter::codegen_driver_mut(ptr);
        region_enter(driver, self, site, block)?;
        let (peeled, _) = crate::hir::peel_blocks(block);
        self.walk.trailing = None;
        driver.walk_block(self, peeled, value_ty)?;
        let value = self.walk.trailing.take();
        region_exit(FnEmitter::codegen_driver_mut(ptr), self, site)?;
        Ok(value)
    }

    pub(super) fn emit_if_with_driver(
        &mut self,
        cond: &HirExpr,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        kind: IfKind<'_>,
    ) -> Result<(), Diagnostic> {
        let (value_ty, llvm_ty) = match kind {
            IfKind::Unit => (None, None),
            IfKind::Value(ty) => {
                let llvm = self.cx.basic_type(ty);
                (llvm.is_some().then_some(ty), llvm)
            }
        };
        let cond = self.emit_bool(cond)?;

        let context = self.cx.context;
        let then_bb = context.append_basic_block(self.llvm_fn, "then");
        let else_bb = else_block.map(|_| context.append_basic_block(self.llvm_fn, "else"));
        let merge_bb = context.append_basic_block(self.llvm_fn, "endif");
        self.cx
            .builder
            .build_conditional_branch(cond, then_bb, else_bb.unwrap_or(merge_bb))?;

        let branches = iter::once((RegionSite::IfThen, then_block, then_bb)).chain(
            else_block
                .zip(else_bb)
                .map(|(block, bb)| (RegionSite::IfElse, block, bb)),
        );
        let mut incoming: Vec<(BasicValueEnum<'ctx>, BasicBlock<'ctx>)> = Vec::new();
        for (site, block, bb) in branches {
            self.cx.builder.position_at_end(bb);
            let value = self
                .emit_region_with_driver(site, block, value_ty)
                .map_err(region_walk_codegen_error)?;
            if self.cx.current_block_terminated() {
                continue;
            }
            let end = self
                .cx
                .builder
                .get_insert_block()
                .expect("builder is positioned");
            self.cx.builder.build_unconditional_branch(merge_bb)?;
            if value_ty.is_some() {
                let value = value.ok_or_else(|| {
                    not_yet_supported("an `if` branch without a trailing value", None)
                })?;
                incoming.push((value, end));
            }
        }
        self.cx.builder.position_at_end(merge_bb);

        if let Some(result_ty) = llvm_ty {
            if !incoming.is_empty() {
                let phi = self
                    .cx
                    .builder
                    .build_phi(result_ty.as_basic_type_enum(), "if")?;
                let incoming: Vec<(&dyn BasicValue<'ctx>, BasicBlock<'ctx>)> = incoming
                    .iter()
                    .map(|(value, block)| (value as &dyn BasicValue<'ctx>, *block))
                    .collect();
                phi.add_incoming(&incoming);
                self.walk.trailing = Some(phi.as_basic_value());
            } else {
                self.walk.trailing = Some(result_ty.const_zero());
            }
        }
        Ok(())
    }

    fn walk_step(
        &mut self,
        step: impl FnOnce(&mut Self) -> Result<(), Diagnostic>,
    ) -> Result<(), WalkError> {
        step(self).map_err(WalkError::from_diagnostic)
    }
}

impl<'s, 'report, 'a, 'ctx> RegionVisitor for FnEmitter<'s, 'report, 'a, 'ctx> {
    fn short_circuit_logical_operands(&self) -> bool {
        true
    }

    fn begin_function_body(
        &mut self,
        _name: &str,
        root: &ArenaNode,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        self.walk_step(|emitter| {
            emitter
                .regions
                .open_known(root, body, None)
                .map_err(schedule_error)?;
            emitter.check_arena()?;
            emitter.function_arena = emitter.regions.sink_mut().current();
            for (param, value) in emitter
                .function
                .params
                .iter()
                .zip(emitter.llvm_fn.get_param_iter())
            {
                emitter.declare_local(&param.name, &param.ty, value)?;
            }
            Ok(())
        })
    }

    fn end_function_body(&mut self) -> Result<(), WalkError> {
        self.walk_step(|emitter| emitter.pop_region())?;
        self.regions.next_function += 1;
        Ok(())
    }

    fn enter_region(
        &mut self,
        site: RegionSite,
        child: &ArenaNode,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        self.walk_step(|emitter| {
            emitter
                .regions
                .open_known(child, body, Some(site))
                .map_err(schedule_error)?;
            emitter.check_arena()?;
            emitter.scopes.push(HashMap::new());
            Ok(())
        })
    }

    fn loop_latch(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        Ok(())
    }

    fn exit_region(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        self.scopes.pop();
        if self.cx.current_block_terminated() {
            self.walk_step(|emitter| emitter.regions.exit_after_return().map_err(schedule_error))?;
        } else {
            self.walk_step(|emitter| emitter.pop_region())?;
        }
        Ok(())
    }

    fn skip_closure(&mut self, _child: &ArenaNode) -> Result<(), WalkError> {
        Ok(())
    }

    fn on_stmt<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        stmt: &HirStmt,
    ) -> Result<(), WalkError> {
        self.pin_walk_driver(driver, |emitter| emitter.walk_step(|e| e.emit_stmt(stmt)))
    }

    fn after_expr<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
    ) -> Result<(), WalkError> {
        self.pin_walk_driver(driver, |emitter| {
            emitter.walk_step(|e| {
                if let Some(value) = e.emit_after_walk(expr)? {
                    e.walk.eval_stack.push(value);
                    e.walk.trailing = Some(value);
                }
                Ok(())
            })
        })
    }

    fn on_trailing_value<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
        ty: &Ty,
    ) -> Result<(), WalkError> {
        driver.walk_expr(self, expr)?;
        if self.walk.trailing.is_some() {
            return Ok(());
        }
        self.pin_walk_driver(driver, |emitter| {
            emitter.walk_step(|e| {
                e.ensure_open_block();
                e.walk.trailing = Some(e.emit_value(expr, ty)?);
                Ok(())
            })
        })
    }

    fn for_loop<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        name: &str,
        iter: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        let result = self.emit_for_with_driver(driver, name, iter, body);
        map_emit_err(result)
    }

    fn while_loop<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        cond: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        let result = self.emit_while_with_driver(driver, cond, body);
        map_emit_err(result)
    }

    fn if_expr<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        cond: &HirExpr,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        result_ty: &Ty,
    ) -> Result<(), WalkError> {
        self.pin_walk_driver(driver, |emitter| {
            emitter.walk_step(|e| {
                let depth = e.walk.eval_stack.len();
                e.emit_if_with_driver(cond, then_block, else_block, IfKind::from_ty(result_ty))?;
                e.walk.eval_stack.truncate(depth);
                if let Some(value) = e.walk.trailing {
                    e.walk.eval_stack.push(value);
                }
                Ok(())
            })
        })
    }
}

pub fn emit_function_body<'report, 'a, 'ctx>(
    emitter: &mut FnEmitter<'_, 'report, 'a, 'ctx>,
    roots: &[ArenaNode],
    function: &HirFunction,
) -> Result<(), Diagnostic> {
    let root_index = emitter.regions.function_index();
    if let Err(walk) =
        crate::region_walk::walk_function_readonly(function, root_index, roots, emitter)
    {
        return Err(super::resolve_walk_failure(walk));
    }
    Ok(())
}
