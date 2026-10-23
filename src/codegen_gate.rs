//! Codegen subset checks shared by `bork build` and the LSP (no LLVM dependency).

use crate::ast::{BinOp, UnaryOp};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirProgram, HirStmt, Prim, UseKind,
};
use crate::span::Span;
use crate::ty::{TyCtx, TyData, TyId};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NullableRepr {
    Buffer,
    TaggedScalar,
}

pub(crate) fn nullable_repr(tcx: &TyCtx, ty: TyId) -> Option<NullableRepr> {
    if !tcx.is_nullable(ty) {
        return None;
    }
    let inner = tcx.base(ty);
    if tcx.is_string(inner) || tcx.is_array(inner) {
        return Some(NullableRepr::Buffer);
    }
    match tcx.data(inner) {
        TyData::Prim(Prim::Unit) => None,
        TyData::Prim(_) => Some(NullableRepr::TaggedScalar),
        _ => None,
    }
}

pub fn codegen_lowers_nullable(tcx: &TyCtx, ty: TyId) -> bool {
    !tcx.is_nullable(ty) || nullable_repr(tcx, ty).is_some()
}

pub fn gate(hir: &HirProgram, tcx: &TyCtx) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for class in hir.classes.iter().filter(|class| !class.generics.is_empty()) {
        let message = format!("generic class `{}` is not compiled yet (monomorphization)", class.name);
        reject(&mut diagnostics, &message, None);
    }
    for function in &hir.functions {
        if !function.generics.is_empty() {
            let message = format!("generic function `{}` is not compiled yet (monomorphization)", function.name);
            reject(&mut diagnostics, &message, Some(function.name_span));
        }
        gate_block(tcx, &function.body, &mut diagnostics);
    }
    diagnostics
}

fn gate_block(tcx: &TyCtx, block: &HirBlock, diagnostics: &mut Vec<Diagnostic>) {
    for statement in &block.stmts {
        match statement {
            HirStmt::Return { value } => {
                if let Some(value) = value {
                    gate_expr(tcx, value, diagnostics);
                }
            }
            HirStmt::Block(block) | HirStmt::MoveBlock { body: block, .. } => {
                gate_block(tcx, block, diagnostics);
            }
            HirStmt::VarDecl { value, .. } | HirStmt::Expr(value) => gate_expr(tcx, value, diagnostics),
            HirStmt::Assign { target, value } => {
                if let Some(receiver) = target.receiver() {
                    gate_expr(tcx, receiver, diagnostics);
                }
                if let Some(index) = target.index() {
                    gate_expr(tcx, index, diagnostics);
                }
                gate_expr(tcx, value, diagnostics);
            }
            HirStmt::For { iter, body, .. } => {
                gate_expr(tcx, iter, diagnostics);
                gate_block(tcx, body, diagnostics);
            }
            HirStmt::While { cond, body } => {
                gate_expr(tcx, cond, diagnostics);
                gate_block(tcx, body, diagnostics);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }
}

fn gate_expr(tcx: &TyCtx, expr: &HirExpr, diagnostics: &mut Vec<Diagnostic>) {
    if !codegen_lowers_nullable(tcx, expr.ty) {
        reject(
            diagnostics,
            &format!("nullable type `{}` is not supported by codegen", tcx.display(expr.ty)),
            expr.span,
        );
    }
    match &expr.kind {
        HirExprKind::Int { .. }
        | HirExprKind::Float { .. }
        | HirExprKind::Bool { .. }
        | HirExprKind::Str { .. } => {}
        HirExprKind::Ident {
            use_kind: UseKind::Move | UseKind::Promote,
            ..
        } => {
            if tcx.record_def(expr.ty).is_some() {
                reject(
                    diagnostics,
                    "moving class objects is not supported by codegen yet",
                    expr.span,
                );
            }
            if tcx
                .array_elem(expr.ty)
                .is_some_and(|elem| tcx.is_managed_ref(elem))
            {
                reject(
                    diagnostics,
                    "moving an array of managed Ref is not supported by codegen yet",
                    expr.span,
                );
            }
        }
        HirExprKind::Ident { .. } => {}
        HirExprKind::ArrayLit { elements } => {
            for element in elements {
                gate_expr(tcx, element, diagnostics);
            }
        }
        HirExprKind::VecNew { cap, .. } => {
            if let Some(cap) = cap {
                gate_expr(tcx, cap, diagnostics);
            }
        }
        HirExprKind::VecOp { receiver, args, .. } => {
            gate_expr(tcx, receiver, diagnostics);
            for arg in args {
                gate_expr(tcx, arg, diagnostics);
            }
        }
        HirExprKind::Index {
            receiver,
            index,
            use_kind: _,
        } => {
            gate_expr(tcx, receiver, diagnostics);
            gate_expr(tcx, index, diagnostics);
        }
        HirExprKind::Slice { receiver, lo, hi } => {
            gate_expr(tcx, receiver, diagnostics);
            gate_expr(tcx, lo, diagnostics);
            gate_expr(tcx, hi, diagnostics);
        }
        HirExprKind::Binary { op, lhs, rhs } => {
            if !matches!(
                op,
                BinOp::Add
                    | BinOp::Sub
                    | BinOp::Mul
                    | BinOp::Div
                    | BinOp::Gt
                    | BinOp::Lt
                    | BinOp::Ge
                    | BinOp::Le
                    | BinOp::Eq
                    | BinOp::Ne
                    | BinOp::And
                    | BinOp::Or
                    | BinOp::Elvis
                    | BinOp::RangeTo
            ) {
                reject(
                    diagnostics,
                    "this binary operator is not supported by codegen",
                    expr.span,
                );
            }
            gate_expr(tcx, lhs, diagnostics);
            gate_expr(tcx, rhs, diagnostics);
        }
        HirExprKind::Call {
            callee,
            args,
            has_trailing_closure,
        } => {
            if *has_trailing_closure {
                reject(
                    diagnostics,
                    "trailing closures are not supported by codegen",
                    expr.span.or(callee.span),
                );
            }
            gate_expr(tcx, callee, diagnostics);
            for argument in args {
                gate_expr(tcx, argument, diagnostics);
            }
        }
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => {
            gate_expr(tcx, cond, diagnostics);
            gate_block(tcx, then_block, diagnostics);
            if let Some(else_block) = else_block {
                gate_block(tcx, else_block, diagnostics);
            }
        }
        HirExprKind::None => {}
        HirExprKind::Some(inner)
        | HirExprKind::RefCreate(inner)
        | HirExprKind::Unary {
            op: UnaryOp::NotNullAssert | UnaryOp::Not | UnaryOp::Borrow,
            expr: inner,
        } => gate_expr(tcx, inner, diagnostics),
        HirExprKind::ObjectConstruct { fields, .. } => {
            for field in fields {
                gate_expr(tcx, field, diagnostics);
            }
        }
        HirExprKind::ObjectField { receiver, .. } => {
            gate_expr(tcx, receiver, diagnostics);
        }
        HirExprKind::PresenceMatch {
            bindings,
            some_block,
            none_block,
            ..
        } => {
            for binding in bindings.iter() {
                gate_expr(tcx, &binding.value, diagnostics);
            }
            gate_block(tcx, some_block, diagnostics);
            if let Some(block) = none_block {
                gate_block(tcx, block, diagnostics);
            }
        }
        HirExprKind::Field {
            receiver,
            name,
            safe,
            ..
        } => {
            let inner = tcx.base(receiver.ty);
            let length_ok = name == "length"
                && (tcx.uses_arena_storage(receiver.ty)
                    || (*safe
                        && tcx.is_nullable(receiver.ty)
                        && (tcx.is_string(inner) || tcx.is_array(inner))));
            if !length_ok {
                reject(
                    diagnostics,
                    "field access is not supported by codegen",
                    expr.span.or(receiver.span),
                );
            }
            gate_expr(tcx, receiver, diagnostics);
        }
    }
}

fn reject(diagnostics: &mut Vec<Diagnostic>, message: &str, span: Option<Span>) {
    diagnostics.push(Diagnostic {
        phase: Phase::Codegen,
        severity: Severity::Error,
        message: message.to_owned(),
        span,
    });
}

#[cfg(test)]
mod tests {
    use crate::diag::Phase;

    #[test]
    fn gate_rejects_trailing_closure() {
        let result = crate::frontend::check(crate::MVP_SAMPLE);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), &result.tcx);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.phase == Phase::Codegen && diagnostic.span.is_some() }));
    }

    #[test]
    fn methods_pass_codegen_gate() {
        let source = "class P {\n    x: i32\n    fun get(): i32 {\n        return x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.get()\n}";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        assert!(super::gate(result.hir.as_ref().unwrap(), &result.tcx).is_empty());
    }

    #[test]
    fn nullable_sample_passes_codegen_gate() {
        let source = r#"
fun main(name: String?): String {
    val empty: String? = None
    val wrapped: String? = Some("x")
    val asserted: String = wrapped!!
    return name ?: asserted
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), &result.tcx);

        assert!(
            diagnostics.is_empty(),
            "nullable constructs should pass codegen gate: {diagnostics:?}"
        );
    }

    #[test]
    fn length_field_passes_codegen_gate() {
        let source = r#"fun main(): i32 {
    val a = [1, 2, 3]
    return a.length
}"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), &result.tcx);

        assert!(
            diagnostics.is_empty(),
            "`.length` on arrays should be allowed at the codegen gate: {diagnostics:?}"
        );
    }

    #[test]
    fn record_and_presence_pass_codegen_gate() {
        let source = r#"
class Box { value: i32 }
fun main(): i32 {
    val item = Box(7)
    val reference: Ref<Box> = item
    if val live = reference { return live.value }
    return item.value
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.is_empty(),
            "records and if-val should lower: {diagnostics:?}"
        );
    }

    #[test]
    fn moving_an_array_of_ref_is_rejected_by_the_codegen_gate() {
        let source = r#"
class Box { value: i32 }
fun main() {
    val item = Box(1)
    val first: Ref<Box> = item
    val refs = [first, None]
    val _taken = move refs
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("array of managed Ref")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn float_optional_passes_codegen_gate() {
        let source = "fun main(): i32 {\n    val n: f32? = None\n    return 0\n}\n";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.is_empty(),
            "f32? is a scalar T? and must lower: {diagnostics:?}"
        );
    }

    #[test]
    fn managed_ref_safe_field_passes_codegen_gate() {
        let source = r#"
class Point { x: i32 }
fun main(): i32 {
    val point = Point(1)
    val p: Ref<Point> = point
    return p?.x ?: 0
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.is_empty(),
            "Ref<Point> `?.x` must pass codegen gate: {diagnostics:?}"
        );
    }
}
