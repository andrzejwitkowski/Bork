use super::{diags_of, hir_of};
use crate::hir::{HirExprKind, HirStmt, TyKind};

#[test]
fn method_lowers_to_function_with_this_param() {
    let hir = hir_of(
        "class Point {\n    x: i32\n    y: i32\n    fun sum(): i32 {\n        return this.x + this.y\n    }\n}\nfun main(): i32 {\n    return 0\n}",
    );
    let names: Vec<_> = hir.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["main", "Point.sum"]);
    let method = &hir.functions[1];
    assert_eq!(method.params[0].name, "this");
    assert!(matches!(&method.params[0].ty.kind, TyKind::Ref(inner) if inner.record_name() == Some("Point")));
    let HirStmt::Return { value: Some(value) } = &method.body.stmts[0] else {
        panic!("expected return");
    };
    let HirExprKind::Binary { lhs, .. } = &value.kind else { panic!("expected +") };
    assert!(matches!(&lhs.kind, HirExprKind::ObjectField { field_index: 0, .. }));
}

#[test]
fn method_param_may_shadow_field() {
    hir_of("class P {\n    x: i32\n    fun set(x: i32) {\n        this.x = x\n    }\n}\nfun main() {}");
}

#[test]
fn this_outside_method_is_error() {
    let diags = diags_of("fun main(): i32 {\n    return this.x\n}");
    assert!(diags.iter().any(|d| d.message.contains("`this` is only available inside a class method")), "{diags:?}");
}

#[test]
fn assign_this_is_error() {
    let diags = diags_of("class P {\n    x: i32\n    fun f(other: &P) {\n        this = other\n    }\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("cannot assign to `this`")), "{diags:?}");
}

#[test]
fn move_this_is_error() {
    let diags = diags_of("class P {\n    x: i32\n    fun f() {\n        val q = move this\n    }\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("cannot move `this`; it is a borrow")), "{diags:?}");
}

#[test]
fn method_named_like_field_is_error() {
    let diags = diags_of("class P {\n    x: i32\n    fun x(): i32 {\n        return 1\n    }\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("class `P` already has a field named `x`")), "{diags:?}");
}

#[test]
fn duplicate_method_is_error() {
    let diags = diags_of("class P {\n    fun f() {}\n    fun f() {}\n}\nfun main() {}");
    assert!(diags.iter().any(|d| d.message.contains("duplicate method `f` in class `P`")), "{diags:?}");
}
