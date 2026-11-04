//! `bork build --emit-mir`: check, monomorphize, lower, verify and print, with no LLVM involved.

use super::build::{build_program, BuildError};
use super::print::print_body;
use super::verify::verify_or_panic;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::frontend::CheckResult;
use crate::mono::monomorphize;

pub fn emit_mir(checked: &CheckResult) -> Result<String, Vec<Diagnostic>> {
    if !checked.is_ok() {
        return Err(checked.diagnostics.clone());
    }
    let (hir, report) = (checked.hir.as_ref().expect("clean check produces HIR"), checked.report.as_ref().expect("clean check produces a report"));
    let mut tcx = checked.tcx.clone();
    let mono = monomorphize(hir, &mut tcx, report)?;
    let bodies = build_program(&mono, &tcx).map_err(|e| vec![diagnostic(e)])?;
    let mut out = String::new();
    for body in &bodies {
        verify_or_panic(body, &tcx);
        out.push_str(&print_body(body, &tcx));
        out.push('\n');
    }
    Ok(out)
}

fn diagnostic(e: BuildError) -> Diagnostic {
    let (message, span) = match e {
        BuildError::Unsupported(u) => (format!("`{}` is not supported by MIR lowering yet", u.capability), Some(u.span)),
        BuildError::Unlowered { what, span } => (format!("{what} is not supported by MIR lowering"), Some(span)),
        BuildError::Internal(m) => (format!("internal error building MIR: {m}"), None),
    };
    Diagnostic { phase: Phase::Codegen, severity: Severity::Error, message, span, notes: Vec::new() }
}
