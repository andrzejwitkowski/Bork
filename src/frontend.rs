//! Compiler frontend orchestration (parse -> sema -> typeck -> HIR).

use crate::diag::{self, Diagnostic};
use crate::hir::HirProgram;
use crate::parse;
use crate::sema::analyze;
use crate::typeck;

/// Parse, analyze ownership, and lower program to typed HIR.
pub fn check(source: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let program = match parse(source) {
        Ok(p) => p,
        Err(err) => return Err(vec![diag::from_parse(&err)]),
    };

    let (_report, ownership_errors) = analyze(&program);
    let mut diagnostics: Vec<_> = ownership_errors.iter().map(diag::from_sema).collect();
    let (hir, mut type_diagnostics) = typeck::check(&program);
    diagnostics.append(&mut type_diagnostics);

    if diagnostics.is_empty() {
        Ok(hir.expect("type checking without diagnostics produces HIR"))
    } else {
        Err(diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Phase;
    use crate::hir::Ty;

    #[test]
    fn check_rejects_parse_error() {
        let err = check("fun oops(").unwrap_err();
        assert!(err.iter().any(|d| d.phase == Phase::Parse));
    }

    #[test]
    fn check_ok_on_minimal_typed_program() {
        let hir = check("fun main(): i32 { return 1 }").expect("program is valid");
        assert_eq!(hir.functions.len(), 1);
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
        let err = check(src).unwrap_err();
        assert!(err.iter().any(|d| d.phase == Phase::Ownership));
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
        let err = check(src).unwrap_err();
        assert!(err.iter().any(|d| d.phase == Phase::Ownership));
        assert!(err.iter().any(|d| d.phase == Phase::Type));
    }

    #[test]
    fn check_lowers_function_shells() {
        let src = r#"
fun add(val a: Int, var b: Int): Int {
    return a
}
"#;
        let hir = check(src).expect("clean check");
        assert_eq!(hir.functions.len(), 1);
        let f = &hir.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.region, 0);
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
