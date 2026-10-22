mod link;
pub mod llvm;
pub mod regions;

use std::path::Path;

use inkwell::context::Context;
use tempfile::Builder;

use crate::diag::Diagnostic;
use crate::frontend::CheckResult;

pub use crate::codegen_gate::gate;
pub use regions::{schedule, RegionEmitter, RegionEvent, RegionSink, RegionSite, ScheduleError};

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

/// Builds with source-level debugging when requested. Debug builds require source context.
pub fn build_with_options(
    checked: &CheckResult,
    output: &Path,
    options: BuildOptions,
    source: Option<SourceContext<'_>>,
) -> Result<(), BuildError> {
    if !checked.is_ok() {
        return Err(BuildError::Diagnostics(checked.diagnostics.clone()));
    }
    let hir = checked.hir.as_ref().expect("clean check produces HIR");
    let report = checked
        .report
        .as_ref()
        .expect("clean check produces arenas");

    let gate_diagnostics = gate(hir, &checked.tcx);
    if !gate_diagnostics.is_empty() {
        return Err(BuildError::Diagnostics(gate_diagnostics));
    }

    let context = Context::create();
    let memory_plan = checked
        .memory_plan
        .as_ref()
        .expect("clean check produces memory plan");
    let module = if options.debug {
        let source = source.ok_or_else(|| {
            BuildError::Toolchain("debug builds require source path and text".into())
        })?;
        llvm::emit_module_with_debug(
            &context,
            hir,
            &checked.tcx,
            report,
            memory_plan.clone(),
            source.text,
            source.path,
        )
    } else {
        llvm::emit_module(&context, hir, &checked.tcx, report, memory_plan.clone())
    }
    .map_err(|diagnostic| BuildError::Diagnostics(vec![diagnostic]))?;

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
