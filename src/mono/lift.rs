//! Closure lifting (RFC 0007): after specialisation every closure body becomes an ordinary
//! function `outer$closureN(captures..., params...)`, and its place in the caller is a
//! `MakeClosure` value. MIR and codegen then see
//! only plain functions, and the 1:1 function/report-root invariant holds.

use std::collections::HashMap;

use crate::diag::Diagnostic;
use crate::hir::{self, HirExpr, HirExprKind, HirFunction, HirParam, HirStmt, UseKind};
use crate::sema::{ArenaNode, ArenaReport};
use crate::span::AstId;
use crate::ty::TyCtx;

/// Lifts every closure of `functions`, appending the new functions and their report roots.
pub(super) fn lift_closures(functions: &mut Vec<HirFunction>, report: &mut ArenaReport, tcx: &TyCtx) -> Result<(), Vec<Diagnostic>> {
    // Lifted closures are appended and already hold no closures, so only the original functions are scanned.
    for at in 0..functions.len() {
        let Some(mut nodes) = take_closure_nodes(&mut report.roots[at]) else {
            return Err(vec![super::error(format!("internal error: `{}` has two closure regions with the same id", functions[at].name), None)]);
        };
        let mut body = std::mem::take(&mut functions[at].body);
        let (outer_name, outer_span, outer_source) = (functions[at].name.clone(), functions[at].name_span, functions[at].source);
        let mut lifted: Vec<(HirFunction, ArenaNode)> = Vec::new();
        let mut missing = false;
        hir::rewrite(&mut body, &mut |t| t, &mut |e| match &mut e.kind {
            HirExprKind::Closure { id, .. } => {
                let Some(mut node) = nodes.remove(id) else {
                    missing = true;
                    return;
                };
                let name = format!("{}$closure{}", outer_name, lifted.len());
                node.label = format!("fun {name}");
                let (function, make) = lift_one(outer_span, outer_source, name, std::mem::replace(e, HirExpr::new(HirExprKind::None, e.ty)), tcx);
                *e = make;
                lifted.push((function, node));
            }
            _ => {}
        });
        functions[at].body = body;
        if missing || !nodes.is_empty() {
            let message = format!("internal error: `{}` has closures that its report regions do not match", outer_name);
            return Err(vec![super::error(message, None)]);
        }
        for (function, node) in lifted {
            functions.push(function);
            report.roots.push(node);
        }
    }
    Ok(())
}

/// Removes the closure regions below `root`, keyed by the closure they belong to. `None` when two share an id.
fn take_closure_nodes(root: &mut ArenaNode) -> Option<HashMap<AstId, ArenaNode>> {
    let mut out = HashMap::new();
    for mut child in std::mem::take(&mut root.children) {
        for (id, node) in take_closure_nodes(&mut child)? {
            out.insert(id, node).is_none().then_some(())?;
        }
        match child.closure {
            Some(id) => out.insert(id, child).is_none().then_some(())?,
            None => root.children.push(child),
        }
    }
    Some(out)
}

fn lift_one(outer_span: crate::span::Span, outer_source: Option<&'static str>, name: String, closure: HirExpr, tcx: &TyCtx) -> (HirFunction, HirExpr) {
    let ty = closure.ty;
    let span = closure.span;
    let HirExprKind::Closure { params, body, captures, is_move, kind, .. } = closure.kind else { unreachable!("lift_one gets a closure") };
    let (_, ret) = tcx.func_sig(ty).expect("a closure has a signature");
    // A closure's value is its last expression; a function must `return` it.
    let mut body = body;
    if ret != tcx.prim_id(crate::hir::Prim::Unit) {
        if let Some(HirStmt::Expr(_)) = body.stmts.last() {
            let Some(HirStmt::Expr(value)) = body.stmts.pop() else { unreachable!() };
            body.stmts.push(HirStmt::Return { value: Some(value) });
            body.statement_spans.truncate(body.stmts.len());
        }
    }
    let capture_params = captures.iter().map(|c| HirParam { kind: crate::ast::BindingKind::Val, name: c.name.clone(), ty: c.ty });
    let function = HirFunction {
        name: name.clone(),
        generics: Vec::new(),
        name_span: span.unwrap_or(outer_span),
        source: outer_source,
        env_params: Some(captures.len()),
        env_mode: match (kind, is_move) {
            (crate::hir::ClosureKind::FnMut, false) => crate::hir::CaptureMode::ByAddress,
            // `FnOnce` too: a capture moved out nulls the environment's slot, so its release does not run twice.
            (crate::hir::ClosureKind::FnMut, true) | (crate::hir::ClosureKind::FnOnce, _) => crate::hir::CaptureMode::InPlace,
            _ => crate::hir::CaptureMode::ByValue,
        },
        params: capture_params.chain(params).collect(),
        return_ty: ret,
        body,
    };
    let use_kind = |ty| if is_move && tcx.is_move_only(ty) { UseKind::Move } else { UseKind::Local };
    let values = captures
        .iter()
        .map(|c| HirExpr::new(HirExprKind::Ident { name: c.name.clone(), use_kind: use_kind(c.ty) }, c.ty))
        .collect();
    let mut make = HirExpr::new(HirExprKind::MakeClosure { function: name, captures: values, kind, is_move }, ty);
    make.span = span;
    (function, make)
}
