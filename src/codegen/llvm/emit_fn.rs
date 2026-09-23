use inkwell::types::BasicType;
use inkwell::values::IntValue;

use crate::diag::Diagnostic;
use crate::hir::{HirExpr, HirExprKind, HirFunction, HirStmt};

use super::context::Codegen;
use super::{builder_error, not_yet_supported};

pub fn emit_function(cx: &Codegen<'_>, function: &HirFunction) -> Result<(), Diagnostic> {
    if !function.params.is_empty() {
        return Err(not_yet_supported("function parameters", None));
    }
    let return_ty = cx
        .basic_type(&function.return_ty)
        .ok_or_else(|| not_yet_supported(&format!("return type `{}`", function.return_ty), None))?;

    let llvm_fn = cx
        .module
        .add_function(&function.name, return_ty.fn_type(&[], false), None);
    let entry = cx.context.append_basic_block(llvm_fn, "entry");
    cx.builder.position_at_end(entry);

    match function.body.stmts.as_slice() {
        [HirStmt::Return { value: Some(value) }] => {
            let value = emit_int_expr(cx, value)?;
            cx.builder
                .build_return(Some(&value))
                .map_err(builder_error)?;
            Ok(())
        }
        _ => Err(not_yet_supported(
            &format!("the body of `{}`", function.name),
            None,
        )),
    }
}

fn emit_int_expr<'ctx>(cx: &Codegen<'ctx>, expr: &HirExpr) -> Result<IntValue<'ctx>, Diagnostic> {
    match expr.kind {
        HirExprKind::Int { value } => {
            let ty = cx
                .basic_type(&expr.ty)
                .ok_or_else(|| not_yet_supported(&format!("type `{}`", expr.ty), expr.span))?;
            Ok(ty.into_int_type().const_int(value as u64, true))
        }
        _ => Err(not_yet_supported("this expression", expr.span)),
    }
}
