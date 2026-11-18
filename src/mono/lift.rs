//! Closure lifting (RFC 0007): after specialisation every closure body becomes an ordinary
//! function `outer$closureN(captures..., params...)`, and its place in the caller is a
//! `MakeClosure` value. A trailing closure becomes the call's last argument. MIR and codegen then
//! see only plain functions, and the 1:1 function/report-root invariant holds.
//!
//! shortcut: a capture is passed by value, so a closure that assigns to a plain capture changes its
//! own copy; MIR lowering must pass such captures by `&rw` (RFC 0007 step B4) before it accepts them.

use std::collections::VecDeque;

use crate::diag::Diagnostic;
use crate::hir::{self, HirExpr, HirExprKind, HirFunction, HirParam, UseKind};
use crate::sema::{ArenaNode, ArenaReport};
use crate::ty::TyCtx;

/// Lifts every closure of `functions`, appending the new functions and their report roots.
pub(super) fn lift_closures(functions: &mut Vec<HirFunction>, report: &mut ArenaReport, tcx: &TyCtx) -> Result<(), Vec<Diagnostic>> {
    let mut at = 0;
    while at < functions.len() {
        let mut nodes: VecDeque<ArenaNode> = take_closure_nodes(&mut report.roots[at]).into();
        let mut body = std::mem::take(&mut functions[at].body);
        let outer = functions[at].clone();
        let mut lifted: Vec<(HirFunction, ArenaNode)> = Vec::new();
        let mut missing = false;
        hir::rewrite(&mut body, &mut |t| t, &mut |e| match &mut e.kind {
            HirExprKind::Closure { .. } => {
                let Some(mut node) = nodes.pop_front() else {
                    missing = true;
                    return;
                };
                let name = format!("{}$closure{}", outer.name, lifted.len());
                node.label = format!("fun {name}");
                let (function, make) = lift_one(&outer, name, std::mem::replace(e, HirExpr::new(HirExprKind::None, e.ty)), tcx);
                *e = make;
                lifted.push((function, node));
            }
            HirExprKind::Call { args, trailing, .. } => args.extend(trailing.take().map(|t| *t)),
            _ => {}
        });
        functions[at].body = body;
        if missing || !nodes.is_empty() {
            let message = format!("internal error: `{}` has closures that its report regions do not match", outer.name);
            return Err(vec![super::error(message, None)]);
        }
        for (function, node) in lifted {
            functions.push(function);
            report.roots.push(node);
        }
        at += 1;
    }
    Ok(())
}

/// Removes the report nodes labelled `Closure` below `root`, children before their parent, so the
/// order matches the post-order the HIR rewrite visits closures in.
fn take_closure_nodes(root: &mut ArenaNode) -> Vec<ArenaNode> {
    let mut out = Vec::new();
    for child in std::mem::take(&mut root.children) {
        let mut child = child;
        out.extend(take_closure_nodes(&mut child));
        if child.label.starts_with("Closure") {
            out.push(child);
        } else {
            root.children.push(child);
        }
    }
    out
}

fn lift_one(outer: &HirFunction, name: String, closure: HirExpr, tcx: &TyCtx) -> (HirFunction, HirExpr) {
    let ty = closure.ty;
    let span = closure.span;
    let HirExprKind::Closure { params, body, captures, is_move, .. } = closure.kind else { unreachable!("lift_one gets a closure") };
    let (_, ret) = tcx.func_sig(ty).expect("a closure has a signature");
    let capture_params = captures.iter().map(|c| HirParam { kind: crate::ast::BindingKind::Val, name: c.name.clone(), ty: c.ty });
    let function = HirFunction {
        name: name.clone(),
        generics: Vec::new(),
        name_span: span.unwrap_or(outer.name_span),
        source: outer.source,
        params: capture_params.chain(params).collect(),
        return_ty: ret,
        body,
    };
    let use_kind = |ty| if is_move && tcx.is_move_only(ty) { UseKind::Move } else { UseKind::Local };
    let values = captures
        .iter()
        .map(|c| HirExpr::new(HirExprKind::Ident { name: c.name.clone(), use_kind: use_kind(c.ty) }, c.ty))
        .collect();
    let mut make = HirExpr::new(HirExprKind::MakeClosure { function: name, captures: values }, ty);
    make.span = span;
    (function, make)
}
