//! LLVM emission driven by `region_walk` (same visitor as schedule / stamp).

use std::collections::HashMap;
use std::iter;

use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicType;
use inkwell::values::{BasicValue, BasicValueEnum, IntValue, PointerValue, StructValue};

use crate::codegen::regions::RegionSite;
use crate::diag::Diagnostic;
use crate::hir::{
    HirBlock, HirConditionalBinding, HirConditionalBindings, HirExpr, HirFunction, HirStmt, Ty,
};
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
        let previous = self.alloc_sink;
        if value_ty.is_some_and(|ty| ty.is_managed_ref() || ty.record_name().is_some()) {
            self.alloc_sink = Some(self.sink_arena());
        }
        let ptr = self.codegen_driver_ptr();
        let driver = FnEmitter::codegen_driver_mut(ptr);
        region_enter(driver, self, site, block)?;
        let (peeled, _) = crate::hir::peel_blocks(block);
        self.walk.trailing = None;
        driver.walk_block(self, peeled, value_ty)?;
        let value = self.walk.trailing.take();
        self.alloc_sink = previous;
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

    fn emit_presence_with_driver<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        bindings: &HirConditionalBindings,
        some_block: &HirBlock,
        none_block: Option<&HirBlock>,
        result_ty: &Ty,
    ) -> Result<(), Diagnostic> {
        let stack_depth = self.walk.eval_stack.len();
        let value_ty = (*result_ty != Ty::unit()).then_some(result_ty);
        let result_sink = self.sink_arena();
        let previous = self.alloc_sink;
        // The head source is evaluated in the enclosing arena, as with single-binding syntax.
        let (observation, object, control, is_absent) =
            self.emit_guard_source(driver, &bindings.head.value)?;
        let mut split = self.open_presence("ref", is_absent, value_ty)?;
        let outer_depth = self.regions.sink_mut().handle_count();
        region_enter(driver, self, RegionSite::PresenceSome, some_block)
            .map_err(region_walk_codegen_error)?;
        self.bind_observation(&bindings.head, observation, object, control)?;
        // Tail sources are evaluated inside the presence region (as in sema), so their
        // allocations use that region's arena; only the arm's values go to the result sink.
        self.emit_chained_guards(driver, &bindings.tail, outer_depth, split.absent_target())?;
        if result_ty.is_managed_ref() || result_ty.record_name().is_some() {
            self.alloc_sink = Some(result_sink);
        }
        let branch_value = self.emit_presence_arm(
            driver,
            RegionSite::PresenceSome,
            some_block,
            value_ty,
            result_ty,
            result_sink,
        )?;
        self.end_present_arm(&mut split, branch_value)?;
        let absent_value = match none_block {
            Some(block) => {
                region_enter(driver, self, RegionSite::PresenceNone, block)
                    .map_err(region_walk_codegen_error)?;
                self.emit_presence_arm(
                    driver,
                    RegionSite::PresenceNone,
                    block,
                    value_ty,
                    result_ty,
                    result_sink,
                )?
            }
            None => None,
        };
        self.alloc_sink = previous;
        let result = self.end_absent_arm(split, absent_value)?;
        self.walk.eval_stack.truncate(stack_depth);
        if let Some(result) = result {
            self.walk.eval_stack.push(result);
            self.walk.trailing = Some(result);
        }
        Ok(())
    }

    /// Walks the body of one `if val` arm and closes its region. A fall-through value is
    /// materialized into `result_sink` before the region's observations are released.
    fn emit_presence_arm<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        site: RegionSite,
        block: &HirBlock,
        value_ty: Option<&Ty>,
        result_ty: &Ty,
        result_sink: PointerValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let (body, _) = crate::hir::peel_blocks(block);
        self.walk.trailing = None;
        driver
            .walk_block(self, body, value_ty)
            .map_err(region_walk_codegen_error)?;
        let mut branch_value = self.walk.trailing.take();
        if !self.cx.current_block_terminated() {
            if let Some(value) = branch_value {
                branch_value = Some(self.materialize_owned_value(value, result_ty, result_sink)?);
            }
        }
        region_exit(driver, self, site).map_err(region_walk_codegen_error)?;
        Ok(branch_value)
    }

    /// Tail sources of an `if val` header. An absent source jumps to a shared cleanup block,
    /// which unwinds arenas opened since the header and then takes the `else` path. Leaves the
    /// insertion point on the all-present path.
    fn emit_chained_guards<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        tail: &[HirConditionalBinding],
        outer_depth: usize,
        absent: BasicBlock<'ctx>,
    ) -> Result<(), Diagnostic> {
        if tail.is_empty() {
            return Ok(());
        }
        let cleanup = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "ref.failure.cleanup");
        for binding in tail {
            let (observation, object, control, is_absent) =
                self.emit_guard_source(driver, &binding.value)?;
            let next = self.cx.context.append_basic_block(self.llvm_fn, "ref.next");
            self.cx
                .builder
                .build_conditional_branch(is_absent, cleanup, next)?;
            self.cx.builder.position_at_end(next);
            self.bind_observation(binding, observation, object, control)?;
        }
        let present = self.cx.builder.get_insert_block().expect("present path");
        self.cx.builder.position_at_end(cleanup);
        self.regions.sink_mut().unwind_to_depth(outer_depth)?;
        self.cx.builder.build_unconditional_branch(absent)?;
        self.cx.builder.position_at_end(present);
        Ok(())
    }

    fn emit_guard_source<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        value: &HirExpr,
    ) -> Result<
        (
            StructValue<'ctx>,
            PointerValue<'ctx>,
            PointerValue<'ctx>,
            IntValue<'ctx>,
        ),
        Diagnostic,
    > {
        let stack_depth = self.walk.eval_stack.len();
        driver
            .walk_expr(self, value)
            .map_err(region_walk_codegen_error)?;
        let reference = self
            .walk
            .trailing
            .take()
            .ok_or_else(|| not_yet_supported("managed Ref match value", value.span))?;
        self.walk.eval_stack.truncate(stack_depth);
        let (observation, object, is_absent) = self.observe_ref(reference)?;
        self.drop_ref_value(reference)?;
        let control = self
            .cx
            .builder
            .build_extract_value(reference.into_struct_value(), 0, "ref.control")?
            .into_pointer_value();
        Ok((observation, object, control, is_absent))
    }

    fn bind_observation(
        &mut self,
        binding: &crate::hir::HirConditionalBinding,
        observation: StructValue<'ctx>,
        object: PointerValue<'ctx>,
        control: PointerValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let observation_slot = self.entry_alloca(self.cx.ref_observation_type(), "ref.guard")?;
        self.cx.builder.build_store(observation_slot, observation)?;
        let guard_owner = self.current_arena();
        self.cx.builder.build_call(
            self.cx.arena_register_observation_drop_fn(),
            &[guard_owner.into(), observation_slot.into()],
            "",
        )?;
        let object_handle = self.cx.object_handle_type().const_named_struct(&[]);
        let object_handle =
            self.cx
                .builder
                .build_insert_value(object_handle, control, 0, "borrow.arena")?;
        let object_handle = self
            .cx
            .builder
            .build_insert_value(object_handle, object, 1, "borrow.ptr")?
            .into_struct_value();
        self.declare_local(
            &binding.name.name,
            &binding.binding_ty,
            object_handle.into(),
        )?;
        Ok(())
    }
}

impl<'s, 'report, 'a, 'ctx> RegionVisitor for FnEmitter<'s, 'report, 'a, 'ctx> {
    fn bind_presence_guard(&mut self, _binding: &HirConditionalBinding) -> Result<(), WalkError> {
        unreachable!("codegen overrides presence_match and binds guards itself")
    }

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
            let hidden = super::call_abi::CallAbi::of(emitter.function).hidden_len();
            for (param, value) in emitter
                .function
                .params
                .iter()
                .zip(emitter.llvm_fn.get_param_iter().skip(hidden))
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

    fn before_expr<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
    ) -> Result<(), WalkError> {
        if matches!(expr.kind, crate::hir::HirExprKind::ObjectConstruct { .. }) {
            self.pin_walk_driver(driver, |emitter| {
                emitter.walk_step(|e| e.begin_object_construct(expr))
            })?;
        }
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
        if matches!(
            expr.kind,
            crate::hir::HirExprKind::PresenceMatch { .. } | crate::hir::HirExprKind::If { .. }
        ) {
            return Ok(());
        }
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
        self.ensure_open_block();
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

    // Overrides the canonical sequence: each tail source needs a branch to the cleanup
    // block before its name is bound, so `bind_presence_guard` is not used here.
    fn presence_match<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        bindings: &HirConditionalBindings,
        some_block: &HirBlock,
        none_block: Option<&HirBlock>,
        result_ty: &Ty,
    ) -> Result<(), WalkError> {
        map_emit_err(
            self.emit_presence_with_driver(driver, bindings, some_block, none_block, result_ty),
        )
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
