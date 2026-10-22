//! Untyped literal locals: a later use fixes the type, otherwise the body end defaults it.
use super::{checked_of, diags_of, shown};
use crate::hir::HirStmt;

fn decl_ty(src: &str, index: usize) -> String {
    let (hir, tcx) = checked_of(src);
    let HirStmt::VarDecl { ty, .. } = &hir.functions.last().unwrap().body.stmts[index] else {
        panic!("expected a declaration");
    };
    shown(&tcx, *ty)
}

const TAKES: &str = "fun takes_u8(a: u8) {}\nfun takes_i64(a: i64) {}\nfun takes_f32(a: f32) {}\nfun takes_f64(a: f64) {}\n";

#[test]
fn later_use_fixes_the_literal_type() {
    let src = format!("{TAKES}fun main() {{\n    val x = 1\n    takes_u8(x)\n}}\n");
    assert_eq!(decl_ty(&src, 0), "u8");
    let src = format!("{TAKES}fun main() {{\n    val x = 1.5\n    takes_f32(x)\n}}\n");
    assert_eq!(decl_ty(&src, 0), "f32");
}

#[test]
fn unconstrained_literals_default_once_at_the_end() {
    assert_eq!(decl_ty("fun main() {\n    val x = 1\n    val y = x\n}\n", 1), "i32");
    assert_eq!(decl_ty("fun main() {\n    val x = 1.5\n}\n", 0), "f64");
}

#[test]
fn arithmetic_and_assignment_share_the_variable() {
    let src = format!("{TAKES}fun main() {{\n    var n = 0\n    n = n + 1\n    takes_i64(n)\n}}\n");
    assert_eq!(decl_ty(&src, 0), "i64");
}

#[test]
fn conflicting_uses_are_reported_at_the_second() {
    let src = format!("{TAKES}fun main() {{\n    val x = 1\n    takes_u8(x)\n    takes_i64(x)\n}}\n");
    let messages: Vec<_> = diags_of(&src).into_iter().map(|d| d.message).collect();
    assert!(messages.iter().any(|m| m.contains("argument 1 to `takes_i64` has type u8, expected i64")), "{messages:?}");
}

#[test]
fn integer_literal_is_not_a_float() {
    let src = format!("{TAKES}fun main() {{\n    val x = 1\n    takes_f64(x)\n}}\n");
    let messages: Vec<_> = diags_of(&src).into_iter().map(|d| d.message).collect();
    assert!(messages.iter().any(|m| m.contains("argument 1 to `takes_f64` has type i32, expected f64")), "{messages:?}");
}

#[test]
fn return_type_reaches_the_literal() {
    let (hir, tcx) = checked_of("fun main(): i64 {\n    val x = 1\n    return x\n}\n");
    assert_eq!(shown(&tcx, hir.functions[0].return_ty), "i64");
    assert_eq!(decl_ty("fun main(): i64 {\n    val x = 1\n    return x\n}\n", 0), "i64");
}
