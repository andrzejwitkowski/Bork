//! Compiler frontend orchestration (parse -> sema -> typeck -> HIR).

use crate::diag::{self, Diagnostic};
use crate::hir::HirProgram;
use crate::parse;
use crate::sema::{analyze, ArenaReport};
use crate::typeck;

/// Outcome of one frontend run over a source string.
pub struct CheckResult {
    /// Ownership arenas, kept even when later phases report errors. `None` only
    /// when the source failed to parse.
    pub report: Option<ArenaReport>,
    /// Typed HIR, present exactly when `diagnostics` is empty.
    pub hir: Option<HirProgram>,
    pub diagnostics: Vec<Diagnostic>,
}

impl CheckResult {
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

/// Parse, analyze ownership, and lower program to typed HIR.
pub fn check(source: &str) -> CheckResult {
    let program = match parse(source) {
        Ok(program) => program,
        Err(err) => {
            return CheckResult {
                report: None,
                hir: None,
                diagnostics: vec![diag::from_parse(source, &err)],
            }
        }
    };

    let (report, ownership_errors) = analyze(&program);
    let mut diagnostics: Vec<_> = ownership_errors.iter().map(diag::from_sema).collect();
    let (hir, mut type_diagnostics) = typeck::check(&program);
    diagnostics.append(&mut type_diagnostics);

    let mut hir = diagnostics.is_empty().then_some(hir).flatten();
    if let Some(mut program) = hir {
        crate::hoist::annotate(&mut program);
        #[cfg(feature = "codegen")]
        {
            let mut escape_diags = Vec::new();
            for function in &program.functions {
                crate::codegen::escape::check_function(function, &mut escape_diags);
            }
            for diagnostic in escape_diags {
                diagnostics.push(crate::diag::Diagnostic {
                    phase: crate::diag::Phase::Ownership,
                    severity: diagnostic.severity,
                    message: diagnostic.message,
                    span: diagnostic.span,
                });
            }
        }
        hir = diagnostics.is_empty().then_some(program);
    }

    CheckResult {
        report: Some(report),
        hir,
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Phase;
    use crate::hir::Ty;

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
        let hir = hir_of(src);
        assert_eq!(hir.functions.len(), 1);
        let f = &hir.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.return_ty, Ty::i32());
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "a");
        assert_eq!(f.params[0].kind, crate::ast::BindingKind::Val);
        assert_eq!(f.params[0].ty, Ty::i32());
        assert_eq!(f.params[1].name, "b");
        assert_eq!(f.params[1].kind, crate::ast::BindingKind::Var);
        assert_eq!(f.params[1].ty, Ty::i32());
        assert!(matches!(
            f.body.stmts.as_slice(),
            [crate::hir::HirStmt::Return { value: Some(_) }]
        ));
    }
}
