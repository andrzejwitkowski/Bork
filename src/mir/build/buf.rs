//! `Buf<T>` primitives (#285): each becomes an `Intrinsic`, after explicit guards that abort on a bad index or size.

use super::expr::span_of;
use super::{BuildError, FnBuilder, Res};
use crate::ast::{BinOp, UnaryOp};
use crate::hir::{HirExpr, HirExprKind};
use crate::mir::body::*;

impl<'a> FnBuilder<'a> {
    pub(super) fn buf_primitive(&mut self, e: &HirExpr, name: &str, args: &[HirExpr]) -> Res<Operand> {
        let span = span_of(e);
        let ops = args.iter().map(|a| self.buf_arg(a)).collect::<Res<Vec<_>>>()?;
        let zero = Operand::Const(Const::Int(0));
        let (op, arena) = match (name, ops.as_slice()) {
            ("abort", []) => {
                self.terminate(TermKind::Abort(AbortReason::Explicit), span, Some(e.id));
                return Ok(Operand::Const(Const::Unit));
            }
            // typeck allows only `arenaOf(this)`: the arena the object handle names.
            ("arenaOf", [Operand::Copy(p) | Operand::Move(p)]) if p.is_bare() => {
                let dst = self.temp(e.ty);
                let arena = Some(ArenaRef::ObjectArena(p.local));
                self.stmt(StmtKind::Intrinsic { dst: Some(Place::local(dst)), op: IntrinsicOp::ArenaOf, args: Vec::new(), arena }, span, Some(e.id));
                return Ok(Operand::Copy(Place::local(dst)));
            }
            ("Buf.cap", [_]) => (IntrinsicOp::BufCap, None),
            ("bufAlloc", [_, cap]) => {
                self.abort_unless(&[(BinOp::Ge, cap.clone(), zero)], AbortReason::NegativeCapacity, span, e);
                (IntrinsicOp::BufAlloc, None)
            }
            ("slotLoad", [buf, i, ..]) | ("slotStore", [buf, i, _]) => {
                let cap = self.buf_cap(buf, e);
                self.abort_unless(&[(BinOp::Ge, i.clone(), zero), (BinOp::Lt, i.clone(), cap)], AbortReason::Bounds, span, e);
                (if name == "slotLoad" { IntrinsicOp::SlotLoad } else { IntrinsicOp::SlotStore }, None)
            }
            ("bufGrow", [buf, new_cap]) => {
                let cap = self.buf_cap(buf, e);
                self.abort_unless(&[(BinOp::Gt, new_cap.clone(), cap)], AbortReason::BufGrow, span, e);
                // shortcut: a `Buf` carries no arena, so the copy goes to `this`'s arena (where a prelude class keeps its
                // buffer field); track the buffer's own arena if a `Buf` ever lives elsewhere.
                (IntrinsicOp::BufGrow, Some(self.this_arena(span)?))
            }
            _ => return Err(BuildError::Internal(format!("`{name}` with {} arguments", ops.len()))),
        };
        let dst = (!self.is_unit(e.ty)).then(|| self.temp(e.ty));
        self.stmt(StmtKind::Intrinsic { dst: dst.map(Place::local), op, args: ops, arena }, span, Some(e.id));
        Ok(dst.map_or(Operand::Const(Const::Unit), |d| Operand::Copy(Place::local(d))))
    }

    /// An argument as a bare operand; `&b` reads the `{ptr, cap}` handle itself (slots live behind `ptr`).
    fn buf_arg(&mut self, a: &HirExpr) -> Res<Operand> {
        match &a.kind {
            HirExprKind::Unary { op: UnaryOp::Borrow { .. }, expr } => self.bare_operand(expr),
            _ => self.bare_operand(a),
        }
    }

    fn buf_cap(&mut self, buf: &Operand, e: &HirExpr) -> Operand {
        let cap = self.temp(self.tcx.prim_id(crate::hir::Prim::I32));
        self.stmt(StmtKind::Intrinsic { dst: Some(Place::local(cap)), op: IntrinsicOp::BufCap, args: vec![buf.clone()], arena: None }, span_of(e), Some(e.id));
        Operand::Copy(Place::local(cap))
    }

    // shortcut: bufGrow allocates in the arena of `this`; the buffer must be a field of `this` (not checked), tighten when Vec moves to Bork (#285 step 2)
    fn this_arena(&self, span: crate::span::Span) -> Res<ArenaRef> {
        let this = self.lookup("this", span).map_err(|_| BuildError::unlowered("a `Buf` primitive outside a method", span))?;
        Ok(ArenaRef::ObjectArena(this))
    }
}
