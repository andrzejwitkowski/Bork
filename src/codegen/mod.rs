mod escape;
mod gate;
mod link;
pub mod llvm;
pub mod regions;

use std::path::Path;

use inkwell::context::Context;
use tempfile::Builder;

use crate::diag::Diagnostic;
use crate::frontend::CheckResult;

pub use gate::gate;
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
    if !checked.is_ok() {
        return Err(BuildError::Diagnostics(checked.diagnostics.clone()));
    }
    let hir = checked.hir.as_ref().expect("clean check produces HIR");
    let report = checked
        .report
        .as_ref()
        .expect("clean check produces arenas");

    let gate_diagnostics = gate(hir);
    if !gate_diagnostics.is_empty() {
        return Err(BuildError::Diagnostics(gate_diagnostics));
    }

    let context = Context::create();
    let module = llvm::emit_module(&context, hir, report)
        .map_err(|diagnostic| BuildError::Diagnostics(vec![diagnostic]))?;

    let object = Builder::new()
        .prefix("bork-")
        .suffix(".o")
        .tempfile()
        .map_err(|err| {
            BuildError::Toolchain(format!("failed to create temporary object: {err}"))
        })?;
    let linked = llvm::write_object(&module, object.path())
        .and_then(|()| link::link_executable(object.path(), output));
    linked.map_err(BuildError::Toolchain)
}
