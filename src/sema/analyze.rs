//! Region analysis entrypoints.

use super::env::{Analyzer, Ty};
use super::region::RegionParam;
use super::report::{ArenaNode, ArenaReport, SemaError};
use super::walk::open_ordinary;
use crate::ast::{Function, Program};

/// Analyze `program` for arena hierarchy and Copy/Move ownership.
pub fn analyze(program: &Program) -> (ArenaReport, Vec<SemaError>) {
    let (_, span_tys, _) = crate::typeck::check(program);
    analyze_with_span_tys(program, span_tys)
}

/// Same as [`analyze`], but reuses `span_tys` from an earlier `typeck::check` (avoids duplicate work).
pub fn analyze_with_span_tys(
    program: &Program,
    span_tys: crate::typeck::SpanTypes,
) -> (ArenaReport, Vec<SemaError>) {
    let mut az = Analyzer::new();
    az.span_tys = span_tys;
    az.classes = program
        .classes
        .iter()
        .map(|class| (class.name.clone(), class.clone()))
        .collect();
    // MVP: keyed by bare function name (no local shadowing of callees yet).
    for f in &program.functions {
        az.fun_sigs.insert(
            f.name.clone(),
            f.params.iter().map(|p| p.kind).collect(),
        );
        az.fun_param_tys.insert(
            f.name.clone(),
            f.params.iter().map(|p| p.ty.clone()).collect(),
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
            borrow_from: super::region::REF_PARAM_BORROW_FROM,
        })
        .collect();
    open_ordinary(az, &format!("fun {}", func.name), &func.body, &params, None)
}
