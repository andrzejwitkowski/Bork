//! Inkwell (LLVM 18) emission for the codegen subset.

mod context;
mod emit_fn;

use std::path::Path;

use inkwell::builder::BuilderError;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::FileType;

use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::HirProgram;
use crate::sema::ArenaReport;
use crate::span::Span;

use context::{native_target_machine, Codegen};

pub fn emit_module<'ctx>(
    context: &'ctx Context,
    hir: &HirProgram,
    _report: &ArenaReport,
) -> Result<Module<'ctx>, Diagnostic> {
    if !hir.functions.iter().any(|function| function.name == "main") {
        return Err(codegen_error(
            "`fun main` is required to build an executable".into(),
            None,
        ));
    }

    let cx = Codegen::new(context, "bork");
    for function in &hir.functions {
        emit_fn::emit_function(&cx, function)?;
    }
    cx.module
        .verify()
        .map_err(|err| codegen_error(format!("LLVM rejected the module: {err}"), None))?;
    Ok(cx.module)
}

/// Failures here are toolchain problems, not user errors.
pub fn write_object(module: &Module<'_>, path: &Path) -> Result<(), String> {
    let machine = native_target_machine()?;
    module.set_triple(&machine.get_triple());
    module.set_data_layout(&machine.get_target_data().get_data_layout());
    machine
        .write_to_file(module, FileType::Object, path)
        .map_err(|err| format!("failed to write {}: {err}", path.display()))
}

fn codegen_error(message: String, span: Option<Span>) -> Diagnostic {
    Diagnostic {
        phase: Phase::Codegen,
        severity: Severity::Error,
        message,
        span,
    }
}

fn not_yet_supported(what: &str, span: Option<Span>) -> Diagnostic {
    codegen_error(format!("{what} is not supported by codegen yet"), span)
}

fn builder_error(err: BuilderError) -> Diagnostic {
    codegen_error(format!("LLVM builder error: {err}"), None)
}
