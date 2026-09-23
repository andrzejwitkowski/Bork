//! Region analysis entrypoints.

use super::env::{Analyzer, Ty};
use super::region::RegionParam;
use super::report::{ArenaNode, ArenaReport, SemaError};
use super::walk::open_ordinary;
use crate::ast::{Function, Program};

/// Analyze `program` for arena hierarchy and Copy/Move ownership.
pub fn analyze(program: &Program) -> (ArenaReport, Vec<SemaError>) {
    let mut az = Analyzer::new();
    // MVP: keyed by bare function name (no local shadowing of callees yet).
    for f in &program.functions {
        az.fun_sigs.insert(
            f.name.clone(),
            f.params.iter().map(|p| p.kind).collect(),
        );
    }
    let roots = program
        .functions
        .iter()
        .map(|f| analyze_function(&mut az, f))
        .collect();
    (ArenaReport { roots }, az.errors)
}

fn analyze_function(az: &mut Analyzer, func: &Function) -> ArenaNode {
    let params: Vec<RegionParam> = func
        .params
        .iter()
        .map(|p| RegionParam {
            name: p.name.name.clone(),
            ty: Ty::Known(p.ty.clone()),
            kind: p.kind,
            span: Some(p.name.span),
        })
        .collect();
    open_ordinary(az, &format!("fun {}", func.name), &func.body, &params)
}
