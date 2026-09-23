//! Compiler frontend orchestration (parse -> sema -> typeck -> HIR).

use crate::diag::{self, Diagnostic};
use crate::hir::{HirBlock, HirFunction, HirParam, HirProgram, RegionId, Ty};
use crate::parse;
use crate::sema::analyze;

/// Parse, analyze ownership, and lower program to typed HIR.
pub fn check(source: &str) -> Result<HirProgram, Vec<Diagnostic>> {
    let program = match parse(source) {
        Ok(p) => p,
        Err(err) => return Err(vec![diag::from_parse(&err)]),
    };

    let (_report, errors) = analyze(&program);
    if !errors.is_empty() {
        return Err(errors.iter().map(diag::from_sema).collect());
    }

    let functions = program
        .functions
        .into_iter()
        .enumerate()
        .map(|(idx, f)| HirFunction {
            name: f.name,
            params: f
                .params
                .into_iter()
                .map(|p| HirParam {
                    kind: p.kind,
                    name: p.name.name,
                    ty: Ty::from_ast(&p.ty),
                })
                .collect(),
            return_ty: Ty::from_ast(&f.return_type),
            body: HirBlock { stmts: Vec::new() },
            region: idx as RegionId,
        })
        .collect();

    Ok(HirProgram { functions })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Phase;

    #[test]
    fn check_rejects_parse_error() {
        let err = check("fun oops(").unwrap_err();
        assert!(err.iter().any(|d| d.phase == Phase::Parse));
    }

    #[test]
    fn check_ok_on_mvp_sample_ownership() {
        let hir = check(crate::MVP_SAMPLE).expect("mvp ownership clean");
        assert_eq!(hir.functions.len(), 2);
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
    fn check_lowers_function_shells() {
        let src = r#"
fun add(val a: Int, var b: Int): Int {
    return a + b
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
        assert!(f.body.stmts.is_empty());
    }
}
