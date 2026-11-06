//! Expression lowering: every expression yields an [`Operand`], evaluated left to right.

use super::{BuildError, FnBuilder, Res};
use crate::ast::{BinOp, UnaryOp};
use crate::builtins;
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirStmt, UseKind};
use crate::mir::body::*;
use crate::mir::capability::Capability;
use crate::region_walk::RegionSite;
use crate::span::Span;

pub(super) fn span_of(e: &HirExpr) -> Span {
    e.span.unwrap_or_default()
}

impl<'a> FnBuilder<'a> {
    /// Evaluates `e`. An owned `Ref` temporary comes back as `Move` so its single consumer is checkable.
    pub(super) fn expr(&mut self, e: &HirExpr) -> Res<Operand> {
        let op = self.expr_inner(e)?;
        match op {
            Operand::Copy(p) if p.is_bare() && self.tcx.is_managed_ref(e.ty) && self.locals[p.local.0 as usize].kind == LocalKind::Temp => {
                Ok(Operand::Move(p))
            }
            other => Ok(other),
        }
    }

    fn expr_inner(&mut self, e: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        match &e.kind {
            HirExprKind::Int { value } => Ok(Operand::Const(Const::Int(*value))),
            HirExprKind::Float { value } => Ok(Operand::Const(Const::Float(*value))),
            HirExprKind::Bool { value } => Ok(Operand::Const(Const::Bool(*value))),
            HirExprKind::Str { value } => Ok(Operand::Const(Const::Str(value.clone()))),
            HirExprKind::Ident { name, use_kind } => self.ident(e, name, *use_kind),
            HirExprKind::Binary { op, lhs, rhs } => self.binary(e, op, lhs, rhs),
            HirExprKind::Unary { op, expr } => self.unary(e, op, expr),
            HirExprKind::Call { callee, args, has_trailing_closure } => self.call(e, callee, args, *has_trailing_closure),
            HirExprKind::If { cond, then_block, else_block } => self.if_expr(e, cond, then_block, else_block.as_ref()),
            HirExprKind::Index { receiver, index, .. } => self.index(e, receiver, index),
            HirExprKind::Slice { receiver, lo, hi } => self.slice(e, receiver, lo, hi),
            HirExprKind::Field { receiver, name, safe } => self.field(e, receiver, name, *safe),
            HirExprKind::VecNew { cap, .. } => self.vec_new(e, cap.as_deref()),
            HirExprKind::VecOp { op, receiver, args } => self.vec_op_expr(e, *op, receiver, args),
            HirExprKind::ArrayLit { elements } => self.array_lit(e, elements),
            HirExprKind::ObjectConstruct { class_name, fields } => self.object_construct(e, class_name, fields),
            HirExprKind::EnumConstruct { enum_name, variant, fields } => self.enum_construct(e, enum_name, variant, fields),
            HirExprKind::ObjectField { receiver, field_index, safe, .. } => self.object_field(e, receiver, *field_index, *safe),
            HirExprKind::None => self.make_none(e.ty, span),
            HirExprKind::Some(inner) => {
                let v = self.expr(inner)?;
                self.make_some(e.ty, v, inner.ty, span)
            }
            HirExprKind::Match { scrutinee, arms } => self.match_expr(e, scrutinee, arms),
            HirExprKind::RefCreate(inner) => self.ref_create(e, inner),
            HirExprKind::PresenceMatch { bindings, some_block, none_block } => self.presence(e, bindings, some_block, none_block.as_ref()),
        }
    }

    pub(super) fn assign_temp(&mut self, e: &HirExpr, rvalue: Rvalue) -> Operand {
        let temp = self.temp(e.ty);
        self.stmt(StmtKind::Assign(Place::local(temp), rvalue), span_of(e), Some(e.id));
        Operand::Copy(Place::local(temp))
    }

    fn ident(&mut self, e: &HirExpr, name: &str, use_kind: UseKind) -> Res<Operand> {
        let local = self.lookup(name, span_of(e))?;
        let place = Place::local(local);
        match use_kind {
            UseKind::Local | UseKind::Shared | UseKind::Copy => Ok(Operand::Copy(place)),
            UseKind::Borrow => Ok(Operand::Reborrow(place)),
            UseKind::Move | UseKind::Promote | UseKind::RefCopy | UseKind::RefMove => self.ident_moved(e, place, use_kind),
        }
    }

    fn binary(&mut self, e: &HirExpr, op: &BinOp, lhs: &HirExpr, rhs: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        match op {
            BinOp::And | BinOp::Or => return self.logical(e, matches!(op, BinOp::And), lhs, rhs),
            BinOp::Elvis => return self.elvis(e, lhs, rhs),
            BinOp::RangeTo => return Err(BuildError::unlowered("a range outside `for`", span)),
            _ => {}
        }
        let special = |t| self.tcx.is_managed_ref(t) || self.tcx.is_text(t) || self.tcx.is_nullable(t);
        if matches!(op, BinOp::Eq | BinOp::Ne)
            && (special(lhs.ty) || special(rhs.ty) || matches!(lhs.kind, HirExprKind::None) || matches!(rhs.kind, HirExprKind::None))
        {
            return self.equality(e, matches!(op, BinOp::Ne), lhs, rhs);
        }
        let l = self.expr(lhs)?;
        let r = self.expr(rhs)?;
        if matches!(op, BinOp::Div) && self.tcx.is_integer(e.ty) {
            // shortcut: only division by zero aborts; the signed `MIN / -1` guard is tracked in #237 and must land before #222.
            let zero = self.assign_temp_of(self.bool_ty, Rvalue::Binary(BinOp::Eq, r.clone(), Operand::Const(Const::Int(0))), span);
            let abort = self.new_block();
            let cont = self.new_block();
            self.terminate(TermKind::Switch { value: zero, cases: vec![(1, abort)], otherwise: cont }, span, Some(e.id));
            self.cur = abort;
            self.terminate(TermKind::Abort(AbortReason::Binop), span, Some(e.id));
            self.cur = cont;
        }
        Ok(self.assign_temp(e, Rvalue::Binary(op.clone(), l, r)))
    }

    fn assign_temp_of(&mut self, ty: crate::ty::TyId, rvalue: Rvalue, span: Span) -> Operand {
        let temp = self.temp(ty);
        self.stmt(StmtKind::Assign(Place::local(temp), rvalue), span, None);
        Operand::Copy(Place::local(temp))
    }

    /// `a && b` / `a || b`: the right side only runs when the left does not decide the result.
    fn logical(&mut self, e: &HirExpr, is_and: bool, lhs: &HirExpr, rhs: &HirExpr) -> Res<Operand> {
        let span = span_of(e);
        let result = self.temp(self.bool_ty);
        let l = self.expr(lhs)?;
        let (eval_rhs, short, end) = (self.new_block(), self.new_block(), self.new_block());
        // `&&` continues on true, `||` on false.
        let (cases, otherwise) = if is_and { (vec![(1, eval_rhs)], short) } else { (vec![(0, eval_rhs)], short) };
        self.terminate(TermKind::Switch { value: l, cases, otherwise }, span, Some(e.id));
        self.cur = eval_rhs;
        let r = self.expr(rhs)?;
        self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(r)), span, Some(e.id));
        self.goto(end, span);
        self.cur = short;
        self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(Operand::Const(Const::Bool(!is_and)))), span, Some(e.id));
        self.goto(end, span);
        self.cur = end;
        Ok(Operand::Copy(Place::local(result)))
    }

    fn unary(&mut self, e: &HirExpr, op: &UnaryOp, inner: &HirExpr) -> Res<Operand> {
        match op {
            UnaryOp::Not => {
                let v = self.expr(inner)?;
                Ok(self.assign_temp(e, Rvalue::Unary(MirUnOp::Not, v)))
            }
            UnaryOp::NotNullAssert => self.not_null_assert(e, inner),
            UnaryOp::Borrow { rw } => {
                let src = self.place_of(inner)?;
                let dst = self.temp(e.ty);
                self.stmt(StmtKind::Borrow { dst: Place::local(dst), src, rw: *rw }, span_of(e), Some(e.id));
                Ok(Operand::Copy(Place::local(dst)))
            }
        }
    }

    fn call(&mut self, e: &HirExpr, callee: &HirExpr, args: &[HirExpr], trailing_closure: bool) -> Res<Operand> {
        let span = span_of(e);
        if trailing_closure {
            return Err(BuildError::unsupported(Capability::Closure, span));
        }
        let HirExprKind::Ident { name, .. } = &callee.kind else {
            return Err(BuildError::unsupported(Capability::IndirectCall, span));
        };
        // Arguments are gathered without the assignment's sink (only `print` keeps it), so nested
        // temporaries do not land in the destination's arena (`call_gathers_without_sink`).
        let arg_sink = if builtins::is_print(name) { self.sink.clone() } else { ArenaRef::Current };
        let ops = self.with_sink(arg_sink, |b| b.exprs(args))?;
        if let Some(abi) = self.abi_of(name) {
            let mut hidden = Vec::new();
            if abi.caller_parent {
                hidden.push(ArenaRef::Current);
            }
            if abi.result_sink {
                hidden.push(if self.abi.result_sink { ArenaRef::ResultSink } else { self.sink.clone() });
            }
            let dest = self.temp(e.ty);
            let target = self.new_block();
            self.terminate(
                TermKind::Call { callee: name.clone(), args: ops, hidden, dest: Place::local(dest), target: Some(target), ret_borrows: Vec::new() },
                span,
                Some(e.id),
            );
            self.cur = target;
            return Ok(if self.is_unit(e.ty) { Operand::Const(Const::Unit) } else { Operand::Copy(Place::local(dest)) });
        }
        if builtins::is_print(name) {
            let newline = name == "println";
            let text = args.first().is_some_and(|a| self.tcx.is_text(a.ty));
            let op = match (newline, text) {
                (false, false) => IntrinsicOp::PrintInt,
                (true, false) => IntrinsicOp::PrintlnInt,
                (false, true) => IntrinsicOp::PrintStr,
                (true, true) => IntrinsicOp::PrintlnStr,
            };
            self.stmt(StmtKind::Intrinsic { dst: None, op, args: ops, arena: None }, span, Some(e.id));
            return Ok(Operand::Const(Const::Unit));
        }
        if builtins::is_ref_count(name) {
            return Ok(self.ref_count(e, name == "weakCount", &ops));
        }
        if builtins::is_string_alloc(name) {
            let op = if builtins::is_concat(name) { IntrinsicOp::Concat } else { IntrinsicOp::StringFromUtf8 };
            let dst = self.temp(e.ty);
            let arena = Some(self.sink.clone());
            self.stmt(StmtKind::Intrinsic { dst: Some(Place::local(dst)), op, args: ops, arena }, span, Some(e.id));
            return Ok(Operand::Copy(Place::local(dst)));
        }
        // Not a user function and not a builtin: a call through a value.
        Err(BuildError::unsupported(Capability::IndirectCall, span))
    }

    fn if_expr(&mut self, e: &HirExpr, cond: &HirExpr, then_block: &HirBlock, else_block: Option<&HirBlock>) -> Res<Operand> {
        let span = span_of(e);
        let c = self.expr(cond)?;
        let result = (!self.is_unit(e.ty)).then(|| self.temp(e.ty));
        let then_b = self.new_block();
        let else_b = else_block.map(|_| self.new_block());
        let merge = self.new_block();
        self.terminate(TermKind::Switch { value: c, cases: vec![(1, then_b)], otherwise: else_b.unwrap_or(merge) }, span, Some(e.id));
        self.cur = then_b;
        self.branch(RegionSite::IfThen, then_block, result, merge, span)?;
        if let (Some(b), Some(block)) = (else_b, else_block) {
            self.cur = b;
            self.branch(RegionSite::IfElse, block, result, merge, span)?;
        }
        self.cur = merge;
        Ok(result.map_or(Operand::Const(Const::Unit), |r| Operand::Copy(Place::local(r))))
    }

    /// One arm of an `if`: a region whose trailing value (if any) goes to `result`.
    fn branch(&mut self, site: RegionSite, block: &HirBlock, result: Option<Local>, merge: BlockId, span: Span) -> Res<()> {
        let sink = result.map(|r| self.locals[r.0 as usize].ty).filter(|&ty| self.owning(ty)).map(|_| self.outer_sink());
        let (peeled, _, _) = self.enter_region(site, block, false, None, span)?;
        match sink {
            Some(s) => self.with_sink(s, |b| b.block_value(peeled, result))?,
            None => self.block_value(peeled, result)?,
        }
        self.exit_region(span)?;
        self.goto(merge, span);
        Ok(())
    }

    /// Lowers `block`; with `result`, the last expression statement is its value.
    pub(super) fn block_value(&mut self, block: &HirBlock, result: Option<Local>) -> Res<()> {
        let Some(result) = result else { return self.stmts(block) };
        let Some((last, prefix)) = block.stmts.split_last() else { return Ok(()) };
        for (i, s) in prefix.iter().enumerate() {
            self.stmt_of(s, block.statement_spans.get(i).copied().unwrap_or_default())?;
        }
        match last {
            HirStmt::Expr(value) => {
                let v = self.expr(value)?;
                self.stmt(StmtKind::Assign(Place::local(result), Rvalue::Use(v)), span_of(value), Some(value.id));
                Ok(())
            }
            other => self.stmt_of(other, block.statement_spans.get(prefix.len()).copied().unwrap_or_default()),
        }
    }
}
