//! Compiler frontend orchestration (parse -> typeck -> sema -> HIR).

use crate::diag::{self, Diagnostic};
use crate::hir::HirProgram;
use crate::memory::MemoryPlan;
use crate::sema::ArenaReport;
use crate::ty::TyCtx;
use crate::typeck;

/// Outcome of one frontend run over a source string.
pub struct CheckResult {
    /// Ownership arenas, kept even when later phases report errors. `None` only
    /// when the source failed to parse.
    pub report: Option<ArenaReport>,
    /// Typed HIR, present exactly when `diagnostics` is empty.
    pub hir: Option<HirProgram>,
    /// Allocation/lifetime decisions for addressable record objects.
    pub memory_plan: Option<MemoryPlan>,
    pub diagnostics: Vec<Diagnostic>,
    /// Member names resolved by typeck (bare fields, method calls, method declarations).
    pub members: crate::typeck::MemberResolutions,
    /// Owns the type ids carried by `hir`; empty when the source failed to parse.
    pub tcx: TyCtx,
    /// Every numbered AST node with its span; the LSP resolves a position to an `AstId` with it.
    pub node_index: Vec<(crate::span::Span, crate::span::AstId)>,
}

impl CheckResult {
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

fn has_generics(hir: &crate::hir::HirProgram) -> bool {
    hir.functions.iter().any(|function| !function.generics.is_empty())
        || hir.classes.iter().any(|class| !class.generics.is_empty())
    || hir.enums.iter().any(|e| !e.generics.is_empty())
}

/// Parse, analyze ownership, and lower program to typed HIR.
pub fn check(source: &str) -> CheckResult {
    let (mut program, node_index) = match crate::parse_indexed(source) {
        Ok(parsed) => parsed,
        Err(err) => {
            return CheckResult {
                report: None,
                hir: None,
                memory_plan: None,
                diagnostics: vec![diag::from_parse(source, &err)],
                members: Default::default(),
                tcx: TyCtx::new(&Default::default()),
                node_index: Vec::new(),
            }
        }
    };

    crate::prelude::add_prelude(&mut program, source, node_index.len() as u32);
    let typeck::Typeck { hir, node_tys, members, diagnostics: mut type_diagnostics, tcx } =
        typeck::check_with_members(&program);
    let (mut report, ownership_errors) =
        crate::sema::analyze_with_members(&program, node_tys, members.clone(), &tcx);
    let mut diagnostics: Vec<_> = ownership_errors.iter().map(diag::from_sema).collect();
    diagnostics.append(&mut type_diagnostics);

    // Instances of generic code are checked here too, so `check` and the LSP agree with `build`.
    // It runs on a copy of the type context (specialising interns new types), and the HIR that
    // hover and the LSP read stays the generic one.
    if diagnostics.is_empty() && has_generics(&hir) {
        diagnostics.extend(crate::mono::check_instances(&hir, &mut tcx.clone()));
    }
    let mut hir = diagnostics.is_empty().then_some(hir);
    let mut memory_plan = None;
    if let Some(mut program) = hir {
        crate::hoist::annotate(&mut program);
        crate::hir::number_nodes(&mut program);
        for function in &program.functions {
            crate::escape::check_function(function, &tcx, &mut diagnostics);
        }
        if diagnostics.is_empty() {
            let (plan, mut plan_diagnostics) = crate::memory::plan(&program, &tcx, &report);
            diagnostics.append(&mut plan_diagnostics);
            if diagnostics.is_empty() {
                match crate::region_walk::stamp_codegen_push(&program, &tcx, &mut report) {
                    Ok(()) => memory_plan = Some(plan),
                    Err(ice) => diagnostics.push(ice),
                }
            }
        }
        hir = diagnostics.is_empty().then_some(program);
    }

    CheckResult {
        report: Some(report),
        hir,
        memory_plan,
        diagnostics,
        members,
        tcx,
        node_index,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Phase;

    fn hir_of(source: &str) -> HirProgram {
        let result = check(source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.hir.expect("clean check produces HIR")
    }

    #[test]
    fn check_rejects_parse_error() {
        let result = check("fun oops(");
        assert!(result.diagnostics.iter().any(|d| d.phase == Phase::Parse));
        assert!(result.report.is_none());
        assert!(result.hir.is_none());
    }

    #[test]
    fn check_ok_on_minimal_typed_program() {
        assert_eq!(hir_of("fun main(): i32 { return 1 }").functions.len(), 1);
    }

    #[test]
    fn check_rejects_ownership_error() {
        let src = r#"
fun main() {
    var s: String = "hi"
    move (s) {}
    val t = s
}
"#;
        let result = check(src);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.phase == Phase::Ownership));
        assert!(result.hir.is_none());
    }

    #[test]
    fn report_survives_semantic_errors() {
        let src = r#"
fun main(): i32 {
    var s: String = "a"
    val t = move s
    val u = s
    return 1 + "x"
}
"#;
        let result = check(src);
        assert!(!result.diagnostics.is_empty());
        assert!(result.report.is_some(), "arenas are kept alongside errors");
    }

    #[test]
    fn reports_ownership_and_type_together() {
        let src = r#"
fun main(): i32 {
    var s: String = "a"
    val t = move s
    val u = s
    return 1 + "x"
}
"#;
        let diagnostics = check(src).diagnostics;
        assert!(diagnostics.iter().any(|d| d.phase == Phase::Ownership));
        assert!(diagnostics.iter().any(|d| d.phase == Phase::Type));
    }

    #[test]
    fn check_lowers_function_shells() {
        let src = r#"
fun add(val a: Int, var b: Int): Int {
    return a
}
"#;
        let checked = check(src);
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let (hir, tcx) = (checked.hir.expect("clean check produces HIR"), checked.tcx);
        let i32_ty = tcx.prim_id(crate::hir::Prim::I32);
        assert_eq!(hir.functions.len(), 1);
        let f = &hir.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.return_ty, i32_ty);
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "a");
        assert_eq!(f.params[0].kind, crate::ast::BindingKind::Val);
        assert_eq!(f.params[0].ty, i32_ty);
        assert_eq!(f.params[1].name, "b");
        assert_eq!(f.params[1].kind, crate::ast::BindingKind::Var);
        assert_eq!(f.params[1].ty, i32_ty);
        assert!(matches!(
            f.body.stmts.as_slice(),
            [crate::hir::HirStmt::Return { value: Some(_) }]
        ));
    }
}
