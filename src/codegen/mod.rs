mod link;
pub mod llvm;

use std::path::Path;

use inkwell::context::Context;
use tempfile::Builder;

use crate::diag::Diagnostic;
use crate::frontend::CheckResult;

pub use crate::codegen_gate::gate;

#[derive(Debug)]
pub enum BuildError {
    /// Problems in the user's program; no binary is produced.
    Diagnostics(Vec<Diagnostic>),
    /// LLVM, object writing, or linking failed.
    Toolchain(String),
}

/// Compiles an already-checked program to a native executable at `output`.
pub fn build(checked: &CheckResult, output: &Path) -> Result<(), BuildError> {
    build_with_options(checked, output, BuildOptions::default(), None)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct BuildOptions {
    pub debug: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct SourceContext<'a> {
    pub path: &'a Path,
    pub text: &'a str,
}

fn emit_checked<'ctx>(
    context: &'ctx Context,
    checked: &CheckResult,
    options: BuildOptions,
    source: Option<SourceContext<'_>>,
) -> Result<inkwell::module::Module<'ctx>, BuildError> {
    if !checked.is_ok() {
        return Err(BuildError::Diagnostics(checked.diagnostics.clone()));
    }
    let hir = checked.hir.as_ref().expect("clean check produces HIR");
    let report = checked
        .report
        .as_ref()
        .expect("clean check produces arenas");

    // Codegen only sees concrete code: generic functions and classes are specialised first, and the
    // report and memory plan must come from the same `Mono` as the HIR (node ids are renumbered).
    let mut tcx = checked.tcx.clone();
    let mono = crate::mono::monomorphize(hir, &mut tcx, report).map_err(BuildError::Diagnostics)?;
    let bodies = llvm::build_bodies(&mono, &tcx).map_err(|d| BuildError::Diagnostics(vec![d]))?;
    let gate_diagnostics = crate::codegen_gate::gate_results(&bodies);
    if !gate_diagnostics.is_empty() {
        return Err(BuildError::Diagnostics(gate_diagnostics));
    }
    let debug = match options.debug {
        true => {
            let source = source.ok_or_else(|| BuildError::Toolchain("debug builds require source path and text".into()))?;
            Some((source.text, source.path))
        }
        false => None,
    };
    llvm::emit_module_from(context, &mono, &tcx, debug, bodies)
    .map_err(|diagnostic| BuildError::Diagnostics(vec![diagnostic]))
}

/// LLVM IR text of the checked program, for tests that compare backends.
#[doc(hidden)]
pub fn emit_ir(checked: &CheckResult, options: BuildOptions) -> Result<String, BuildError> {
    emit_ir_with_source(checked, options, None)
}

/// As [`emit_ir`], with the source context `-g` builds require.
#[doc(hidden)]
pub fn emit_ir_with_source(checked: &CheckResult, options: BuildOptions, source: Option<SourceContext<'_>>) -> Result<String, BuildError> {
    let context = Context::create();
    let module = emit_checked(&context, checked, options, source)?;
    let machine = llvm::native_target_machine().map_err(BuildError::Toolchain)?;
    module
        .run_passes("unreachableblockelim", &machine, inkwell::passes::PassBuilderOptions::create())
        .map_err(|e| BuildError::Toolchain(e.to_string()))?;
    Ok(module.print_to_string().to_string())
}

/// Builds with source-level debugging when requested. Debug builds require source context.
pub fn build_with_options(
    checked: &CheckResult,
    output: &Path,
    options: BuildOptions,
    source: Option<SourceContext<'_>>,
) -> Result<(), BuildError> {
    let context = Context::create();
    let module = emit_checked(&context, checked, options, source)?;
    let object = Builder::new()
        .prefix("bork-")
        .suffix(".o")
        .tempfile()
        .map_err(|err| {
            BuildError::Toolchain(format!("failed to create temporary object: {err}"))
        })?;
    let linked =
        llvm::write_object_with_debug(&module, object.path(), options.debug).and_then(|()| {
            if options.debug {
                link::link_executable_with_debug(object.path(), output, true)
            } else {
                link::link_executable(object.path(), output)
            }
        });
    linked.map_err(BuildError::Toolchain)
}
