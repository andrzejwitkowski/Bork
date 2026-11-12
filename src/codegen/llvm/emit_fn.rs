use std::collections::HashMap;

use inkwell::module::Linkage;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::FunctionValue;

use crate::diag::Diagnostic;
use crate::hir::HirFunction;

use super::call_abi::CallAbi;
use super::context::Codegen;
use super::not_yet_supported;

pub type Callees<'h, 'ctx> = HashMap<&'h str, (FunctionValue<'ctx>, &'h HirFunction)>;

/// `main` keeps its C name and returns `i32` even when declared `unit`; other functions get
/// internal `bork.`-prefixed symbols so they cannot collide with libc or the runtime.
pub fn declare_function<'ctx>(
    cx: &Codegen<'ctx>,
    function: &HirFunction,
) -> Result<FunctionValue<'ctx>, Diagnostic> {
    let mut params = CallAbi::of(cx, function).hidden_types(cx);
    for param in &function.params {
        params.push(
            cx.basic_type(param.ty)
                .map(BasicMetadataTypeEnum::from)
                .ok_or_else(|| {
                    not_yet_supported(&format!("parameter type `{}`", cx.tcx.display(param.ty)), None)
                })?,
        );
    }

    if function.name == "main" {
        if !params.is_empty() {
            return Err(not_yet_supported("parameters on `main`", None));
        }
        if function.return_ty != cx.i32_ty() && function.return_ty != cx.unit_ty() {
            return Err(not_yet_supported(
                &format!("`main` returning `{}`", cx.tcx.display(function.return_ty)),
                None,
            ));
        }
        let fn_type = cx.context.i32_type().fn_type(&[], false);
        return Ok(cx.module.add_function("main", fn_type, None));
    }

    let fn_type = if function.return_ty == cx.unit_ty() {
        cx.context.void_type().fn_type(&params, false)
    } else {
        cx.basic_type(function.return_ty)
            .ok_or_else(|| {
                not_yet_supported(&format!("return type `{}`", cx.tcx.display(function.return_ty)), None)
            })?
            .fn_type(&params, false)
    };
    Ok(cx.module.add_function(
        &format!("bork.{}", function.name),
        fn_type,
        Some(Linkage::Internal),
    ))
}
