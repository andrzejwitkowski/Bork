//! Codegen subset checks shared by `bork build` and the LSP (no LLVM dependency).

use crate::ast::{BinOp, UnaryOp};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{
    HirBlock, HirExpr, HirExprKind, HirProgram, HirStmt, Prim, Ty, TyKind, UseKind,
};
use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NullableRepr {
    Buffer,
    TaggedScalar,
}

pub(crate) fn nullable_repr(ty: &Ty) -> Option<NullableRepr> {
    if !ty.is_nullable() {
        return None;
    }
    let inner = ty.with_nullable(false);
    if inner.is_string() || inner.is_array() {
        return Some(NullableRepr::Buffer);
    }
    match inner.kind {
        TyKind::Prim(Prim::Unit) => None,
        TyKind::Prim(_) => Some(NullableRepr::TaggedScalar),
        _ => None,
    }
}

pub fn codegen_lowers_nullable(ty: &Ty) -> bool {
    !ty.is_nullable() || nullable_repr(ty).is_some()
}

pub fn gate(hir: &HirProgram) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for function in &hir.functions {
        gate_block(&function.body, &mut diagnostics);
    }
    diagnostics
}

fn gate_block(block: &HirBlock, diagnostics: &mut Vec<Diagnostic>) {
    for statement in &block.stmts {
        match statement {
            HirStmt::Return { value } => {
                if let Some(value) = value {
                    gate_expr(value, diagnostics);
                }
            }
            HirStmt::Block(block) | HirStmt::MoveBlock { body: block, .. } => {
                gate_block(block, diagnostics);
            }
            HirStmt::VarDecl { value, .. } | HirStmt::Expr(value) => gate_expr(value, diagnostics),
            HirStmt::Assign { target, value } => {
                if let Some(receiver) = target.receiver() {
                    gate_expr(receiver, diagnostics);
                }
                if let Some(index) = target.index() {
                    gate_expr(index, diagnostics);
                }
                gate_expr(value, diagnostics);
            }
            HirStmt::For { iter, body, .. } => {
                gate_expr(iter, diagnostics);
                gate_block(body, diagnostics);
            }
            HirStmt::While { cond, body } => {
                gate_expr(cond, diagnostics);
                gate_block(body, diagnostics);
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }
}

fn gate_expr(expr: &HirExpr, diagnostics: &mut Vec<Diagnostic>) {
    if !codegen_lowers_nullable(&expr.ty) {
        reject(
            diagnostics,
            &format!("nullable type `{}` is not supported by codegen", expr.ty),
            expr.span,
        );
    }
    match &expr.kind {
        HirExprKind::Int { .. }
        | HirExprKind::Float { .. }
        | HirExprKind::Bool { .. }
        | HirExprKind::Str { .. } => {}
        HirExprKind::Ident { use_kind, .. } => {
            if matches!(use_kind, UseKind::Move | UseKind::Promote)
                && expr.ty.record_name().is_some()
            {
                reject(
                    diagnostics,
                    "moving class objects is not supported by codegen yet",
                    expr.span,
                );
            }
            if matches!(use_kind, UseKind::Move | UseKind::Promote)
                && expr
                    .ty
                    .array_elem()
                    .is_some_and(|elem| elem.is_managed_ref())
            {
                reject(
                    diagnostics,
                    "moving an array of managed Ref is not supported by codegen yet",
                    expr.span,
                );
            }
        }
        HirExprKind::ArrayLit { elements } => {
            for element in elements {
                gate_expr(element, diagnostics);
            }
        }
        HirExprKind::Index {
            receiver,
            index,
            use_kind: _,
        } => {
            gate_expr(receiver, diagnostics);
            gate_expr(index, diagnostics);
        }
        HirExprKind::Slice { receiver, lo, hi } => {
            gate_expr(receiver, diagnostics);
            gate_expr(lo, diagnostics);
            gate_expr(hi, diagnostics);
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
            gate_expr(lhs, diagnostics);
            gate_expr(rhs, diagnostics);
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
            gate_expr(callee, diagnostics);
            for argument in args {
                gate_expr(argument, diagnostics);
            }
        }
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => {
            gate_expr(cond, diagnostics);
            gate_block(then_block, diagnostics);
            if let Some(else_block) = else_block {
                gate_block(else_block, diagnostics);
            }
        }
        HirExprKind::None => {}
        HirExprKind::Some(inner)
        | HirExprKind::RefCreate(inner)
        | HirExprKind::Unary {
            op: UnaryOp::NotNullAssert | UnaryOp::Not | UnaryOp::Borrow,
            expr: inner,
        } => gate_expr(inner, diagnostics),
        HirExprKind::ObjectConstruct { fields, .. } => {
            for field in fields {
                gate_expr(field, diagnostics);
            }
        }
        HirExprKind::ObjectField { receiver, .. } => {
            gate_expr(receiver, diagnostics);
        }
        HirExprKind::PresenceMatch {
            bindings,
            some_block,
            none_block,
            ..
        } => {
            for binding in bindings {
                gate_expr(&binding.value, diagnostics);
            }
            gate_block(some_block, diagnostics);
            if let Some(block) = none_block {
                gate_block(block, diagnostics);
            }
        }
        HirExprKind::Field {
            receiver,
            name,
            safe,
            ..
        } => {
            let inner = receiver.ty.with_nullable(false);
            let length_ok = name == "length"
                && (receiver.ty.uses_arena_storage()
                    || (*safe
                        && receiver.ty.is_nullable()
                        && (inner.is_string() || inner.is_array())));
            if !length_ok {
                reject(
                    diagnostics,
                    "field access is not supported by codegen",
                    expr.span.or(receiver.span),
                );
            }
            gate_expr(receiver, diagnostics);
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

        let diagnostics = super::gate(result.hir.as_ref().unwrap());

        assert!(diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.phase == Phase::Codegen && diagnostic.span.is_some() }));
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

        let diagnostics = super::gate(result.hir.as_ref().unwrap());

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

        let diagnostics = super::gate(result.hir.as_ref().unwrap());

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
        let diagnostics = super::gate(result.hir.as_ref().unwrap());
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
        let diagnostics = super::gate(result.hir.as_ref().unwrap());
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

        let diagnostics = super::gate(result.hir.as_ref().unwrap());
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
        let diagnostics = super::gate(result.hir.as_ref().unwrap());
        assert!(
            diagnostics.is_empty(),
            "Ref<Point> `?.x` must pass codegen gate: {diagnostics:?}"
        );
    }
}
