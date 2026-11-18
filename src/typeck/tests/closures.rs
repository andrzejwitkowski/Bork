use crate::diag::Phase;

use super::{diags_of, hir_of};

#[test]
fn trailing_closure_arg_count() {
    let src = r#"
fun action(a: i32, b: i32, block: (i32, i32) -> i32): i32 {
    return block(a, b)
}
fun main(): i32 {
    return action(1, 2) { x -> x }
}
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn closure_expression_takes_its_signature_from_the_annotation() {
    let src = r#"
fun main(): i32 {
    val f: (i32) -> i32 = { x -> x + 1 }
    return f(2)
}
"#;
    hir_of(src);
}

#[test]
fn closure_expression_passes_where_a_function_is_expected() {
    let src = r#"
fun twice(f: (i32) -> i32, v: i32): i32 {
    return f(f(v))
}
fun main(): i32 {
    return twice({ x -> x * 2 }, 3)
}
"#;
    hir_of(src);
}

#[test]
fn closure_expression_body_must_match_the_expected_result() {
    let src = r#"
fun main() {
    val f: (i32) -> i32 = { x -> true }
}
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn closure_expression_arity_must_match_the_expected_function() {
    let src = r#"
fun main() {
    val f: (i32, i32) -> i32 = { x -> x }
}
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn fn_bound_lets_a_generic_function_call_its_parameter() {
    let src = r#"
fun apply<F: Fn(i32) -> i32>(f: F, v: i32): i32 {
    return f(v)
}
fun main(): i32 {
    return apply({ x -> x + 1 }, 2)
}
"#;
    hir_of(src);
}

#[test]
fn fn_bound_accepts_a_function_typed_argument() {
    let src = r#"
fun apply<F: Fn(i32) -> i32>(f: F, v: i32): i32 {
    return f(v)
}
fun twice(g: (i32) -> i32): i32 {
    return apply(g, 1)
}
"#;
    hir_of(src);
}

#[test]
fn fn_bound_rejects_a_closure_with_another_signature() {
    let src = r#"
fun apply<F: Fn(i32) -> i32>(f: F, v: i32): i32 {
    return f(v)
}
fun main(): i32 {
    return apply({ x -> true }, 2)
}
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn fn_bound_works_with_a_trailing_closure() {
    let src = r#"
fun apply<F: FnMut(i32) -> i32>(v: i32, f: F): i32 {
    return f(v)
}
fun main(): i32 {
    return apply(2) { x -> x * 3 }
}
"#;
    hir_of(src);
}

#[test]
fn closure_kind_follows_how_the_body_uses_a_capture() {
    // Reads only: satisfies Fn. Assigns: FnMut but not Fn. Moves out: FnOnce only.
    let reads = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    val n = 1\n    return apply({ -> n })\n}";
    hir_of(reads);
    let writes = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 1\n    return apply({ -> n = 2\n 0 })\n}";
    assert!(diags_of(writes).iter().any(|d| d.message.contains("FnMut")), "{:?}", diags_of(writes));
    let writes_ok = "fun apply<F: FnMut() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 1\n    return apply({ -> n = 2\n 0 })\n}";
    hir_of(writes_ok);
}

#[test]
fn closure_expression_records_its_captures() {
    use crate::hir::HirExprKind;
    let src = "fun main(): i32 {\n    val a = 1\n    val b = 2\n    val f: () -> i32 = { -> a + 1 }\n    return f()\n}";
    let hir = hir_of(src);
    let found = hir.functions[0].body.stmts.iter().find_map(|s| match s {
        crate::hir::HirStmt::VarDecl { value, name, .. } if name == "f" => Some(value),
        _ => None,
    });
    let HirExprKind::Closure { captures, .. } = &found.expect("val f").kind else { panic!("closure") };
    assert_eq!(captures.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["a"]);
}

#[test]
fn plain_closure_that_captures_is_a_view_and_cannot_be_returned() {
    let src = "fun make(): () -> i32 {\n    val n = 1\n    return { -> n }\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("cannot return a closure that borrows")), "{:?}", diags_of(src));
}

#[test]
fn plain_closure_without_captures_may_be_returned() {
    hir_of("fun make(): () -> i32 {\n    return { -> 1 }\n}");
}

#[test]
fn a_view_closure_cannot_be_stored_in_a_var_or_assigned_to_an_outer_one() {
    let init = "fun main() {\n    val n = 1\n    var keep = { -> n }\n}";
    assert!(diags_of(init).iter().any(|d| d.message.contains("borrows its captures")));
    let assign = "fun main() {\n    var keep: () -> i32 = { -> 0 }\n    var n = 5\n    keep = { -> n }\n}";
    assert!(diags_of(assign).iter().any(|d| d.message.contains("borrows its captures")));
}

#[test]
fn closures_without_borrowed_captures_may_be_stored_in_a_var() {
    hir_of("fun main() {\n    var keep: () -> i32 = { -> 0 }\n    keep = { -> 1 }\n}");
    hir_of("fun main() {\n    val n = 3\n    var keep = move (n) { -> n }\n}");
}

#[test]
fn a_nested_closure_that_assigns_makes_the_outer_one_fn_mut() {
    let src = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 1\n    return apply({ -> val inner = { -> n = n + 1\n n }\n inner() })\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("FnMut")), "{:?}", diags_of(src));
}

#[test]
fn break_inside_a_closure_does_not_reach_a_loop_outside_it() {
    let src = "fun run(f: () -> i32): i32 {\n    return f()\n}\nfun main() {\n    var i = 0\n    while (i < 3) {\n        run() { -> break\n 1 }\n        i = i + 1\n    }\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("break")), "{:?}", diags_of(src));
}

#[test]
fn an_outer_name_used_before_a_same_named_declaration_is_still_captured() {
    use crate::hir::HirExprKind;
    let src = "fun main(): i32 {\n    val x = 3\n    val s: (i32) -> i32 = { y -> val z = x + y\n val x = 10\n x + z }\n    return s(1)\n}";
    let hir = hir_of(src);
    let value = hir.functions[0].body.stmts.iter().find_map(|s| match s {
        crate::hir::HirStmt::VarDecl { value, name, .. } if name == "s" => Some(value),
        _ => None,
    });
    let HirExprKind::Closure { captures, .. } = &value.expect("val s").kind else { panic!("closure") };
    assert_eq!(captures.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), vec!["x"]);
}

#[test]
fn a_move_capture_list_names_exactly_what_the_body_uses() {
    let unlisted = "fun main() {\n    val n = 1\n    val m = 2\n    val f = move (n) { -> n + m }\n}";
    assert!(diags_of(unlisted).iter().any(|d| d.message.contains("`m` is used by the closure but not listed")));
    let unused = "fun main() {\n    val n = 1\n    val m = 2\n    val f = move (n, m) { -> n }\n}";
    assert!(diags_of(unused).iter().any(|d| d.message.contains("`m` is listed")));
    hir_of("fun main() {\n    val n = 1\n    val m = 2\n    val f = move (n, m) { -> n + m }\n}");
}

#[test]
fn a_view_closure_stays_a_view_when_annotated_as_a_function_type() {
    let moved = "fun main() {\n    var keep: () -> i32 = { -> 0 }\n    var n = 5\n    val h: () -> i32 = { -> n }\n    keep = move h\n}";
    assert!(diags_of(moved).iter().any(|d| d.message.contains("borrows its captures")), "{:?}", diags_of(moved));
    let returned = "fun make(): () -> i32 {\n    val n = 1\n    val h: () -> i32 = { -> n }\n    return h\n}";
    assert!(diags_of(returned).iter().any(|d| d.message.contains("cannot return a closure that borrows")), "{:?}", diags_of(returned));
    hir_of("fun main() {\n    val n = 5\n    val h: () -> i32 = { -> n }\n    val g = h\n}");
}

#[test]
fn a_local_closure_parameter_shadows_a_global_function_of_the_same_name() {
    use crate::hir::{HirExprKind, UseKind};
    let src = "fun f(x: i32): i32 {\n    return x + 100\n}\nfun apply(f: (i32) -> i32): i32 {\n    return f(1)\n}";
    let hir = hir_of(src);
    let apply = hir.functions.iter().find(|f| f.name == "apply").expect("apply");
    let crate::hir::HirStmt::Return { value: Some(call) } = &apply.body.stmts[0] else { panic!("return") };
    let HirExprKind::Call { callee, .. } = &call.kind else { panic!("call") };
    assert!(matches!(&callee.kind, HirExprKind::Ident { use_kind: UseKind::Copy, .. }), "{:?}", callee.kind);
}

#[test]
fn an_unconstrained_closure_parameter_is_a_diagnostic_not_a_panic() {
    let src = "fun main() {\n    val f = { unused -> 1 }\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("closure parameter `unused`")));
}

#[test]
fn an_fn_bound_satisfies_a_weaker_fn_mut_bound() {
    hir_of("fun run<G: FnMut() -> i32>(g: G): i32 {\n    return g()\n}\nfun forward<F: Fn() -> i32>(f: F): i32 {\n    return run(f)\n}");
}

#[test]
fn moving_a_copy_capture_is_only_a_read() {
    hir_of("fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    val n: i32 = 1\n    return apply({ -> move n })\n}");
}

#[test]
fn a_view_closure_cannot_be_stored_in_an_enum_payload_or_returned_through_a_call() {
    let some = "fun main() {\n    val n = 3\n    val slot = Some({ -> n })\n}";
    assert!(diags_of(some).iter().any(|d| d.message.contains("borrows its captures")), "{:?}", diags_of(some));
    let forwarded = "fun forward(f: () -> i32): () -> i32 {\n    return f\n}\nfun make(): () -> i32 {\n    val n = 5\n    return forward({ -> n })\n}";
    assert!(diags_of(forwarded).iter().any(|d| d.message.contains("cannot return a closure that borrows")), "{:?}", diags_of(forwarded));
}

#[test]
fn an_editable_borrow_of_a_capture_makes_the_closure_fn_mut() {
    let src = "fun bump(a: &rw [i32; 1]) {\n    a[0] = a[0] + 1\n}\nfun apply<F: Fn() -> unit>(f: F) {\n    f()\n}\nfun main() {\n    var a = [1]\n    apply({ -> bump(&rw a) })\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("FnMut")), "{:?}", diags_of(src));
}

#[test]
fn a_trailing_closure_must_match_the_type_an_earlier_argument_fixed() {
    let src = "fun both<F: Fn() -> i32>(a: F, b: F): i32 {\n    return a() + b()\n}\nfun main(): i32 {\n    return both({ -> 1 }) { -> 2 }\n}";
    assert!(!diags_of(src).is_empty());
}
