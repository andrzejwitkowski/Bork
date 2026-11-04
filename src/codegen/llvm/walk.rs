//! Region-walk driver for LLVM emission.
//!
//! `CodegenWalker` is the `RegionVisitor` for one function body; `FnEmitter` itself is not a
//! visitor. Emission re-enters the walk through the `FnEmitter` methods defined here
//! (`walk_expr`, `walk_block`, `region_enter*`, `region_exit`), which build a fresh driver over
//! the shared cursor. Dependency direction: this module uses the emitter modules, and they
//! never name it (they only call those methods), so there is no `walk` <-> `expr` cycle.

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

use crate::codegen::regions::RegionSite;
use crate::diag::Diagnostic;
use crate::hir::{
    HirArm, HirBlock, HirConditionalBinding, HirConditionalBindings, HirExpr, HirFunction, HirStmt,
};
use crate::region_walk::{
    region_enter, region_enter_forced, region_exit, walk_function_readonly, ArenaCursor,
    RegionVisitor, SharedCursor, WalkDriver, WalkError,
};
use crate::sema::ArenaNode;
use crate::ty::TyId;

use super::emit_fn::{Callees, CodegenWalkState, FnEmitter, IfKind, Regions};
use super::schedule_error;
use super::context::Codegen;
use super::call_abi::CallAbi;

fn map_emit_err(result: Result<(), Diagnostic>) -> Result<(), WalkError> {
    result.map_err(WalkError::from_diagnostic)
}

/// The visitor handed to `region_walk`; derefs to the emitter so hooks read as emitter code.
pub(super) struct CodegenWalker<'w, 's, 'report, 'a, 'ctx>(
    &'w mut FnEmitter<'s, 'report, 'a, 'ctx>,
);

impl<'s, 'report, 'a, 'ctx> Deref for CodegenWalker<'_, 's, 'report, 'a, 'ctx> {
    type Target = FnEmitter<'s, 'report, 'a, 'ctx>;
    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl<'s, 'report, 'a, 'ctx> DerefMut for CodegenWalker<'_, 's, 'report, 'a, 'ctx> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0
    }
}

/// Re-entry into the walk from emission code.
impl<'s, 'report, 'a, 'ctx> FnEmitter<'s, 'report, 'a, 'ctx> {
    fn with_walk<R>(
        &mut self,
        f: impl FnOnce(&mut WalkDriver<'_, SharedCursor<'report>>, &mut CodegenWalker<'_, 's, 'report, 'a, 'ctx>) -> R,
    ) -> R {
        let mut cursor = self
            .walk
            .cursor
            .clone()
            .expect("re-entering the region walk requires an active walk");
        let cx = self.cx;
        let mut driver = WalkDriver {
            cursor: &mut cursor,
            tcx: &cx.tcx,
        };
        f(&mut driver, &mut CodegenWalker(self))
    }

    pub(super) fn walk_expr(&mut self, expr: &HirExpr) -> Result<(), WalkError> {
        self.with_walk(|driver, walker| driver.walk_expr(walker, expr))
    }

    pub(super) fn walk_block(
        &mut self,
        block: &HirBlock,
        value_ty: Option<TyId>,
    ) -> Result<(), WalkError> {
        self.with_walk(|driver, walker| driver.walk_block(walker, block, value_ty))
    }

    pub(super) fn region_enter(&mut self, site: RegionSite, block: &HirBlock) -> Result<(), WalkError> {
        self.with_walk(|driver, walker| region_enter(driver, walker, site, block))
    }

    pub(super) fn region_enter_forced(
        &mut self,
        site: RegionSite,
        block: &HirBlock,
        force_push: bool,
    ) -> Result<(), WalkError> {
        self.with_walk(|driver, walker| region_enter_forced(driver, walker, site, block, force_push))
    }

    pub(super) fn region_exit(&mut self, site: RegionSite) -> Result<(), WalkError> {
        self.with_walk(|driver, walker| region_exit(driver, walker, site))
    }

    fn walk_step(
        &mut self,
        step: impl FnOnce(&mut Self) -> Result<(), Diagnostic>,
    ) -> Result<(), WalkError> {
        step(self).map_err(WalkError::from_diagnostic)
    }
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
    cx.position_at_end(entry);

    let mut emitter = FnEmitter {
        cx,
        callees,
        regions,
        function,
        llvm_fn,
        scopes: vec![HashMap::new()],
        debug_scopes: Vec::new(),
        statement_span: None,
        alloc_sink: None,
        function_arena: None,
        loop_stack: Vec::new(),
        walk: CodegenWalkState::new(),
        object_constructions: Vec::new(),
        result_sink: None,
    };
    cx.builder.unset_current_debug_location();
    if let Some(debug) = &cx.debug {
        let scope = debug.function(cx, function, llvm_fn);
        emitter.debug_enter_scope(Some(scope))?;
    }
    let abi = CallAbi::of(cx, function);
    if abi.hidden_len() > 0 {
        let parent = llvm_fn
            .get_nth_param(0)
            .expect("callee has a lexical parent parameter")
            .into_pointer_value();
        emitter.regions.sink_mut().set_caller_parent(parent);
        if abi.has_result_sink() {
            emitter.result_sink = Some(
                llvm_fn
                    .get_nth_param(1)
                    .expect("owning return has a result sink parameter")
                    .into_pointer_value(),
            );
        }
    }
    let emitted = emit_function_body(&mut emitter, roots, function);
    emitter.regions.sink_mut().clear_caller_parent();
    emitted?;
    if !cx.current_block_terminated() {
        emitter.build_fallthrough()?;
    }
    emitter.debug_location(None);
    Ok(())
}

impl<'w, 's, 'report, 'a, 'ctx> RegionVisitor for CodegenWalker<'w, 's, 'report, 'a, 'ctx> {
    fn before_stmt(
        &mut self,
        _stmt: &HirStmt,
        span: Option<crate::span::Span>,
    ) -> Result<(), WalkError> {
        self.statement_span = span;
        self.debug_location(span);
        Ok(())
    }

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
            let hidden = super::call_abi::CallAbi::of(emitter.cx, emitter.function).hidden_len();
            for (index, (param, value)) in emitter
                .function
                .params
                .iter()
                .zip(emitter.llvm_fn.get_param_iter().skip(hidden))
                .enumerate()
            {
                emitter.declare_local(&param.name, param.ty, value, Some(index as u32 + 1))?;
            }
            emitter.debug_location(Some(emitter.function.name_span));
            Ok(())
        })
    }

    fn end_function_body(&mut self) -> Result<(), WalkError> {
        self.walk_step(|emitter| {
            emitter.debug_exit_scope()?;
            emitter.pop_region()
        })?;
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
            emitter.debug_location(body.span);
            emitter
                .regions
                .open_known(child, body, Some(site))
                .map_err(schedule_error)?;
            emitter.check_arena()?;
            emitter.scopes.push(HashMap::new());
            emitter.debug_enter_scope(None)?;
            Ok(())
        })
    }

    fn loop_latch(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        Ok(())
    }

    fn exit_region(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        self.walk_step(|emitter| emitter.debug_exit_scope())?;
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
        _driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
    ) -> Result<(), WalkError> {
        self.debug_location(expr.span);
        if matches!(
            expr.kind,
            crate::hir::HirExprKind::ObjectConstruct { .. } | crate::hir::HirExprKind::EnumConstruct { .. }
        ) {
            self.walk_step(|e| e.begin_object_construct(expr))?;
        }
        Ok(())
    }

    fn on_stmt<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        stmt: &HirStmt,
    ) -> Result<(), WalkError> {
        let span = match stmt {
            HirStmt::VarDecl { value, .. }
            | HirStmt::Assign { value, .. }
            | HirStmt::Return { value: Some(value) }
            | HirStmt::Expr(value) => value.span,
            HirStmt::Break { span } | HirStmt::Continue { span } => Some(*span),
            _ => None,
        };
        self.debug_location(self.statement_span.or(span));
        self.walk_step(|e| e.emit_stmt(stmt))
    }

    fn after_expr<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
    ) -> Result<(), WalkError> {
        self.debug_location(expr.span);
        if matches!(
            expr.kind,
            crate::hir::HirExprKind::PresenceMatch { .. }
                | crate::hir::HirExprKind::If { .. }
                | crate::hir::HirExprKind::Match { .. }
        ) {
            return Ok(());
        }
        self.walk_step(|e| {
            if let Some(value) = e.emit_after_walk(expr)? {
                e.walk.eval_stack.push(value);
                e.walk.trailing = Some(value);
            }
            Ok(())
        })
    }

    fn on_trailing_value<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
        ty: TyId,
    ) -> Result<(), WalkError> {
        self.ensure_open_block();
        driver.walk_expr(self, expr)?;
        if self.walk.trailing.is_some() {
            return Ok(());
        }
        self.walk_step(|e| {
            e.ensure_open_block();
            e.walk.trailing = Some(e.emit_value(expr, ty)?);
            Ok(())
        })
    }

    fn for_loop<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        name: &str,
        iter: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        let result = self.emit_for(name, iter, body);
        map_emit_err(result)
    }

    fn while_loop<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        cond: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        let result = self.emit_while(cond, body);
        map_emit_err(result)
    }

    fn if_expr<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        cond: &HirExpr,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        result_ty: TyId,
    ) -> Result<(), WalkError> {
        self.walk_step(|e| {
            let depth = e.walk.eval_stack.len();
            e.emit_if(cond, then_block, else_block, IfKind::from_ty(e.cx.unit_ty(), result_ty))?;
            e.walk.eval_stack.truncate(depth);
            if let Some(value) = e.walk.trailing {
                e.walk.eval_stack.push(value);
            }
            Ok(())
        })
    }

    fn match_expr<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        scrutinee: &HirExpr,
        arms: &[HirArm],
        result_ty: TyId,
    ) -> Result<(), WalkError> {
        map_emit_err(self.emit_match(scrutinee, arms, result_ty))
    }

    // Overrides the canonical sequence: each tail source needs a branch to the cleanup
    // block before its name is bound, so `bind_presence_guard` is not used here.
    fn presence_match<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        bindings: &HirConditionalBindings,
        some_block: &HirBlock,
        none_block: Option<&HirBlock>,
        result_ty: TyId,
    ) -> Result<(), WalkError> {
        map_emit_err(
            self.emit_presence(bindings, some_block, none_block, result_ty),
        )
    }
}

fn emit_function_body<'report>(
    emitter: &mut FnEmitter<'_, 'report, '_, '_>,
    roots: &'report [ArenaNode],
    function: &HirFunction,
) -> Result<(), Diagnostic> {
    let mut cursor = SharedCursor::new(roots, emitter.regions.function_index());
    emitter.walk.cursor = Some(cursor.clone());
    let walked = walk_function_readonly(function, &emitter.cx.tcx, &mut cursor, &mut CodegenWalker(emitter));
    emitter.walk.cursor = None;
    walked.map_err(super::resolve_walk_failure)
}
