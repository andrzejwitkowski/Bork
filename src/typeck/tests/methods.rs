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

use crate::ast::UnaryOp;
use crate::hir::HirExpr;

fn call_in_return(hir: &crate::hir::HirProgram, function: &str) -> HirExpr {
    let f = hir.functions.iter().find(|f| f.name == function).expect("function");
    let HirStmt::Return { value: Some(value) } = f.body.stmts.last().unwrap() else {
        panic!("expected return");
    };
    value.clone()
}

#[test]
fn receiver_call_lowers_to_qualified_call_with_borrowed_receiver() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun add(k: i32): i32 {\n        return this.x + k\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.add(2)\n}",
    );
    let call = call_in_return(&hir, "main");
    let HirExprKind::Call { callee, args, .. } = &call.kind else { panic!("call") };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "P.add"));
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[0].kind, HirExprKind::Unary { op: UnaryOp::Borrow, .. }));
    assert!(args[0].ty.is_ref());
}

#[test]
fn bare_and_this_calls_resolve_to_method() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun one(): i32 {\n        return 1\n    }\n    fun two(): i32 {\n        return one() + this.one()\n    }\n}\nfun main() {}",
    );
    let HirExprKind::Binary { lhs, rhs, .. } = &call_in_return(&hir, "P.two").kind else { panic!("+") };
    for side in [lhs, rhs] {
        let HirExprKind::Call { callee, args, .. } = &side.kind else { panic!("call") };
        assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "P.one"));
        assert_eq!(args.len(), 1, "only the receiver");
    }
}

#[test]
fn method_wins_over_top_level_function_inside_class() {
    let hir = hir_of(
        "fun one(): i32 {\n    return 9\n}\nclass P {\n    fun one(): i32 {\n        return 1\n    }\n    fun two(): i32 {\n        return one()\n    }\n}\nfun main(): i32 {\n    return one()\n}",
    );
    let HirExprKind::Call { callee, .. } = &call_in_return(&hir, "P.two").kind else { panic!() };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "P.one"));
    let HirExprKind::Call { callee, .. } = &call_in_return(&hir, "main").kind else { panic!() };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "one"));
}

#[test]
fn borrowed_and_field_chain_receivers_are_accepted() {
    hir_of(
        "class In {\n    n: i32\n    fun get(): i32 {\n        return this.n\n    }\n}\nclass Out {\n    inner: In\n    fun deep(): i32 {\n        return this.inner.get()\n    }\n}\nfun via(o: &Out): i32 {\n    return o.inner.get() + o.deep()\n}\nfun main(): i32 {\n    val o = Out(In(1))\n    return via(&o) + o.inner.get()\n}",
    );
}

#[test]
fn method_call_receiver_errors() {
    let cases = [
        ("class P {\n    fun f() {}\n}\nfun main() {\n    val r: Ref<P> = None\n    r.f()\n}", "method call on managed Ref requires `if val` or `when`"),
        ("class P {\n    fun f() {}\n}\nfun g(p: P?) {\n    p.f()\n}\nfun main() {}", "method call on a nullable receiver"),
        ("class P {\n    fun f() {}\n}\nfun main() {\n    P().f()\n}", "method receiver must be a named value; bind it to a `val` first"),
        ("class P {\n    fun f() {}\n}\nfun main() {\n    val p = P()\n    p.g()\n}", "class `P` has no method `g`"),
        ("class P {\n    fun f(a: i32) {}\n}\nfun main() {\n    val p = P()\n    p.f()\n}", "function `P.f` expects 1 arguments, got 0"),
    ];
    for (src, expected) in cases {
        let diags = diags_of(src);
        assert!(diags.iter().any(|d| d.message.contains(expected)), "{expected}: {diags:?}");
    }
}
