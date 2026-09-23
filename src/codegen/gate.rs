use crate::ast::BinOp;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirProgram, HirStmt};
use crate::span::Span;

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
            HirStmt::VarDecl { value, .. }
            | HirStmt::Assign { value, .. }
            | HirStmt::Expr(value) => gate_expr(value, diagnostics),
            HirStmt::For { iter, body, .. } => {
                gate_expr(iter, diagnostics);
                gate_block(body, diagnostics);
            }
        }
    }
}

fn gate_expr(expr: &HirExpr, diagnostics: &mut Vec<Diagnostic>) {
    match &expr.kind {
        HirExprKind::Int { .. } | HirExprKind::Str { .. } | HirExprKind::Ident { .. } => {}
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
                    | BinOp::RangeTo
            ) {
                reject(diagnostics, "`?:` is not supported by codegen", expr.span);
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
        HirExprKind::None => reject(diagnostics, "`None` is not supported by codegen", expr.span),
        HirExprKind::Some(inner) => {
            reject(
                diagnostics,
                "`Some` is not supported by codegen",
                expr.span.or(inner.span),
            );
            gate_expr(inner, diagnostics);
        }
        HirExprKind::Unary { expr: inner, .. } => {
            reject(
                diagnostics,
                "`!!` is not supported by codegen",
                expr.span.or(inner.span),
            );
            gate_expr(inner, diagnostics);
        }
        HirExprKind::Field { receiver, .. } => {
            reject(
                diagnostics,
                "field access is not supported by codegen",
                expr.span.or(receiver.span),
            );
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
    fn nullable_rejections_have_codegen_spans() {
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

        for construct in ["`None`", "`Some`", "`!!`", "`?:`"] {
            assert!(
                diagnostics.iter().any(|diagnostic| {
                    diagnostic.phase == Phase::Codegen
                        && diagnostic.message.contains(construct)
                        && diagnostic.span.is_some()
                }),
                "missing spanned codegen diagnostic for {construct}: {diagnostics:?}"
            );
        }
    }
}
