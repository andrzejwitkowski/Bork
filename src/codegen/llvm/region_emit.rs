//! Control-flow emission (`if`, `if val`) that re-enters the region walk; the visitor is in `walk.rs`.

use std::iter;

use inkwell::basic_block::BasicBlock;
use inkwell::types::BasicType;
use inkwell::values::{BasicValue, BasicValueEnum, IntValue, PointerValue, StructValue};

use crate::codegen::regions::RegionSite;
use crate::diag::Diagnostic;
use crate::ty::TyId;
use crate::hir::{HirBlock, HirConditionalBinding, HirConditionalBindings, HirExpr};
use crate::region_walk::WalkError;

use super::emit_fn::{FnEmitter, IfKind};
use super::{not_yet_supported, region_walk_codegen_error};

impl<'s, 'report, 'a, 'ctx> FnEmitter<'s, 'report, 'a, 'ctx> {
    fn emit_region(
        &mut self,
        site: RegionSite,
        block: &HirBlock,
        value_ty: Option<TyId>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, WalkError> {
        let previous = self.alloc_sink;
        if value_ty.is_some_and(|ty| self.cx.tcx.owns_result(ty)) {
            self.alloc_sink = Some(self.sink_arena());
        }
        self.region_enter(site, block)?;
        let (peeled, _) = crate::hir::peel_blocks(block);
        self.walk.trailing = None;
        self.walk_block(peeled, value_ty)?;
        let value = self.walk.trailing.take();
        self.alloc_sink = previous;
        self.region_exit(site)?;
        Ok(value)
    }

    pub(super) fn emit_if(
        &mut self,
        cond: &HirExpr,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        kind: IfKind,
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
            self.cx.position_at_end(bb);
            let value = self
                .emit_region(site, block, value_ty)
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
        self.cx.position_at_end(merge_bb);

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

    pub(super) fn emit_presence(
        &mut self,
        bindings: &HirConditionalBindings,
        some_block: &HirBlock,
        none_block: Option<&HirBlock>,
        result_ty: TyId,
    ) -> Result<(), Diagnostic> {
        let stack_depth = self.walk.eval_stack.len();
        let value_ty = (result_ty != self.cx.unit_ty()).then_some(result_ty);
        let result_sink = self.sink_arena();
        let previous = self.alloc_sink;
        // The head source is evaluated in the enclosing arena, as with single-binding syntax.
        let (observation, object, control, is_absent) =
            self.emit_guard_source(&bindings.head.value)?;
        let mut split = self.open_presence("ref", is_absent, value_ty)?;
        let outer_depth = self.regions.sink_mut().handle_count();
        self.region_enter(RegionSite::PresenceSome, some_block)
            .map_err(region_walk_codegen_error)?;
        self.bind_observation(&bindings.head, observation, object, control)?;
        // Tail sources are evaluated inside the presence region (as in sema), so their
        // allocations use that region's arena; only the arm's values go to the result sink.
        self.emit_chained_guards(&bindings.tail, outer_depth, split.absent_target())?;
        if self.cx.tcx.owns_result(result_ty) {
            self.alloc_sink = Some(result_sink);
        }
        let branch_value = self.emit_presence_arm(
            RegionSite::PresenceSome,
            some_block,
            value_ty,
            result_ty,
            result_sink,
        )?;
        self.end_present_arm(&mut split, branch_value)?;
        let absent_value = match none_block {
            Some(block) => {
                self.region_enter(RegionSite::PresenceNone, block)
                    .map_err(region_walk_codegen_error)?;
                self.emit_presence_arm(
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
    pub(super) fn emit_presence_arm(
        &mut self,
        site: RegionSite,
        block: &HirBlock,
        value_ty: Option<TyId>,
        result_ty: TyId,
        result_sink: PointerValue<'ctx>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let (body, _) = crate::hir::peel_blocks(block);
        self.walk.trailing = None;
        self.walk_block(body, value_ty)
            .map_err(region_walk_codegen_error)?;
        let mut branch_value = self.walk.trailing.take();
        if !self.cx.current_block_terminated() {
            if let Some(value) = branch_value {
                branch_value = Some(self.materialize_owned_value(value, result_ty, result_sink)?);
            }
        }
        self.region_exit(site).map_err(region_walk_codegen_error)?;
        Ok(branch_value)
    }

    /// Tail sources of an `if val` header. An absent source jumps to a shared cleanup block,
    /// which unwinds arenas opened since the header and then takes the `else` path. Leaves the
    /// insertion point on the all-present path.
    fn emit_chained_guards(
        &mut self,
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
                self.emit_guard_source(&binding.value)?;
            let next = self.cx.context.append_basic_block(self.llvm_fn, "ref.next");
            self.cx
                .builder
                .build_conditional_branch(is_absent, cleanup, next)?;
            self.cx.position_at_end(next);
            self.bind_observation(binding, observation, object, control)?;
        }
        let present = self.cx.builder.get_insert_block().expect("present path");
        self.cx.position_at_end(cleanup);
        self.debug_unwind(self.debug_scopes.len().saturating_sub(1))?;
        self.regions.sink_mut().unwind_to_depth(outer_depth)?;
        self.cx.builder.build_unconditional_branch(absent)?;
        self.cx.position_at_end(present);
        Ok(())
    }

    fn emit_guard_source(
        &mut self,
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
        self.walk_expr(value).map_err(region_walk_codegen_error)?;
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
            binding.binding_ty,
            object_handle.into(),
            None,
        )?;
        Ok(())
    }
}
