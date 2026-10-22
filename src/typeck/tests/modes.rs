//! Bidirectional modes: expected types flow into `if` branches and array literals, and a
//! mismatch is worded in one place.
use super::{checked_of, diags_of, shown};
use crate::hir::HirStmt;

fn decl_ty(src: &str) -> String {
    let (hir, tcx) = checked_of(src);
    let HirStmt::VarDecl { ty, .. } = &hir.functions[0].body.stmts[0] else {
        panic!("expected a declaration");
    };
    shown(&tcx, *ty)
}

#[test]
fn expected_type_reaches_if_branches() {
    let src = "fun main(c: bool): i64 {\n    val x: i64 = if (c) { 1 } else { 2 }\n    return x\n}\n";
    assert_eq!(decl_ty(src), "i64");
}

#[test]
fn expected_nullable_reaches_none_in_if_branch() {
    let src = "fun main(c: bool): i32 {\n    val x: i32? = if (c) { Some(1) } else { None }\n    return 0\n}\n";
    assert_eq!(decl_ty(src), "i32?");
}

#[test]
fn if_branch_mismatch_uses_shared_wording() {
    let src = "fun main(c: bool): i32 {\n    val x: i32 = if (c) { 1 } else { \"a\" }\n    return x\n}\n";
    let messages: Vec<_> = diags_of(src).into_iter().map(|d| d.message).collect();
    assert!(messages.iter().any(|m| m.contains("else branch has type")), "{messages:?}");
}

#[test]
fn expected_element_type_reaches_array_literal() {
    assert_eq!(decl_ty("fun main(): i32 {\n    val a: [i64; 2] = [1, 2]\n    return 0\n}\n"), "[i64; 2]");
}

#[test]
fn array_length_mismatch_is_an_error() {
    let src = "fun main(): i32 {\n    val a: [i32; 3] = [1, 2]\n    return 0\n}\n";
    assert!(diags_of(src).iter().any(|d| d.message.contains("expected 3")));
}

#[test]
fn synthesized_mismatches_share_one_message_shape() {
    for (src, what) in [
        ("fun main(): i32 {\n    return \"x\"\n}\n", "return value has type"),
        ("fun main(): i32 {\n    val x: i32 = \"x\"\n    return x\n}\n", "initializer for `x` has type"),
        ("fun f(a: i32): i32 {\n    return a\n}\nfun main(): i32 {\n    return f(\"x\")\n}\n", "argument 1 to `f` has type"),
    ] {
        let messages: Vec<_> = diags_of(src).into_iter().map(|d| d.message).collect();
        assert!(messages.iter().any(|m| m.contains(what) && m.contains(", expected i32")), "{messages:?}");
    }
}
