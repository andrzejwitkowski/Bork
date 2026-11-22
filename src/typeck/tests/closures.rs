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
fn a_trailing_closure_is_checked_as_the_calls_last_argument() {
    use crate::hir::{HirExprKind, HirStmt};
    let hir = hir_of("fun apply(v: i32, f: (i32) -> i32): i32 {\n    return f(v)\n}\nfun main(): i32 {\n    return apply(2) { x -> x + 1 }\n}");
    let main = hir.functions.iter().find(|f| f.name == "main").expect("main");
    let Some(HirStmt::Return { value: Some(call) }) = main.body.stmts.last() else { panic!("{:?}", main.body.stmts) };
    let HirExprKind::Call { args, .. } = &call.kind else { panic!("{call:?}") };
    assert_eq!(args.len(), 2);
    assert!(matches!(args[1].kind, HirExprKind::Closure { .. }), "{:?}", args[1].kind);
}

#[test]
fn a_trailing_closure_is_one_argument_too_many_for_a_call_without_a_function_parameter() {
    let src = "fun add(a: i32, b: i32): i32 {\n    return a + b\n}\nfun main(): i32 {\n    return add(1, 2) { x -> x }\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("expects 2 arguments, got 3")), "{:?}", diags_of(src));
}

#[test]
fn a_trailing_closure_is_checked_against_the_last_parameters_type() {
    let src = "fun apply(v: i32, f: (i32) -> i32): i32 {\n    return f(v)\n}\nfun main(): i32 {\n    return apply(2) { x -> true }\n}";
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type), "{:?}", diags_of(src));
}

#[test]
fn trailing_and_positional_closures_agree_on_kind_and_move_captures() {
    let hdr = "fun run<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun runMut<F: FnMut() -> i32>(f: F): i32 {\n    return f()\n}\nfun runOnce<F: FnOnce() -> i32>(f: F): i32 {\n    return f()\n}\n";
    let main = |body: &str| format!("{hdr}fun main(): i32 {{\n    var n = 1\n    {body}\n}}");
    // A writing closure is FnMut by either syntax, and not Fn by either.
    for call in ["return runMut({ -> n = 2\n 0 })", "return runMut() { -> n = 2\n 0 }"] {
        hir_of(&main(call));
    }
    for call in ["return run({ -> n = 2\n 0 })", "return run() { -> n = 2\n 0 }"] {
        assert!(diags_of(&main(call)).iter().any(|d| d.message.contains("FnMut")), "{call}");
    }
    // A reading closure satisfies all three, and a `move` one owns its capture.
    for call in ["return runOnce({ -> n })", "return runOnce() { -> n }", "return run(move (n) { -> n })", "return run() move (n) { -> n }"] {
        hir_of(&main(call));
    }
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

#[test]
fn a_closure_calling_a_captured_fn_mut_is_fn_mut() {
    let src = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 0\n    val inner = { -> n = n + 1\n n }\n    return apply({ -> inner() })\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("FnMut")), "{:?}", diags_of(src));
}

#[test]
fn moving_an_unannotated_literal_capture_is_a_read() {
    let src = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    val n = 1\n    return apply({ -> move n })\n}";
    hir_of(src);
}

#[test]
fn a_trailing_closure_that_borrows_is_a_view_when_returned() {
    let src = "fun forward(f: () -> i32): () -> i32 {\n    return f\n}\nfun make(): () -> i32 {\n    val n = 5\n    return forward() { -> n }\n}";
    assert!(!diags_of(src).is_empty());
}

#[test]
fn calling_an_alias_of_a_captured_fn_mut_is_fn_mut() {
    let src = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 0\n    val inner = { -> n = n + 1\n n }\n    return apply({ -> val alias = inner\n alias() })\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("FnMut")), "{:?}", diags_of(src));
}

#[test]
fn an_annotated_callable_keeps_its_kind() {
    let hdr = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 0\n";
    // `() -> i32` means `Fn`: a closure that assigns to a capture does not fit it, `FnMut() -> i32` does.
    let plain = format!("{hdr}    val inner: () -> i32 = {{ -> n = n + 1\n n }}\n    return apply({{ -> inner() }})\n}}");
    assert!(diags_of(&plain).iter().any(|d| d.message.contains("initializer for `inner`")), "{:?}", diags_of(&plain));
    let mutable = format!("{hdr}    val inner: FnMut() -> i32 = {{ -> n = n + 1\n n }}\n    return apply({{ -> inner() }})\n}}");
    assert!(diags_of(&mutable).iter().any(|d| d.message.contains("FnMut")), "{:?}", diags_of(&mutable));
    let reads = "fun main(): i32 {\n    val n = 1\n    val f: () -> i32 = { -> n }\n    val g: FnMut() -> i32 = { -> n }\n    return f() + g()\n}";
    hir_of(reads);
}

#[test]
fn an_alias_shadowed_in_a_nested_block_is_back_after_it() {
    let src = "fun apply<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 0\n    val inner = { -> n = n + 1\n n }\n    return apply({ -> val alias = inner\n if (true) {\n val alias = 0\n }\n alias() })\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("FnMut")), "{:?}", diags_of(src));
}

#[test]
fn a_func_parameter_may_hold_a_view_so_the_callee_cannot_return_or_branch_to_it() {
    let forward = "fun forward(f: () -> i32): () -> i32 {\n    return f\n}\nfun main(): i32 {\n    return 0\n}";
    assert!(diags_of(forward).iter().any(|d| d.message.contains("cannot return")), "{:?}", diags_of(forward));
    let branch = "fun pick(c: bool, a: () -> i32, b: () -> i32): () -> i32 {\n    return if (c) { a } else { b }\n}\nfun main(): i32 {\n    return 0\n}";
    assert!(diags_of(branch).iter().any(|d| d.message.contains("cannot return")), "{:?}", diags_of(branch));
    let tail = "fun main(): i32 {\n    val n = 1\n    val outer = { -> { -> n } }\n    return 0\n}";
    assert!(diags_of(tail).iter().any(|d| d.message.contains("cannot return")), "{:?}", diags_of(tail));
    // Calling the parameter is fine.
    hir_of("fun run(f: () -> i32): i32 {\n    return f()\n}\nfun main(): i32 {\n    val n = 1\n    return run({ -> n })\n}");
}

#[test]
fn an_owned_branch_alias_is_not_mistaken_for_an_outer_view_parameter_of_the_same_name() {
    let src = "fun pick(c: bool, f: () -> i32): () -> i32 {\n    return if (c) {\n        val f: () -> i32 = move { -> 1 }\n        val g = f\n        g\n    } else {\n        move { -> 2 }\n    }\n}\nfun main(): i32 {\n    return 0\n}";
    assert!(!diags_of(src).iter().any(|d| d.message.contains("borrows its captures")), "{:?}", diags_of(src));
}

#[test]
fn a_view_alias_is_rejected_through_nested_branch_tails() {
    let src = "fun pick(c: bool, d: bool, f: () -> i32): () -> i32 {\n    return if (c) {\n        val g = f\n        if (d) { g } else { g }\n    } else {\n        move { -> 0 }\n    }\n}\nfun main(): i32 {\n    return 0\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("cannot return")), "{:?}", diags_of(src));
}

#[test]
fn a_long_chain_of_owned_branch_aliases_is_checked_in_linear_time() {
    let src = "fun pick(c: bool, f: () -> i32): () -> i32 {\n    return if (c) {\n        val g0: () -> i32 = move { -> 1 }\n        val g1 = if (c) { g0 } else { g0 }\n        val g2 = if (c) { g1 } else { g1 }\n        val g3 = if (c) { g2 } else { g2 }\n        val g4 = if (c) { g3 } else { g3 }\n        val g5 = if (c) { g4 } else { g4 }\n        val g6 = if (c) { g5 } else { g5 }\n        val g7 = if (c) { g6 } else { g6 }\n        val g8 = if (c) { g7 } else { g7 }\n        val g9 = if (c) { g8 } else { g8 }\n        val g10 = if (c) { g9 } else { g9 }\n        val g11 = if (c) { g10 } else { g10 }\n        val g12 = if (c) { g11 } else { g11 }\n        val g13 = if (c) { g12 } else { g12 }\n        val g14 = if (c) { g13 } else { g13 }\n        val g15 = if (c) { g14 } else { g14 }\n        val g16 = if (c) { g15 } else { g15 }\n        val g17 = if (c) { g16 } else { g16 }\n        val g18 = if (c) { g17 } else { g17 }\n        val g19 = if (c) { g18 } else { g18 }\n        val g20 = if (c) { g19 } else { g19 }\n        val g21 = if (c) { g20 } else { g20 }\n        val g22 = if (c) { g21 } else { g21 }\n        val g23 = if (c) { g22 } else { g22 }\n        val g24 = if (c) { g23 } else { g23 }\n        val g25 = if (c) { g24 } else { g24 }\n        g25\n    } else {\n        move { -> 2 }\n    }\n}\nfun main(): i32 {\n    return 0\n}";
    assert!(!diags_of(src).iter().any(|d| d.message.contains("borrows its captures")), "{:?}", diags_of(src));
}

#[test]
fn a_move_block_inside_a_closure_moves_its_capture_so_the_closure_is_fn_once() {
    let hdr = "class Node {\n    id: i32\n}\nfun once<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    val n = Node(1)\n";
    let moves = format!("{hdr}    return once({{ ->\n        move (n) {{ println(\"x\") }}\n        0\n    }})\n}}");
    assert!(diags_of(&moves).iter().any(|d| d.message.contains("FnOnce")), "{:?}", diags_of(&moves));
}

#[test]
fn building_a_nested_move_closure_moves_its_captures_so_the_outer_closure_is_fn_once() {
    let src = "class Node {\n    id: i32\n}\nfun once<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    val n = Node(1)\n    return once({ ->\n        val g = move (n) { -> n.id }\n        g()\n    })\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("FnOnce")), "{:?}", diags_of(src));
}

#[test]
fn a_move_block_without_a_capture_list_moves_what_it_uses_so_the_closure_is_fn_once() {
    let src = "class Node {\n    id: i32\n}\nfun once<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    val n = Node(1)\n    return once({ ->\n        move {\n            val id = n.id\n        }\n        0\n    })\n}";
    assert!(diags_of(src).iter().any(|d| d.message.contains("FnOnce")), "{:?}", diags_of(src));
}

#[test]
fn a_nested_closure_that_writes_only_its_own_local_leaves_the_outer_closure_fn() {
    let src = "fun once<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    val y = 1\n    return once({ ->\n        var x = 0\n        val g = { -> x = x + 1\n            y }\n        g() })\n}";
    hir_of(src);
}

#[test]
fn a_nested_move_closure_that_writes_its_copy_leaves_the_outer_closure_fn() {
    let src = "fun once<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 1\n    return once({ ->\n        var g = move (n) { -> n = n + 1\n            n }\n        g() })\n}";
    hir_of(src);
}

#[test]
fn a_nested_closure_parameter_shadows_an_outer_alias() {
    let src = "fun once<F: Fn() -> i32>(f: F): i32 {\n    return f()\n}\nfun main(): i32 {\n    var n = 0\n    val f = { -> n = n + 1\n        n }\n    return once({ ->\n        val h = f\n        val g: (() -> i32) -> i32 = { h -> h() }\n        0 })\n}";
    hir_of(src);
}
