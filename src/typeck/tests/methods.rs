use super::{checked_of, diags_of, hir_of};
use crate::hir::{HirExprKind, HirStmt};

#[test]
fn method_lowers_to_function_with_this_param() {
    let (hir, tcx) = checked_of(
        "class Point {\n    x: i32\n    y: i32\n    fun sum(): i32 {\n        return this.x + this.y\n    }\n}\nfun main(): i32 {\n    return 0\n}",
    );
    let names: Vec<_> = hir.functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["main", "Point.sum"]);
    let method = &hir.functions[1];
    assert_eq!(method.params[0].name, "this");
    assert_eq!(tcx.display(method.params[0].ty).to_string(), "&Point");
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
    let (hir, tcx) = checked_of(
        "class P {\n    x: i32\n    fun add(k: i32): i32 {\n        return this.x + k\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.add(2)\n}",
    );
    let call = call_in_return(&hir, "main");
    let HirExprKind::Call { callee, args, .. } = &call.kind else { panic!("call") };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "P.add"));
    assert_eq!(args.len(), 2);
    assert!(matches!(&args[0].kind, HirExprKind::Unary { op: UnaryOp::Borrow, .. }));
    assert!(tcx.is_ref(args[0].ty));
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

#[test]
fn bare_field_reads_and_writes_go_through_this() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun bump(): i32 {\n        x = x + 1\n        return x\n    }\n}\nfun main() {}",
    );
    let method = hir.functions.iter().find(|f| f.name == "P.bump").unwrap();
    let HirStmt::Assign { target, .. } = &method.body.stmts[0] else { panic!("assign") };
    assert!(matches!(target, crate::hir::HirAssignTarget::Field { field_index: 0, .. }));
    let HirExprKind::ObjectField { receiver, .. } = &call_in_return(&hir, "P.bump").kind else {
        panic!("bare x reads the field");
    };
    assert!(matches!(&receiver.kind, HirExprKind::Ident { name, .. } if name == "this"));
}

#[test]
fn bare_field_then_local_shadow() {
    let hir = hir_of(
        "class P {\n    x: i32\n    fun f(): i32 {\n        val a = x\n        val x = 5\n        return a + x\n    }\n}\nfun main() {}",
    );
    let method = hir.functions.iter().find(|f| f.name == "P.f").unwrap();
    let HirStmt::VarDecl { value, .. } = &method.body.stmts[0] else { panic!("val a") };
    assert!(matches!(&value.kind, HirExprKind::ObjectField { .. }), "before the local: field");
    let HirExprKind::Binary { rhs, .. } = &call_in_return(&hir, "P.f").kind else { panic!("+") };
    assert!(matches!(&rhs.kind, HirExprKind::Ident { name, .. } if name == "x"), "after the local: local");
}

#[test]
fn bare_field_outside_method_is_unknown() {
    let diags = diags_of("class P {\n    x: i32\n}\nfun main(): i32 {\n    return x\n}");
    assert!(diags.iter().any(|d| d.message.contains("unknown binding `x`")), "{diags:?}");
}

#[test]
fn param_shadows_field() {
    let hir = hir_of("class P {\n    x: i32\n    fun f(x: i32): i32 {\n        return x\n    }\n}\nfun main() {}");
    assert!(matches!(&call_in_return(&hir, "P.f").kind, HirExprKind::Ident { name, .. } if name == "x"));
}

#[test]
fn bare_field_in_name_only_positions_gets_targeted_error() {
    let class = "class Inner {\n    n: i32\n}\nfun look(i: &Inner): i32 {\n    return 1\n}\nclass P {\n    s: String\n    inner: Inner\n    items: [i32; 2]\n    fun apply(k: i32, f: (i32) -> i32): i32 {\n        return f(k)\n    }\n    fun f(): i32 {\n        ";
    let tail = "\n        return 0\n    }\n}\nfun main() {}";
    let cases = [
        ("val a = move s", "cannot move field `s`"),
        ("val a = promote s", "cannot promote field `s`"),
        ("val a = look(&inner)", "cannot borrow field `inner`"),
        ("items[0] = 5", "cannot index-assign field `items`"),
        ("move(s) {\n            val y = 1\n        }", "cannot capture field `s`"),
        ("val b = apply(1) move(s) { v -> v }", "cannot capture field `s`"),
    ];
    for (body, expected) in cases {
        let diags = diags_of(&format!("{class}{body}{tail}"));
        assert!(diags.iter().any(|d| d.message.contains(expected)), "{expected}: {diags:?}");
        assert!(!diags.iter().any(|d| d.message.contains("unknown binding")), "{body}: {diags:?}");
        assert_eq!(diags.len(), 1, "{body}: {diags:?}");
    }
}
