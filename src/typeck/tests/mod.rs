use crate::diag::{Diagnostic, Phase};
use crate::frontend::check;
use crate::hir::{HirExpr, HirExprKind, HirProgram, HirStmt, Prim, Ty, UseKind};

mod closures;
mod nullable;
mod managed_ref;
mod reference;

/// Typed HIR for a source that must check cleanly.
fn hir_of(source: &str) -> HirProgram {
    let result = check(source);
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
    result.hir.expect("clean check produces HIR")
}

/// Diagnostics for a source that must not check cleanly.
fn diags_of(source: &str) -> Vec<Diagnostic> {
    let result = check(source);
    assert!(!result.diagnostics.is_empty(), "expected diagnostics");
    assert!(result.hir.is_none(), "HIR is withheld when checks fail");
    result.diagnostics
}

#[test]
fn assign_to_val_is_type_error() {
    let src = r#"
fun main(): i32 {
    val x: i32 = 1
    x = 2
    return x
}
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn return_type_mismatch() {
    let src = r#"
fun main(): i32 {
    return "nope"
}
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn adds_i32() {
    assert!(check(r#"fun main(): i32 { return 1 + 2 }"#).is_ok());
}

#[test]
fn rejects_add_string_int() {
    let errors = diags_of(r#"fun main(): i32 { return 1 + "a" }"#);
    assert!(errors.iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn if_cond_must_be_bool() {
    let src = r#"fun main(): i32 { if (1) { return 1 } else { return 0 } }"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn checks_if_blocks_and_infers_local_binary_type() {
    let src = r#"
fun main(): i32 {
    val x = 1 + 2
    if (x > 0) { return x } else { return 0 }
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn call_arity_mismatch() {
    let src = r#"
fun f(x: i32): i32 { return x }
fun main(): i32 { return f() }
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn call_arg_type_mismatch() {
    let src = r#"
fun f(x: i32): i32 { return x }
fun main(): i32 { return f("a") }
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn call_ok() {
    let src = r#"
fun f(x: i32): i32 { return x + 1 }
fun main(): i32 { return f(41) }
"#;
    assert!(check(src).is_ok());
}

#[test]
fn print_i32_typechecks() {
    let src = r#"
fun main(): i32 {
    println(42)
    return 0
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn print_i64_typechecks() {
    let src = r#"
fun main(): i32 {
    val value: i64 = 42
    print(value)
    return 0
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn println_string_typechecks() {
    let src = r#"
fun main(): i32 {
    println("hello")
    return 0
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn print_unknown_name_still_errors() {
    let src = r#"
fun main(): i32 {
    printlnn(1)
    return 0
}
"#;
    assert!(check(src)
        .diagnostics
        .iter()
        .any(|d| d.phase == Phase::Type));
}

#[test]
fn for_range_ok() {
    let src = r#"
fun main(): i32 {
    var a: i32 = 0
    for (i in 0..3) {
        a = a + i
    }
    return a
}
"#;
    let hir = hir_of(src);
    let HirStmt::For { name, iter, body } = &hir.functions[0].body.stmts[1] else {
        panic!(
            "expected a for loop, got {:?}",
            hir.functions[0].body.stmts[1]
        );
    };
    assert_eq!(name, "i");
    assert_eq!(iter.ty, Ty::range(Ty::i32()));
    assert!(matches!(body.stmts.as_slice(), [HirStmt::Assign { .. }]));
}

#[test]
fn move_expr_preserves_type_and_use_kind() {
    let src = r#"
fun main() {
    var s: String = "hi"
    val t = move s
    println(t)
}
"#;
    let hir = hir_of(src);
    assert!(matches!(
        &hir.functions[0].body.stmts[1],
        HirStmt::VarDecl {
            value: HirExpr {
                kind: HirExprKind::Ident {
                    use_kind: UseKind::Move,
                    ..
                },
                ..
            },
            ..
        }
    ));
}

#[test]
fn parent_val_string_read_in_nested_block_is_shared() {
    let src = r#"
fun main(): i32 {
    val s: String = "hi"
    {
        s
    }
    return 0
}
"#;
    let hir = hir_of(src);
    let HirStmt::Block(block) = &hir.functions[0].body.stmts[1] else {
        panic!("expected a nested block");
    };
    assert!(matches!(
        block.stmts.as_slice(),
        [HirStmt::Expr(HirExpr {
            kind: HirExprKind::Ident {
                use_kind: UseKind::Shared,
                ..
            },
            ..
        })]
    ));
}

#[test]
fn var_string_read_is_local() {
    let src = r#"
fun main(): i32 {
    var s: String = "hi"
    s
    return 0
}
"#;
    let hir = hir_of(src);
    assert!(matches!(
        &hir.functions[0].body.stmts[1],
        HirStmt::Expr(HirExpr {
            kind: HirExprKind::Ident {
                use_kind: UseKind::Local,
                ..
            },
            ..
        })
    ));
}

#[test]
fn return_move_string_is_move_at_typeck() {
    let src = r#"
fun main(): String {
    val s: String = "hi"
    return move s
}
"#;
    let program = crate::parse(src).expect("parse");
    let (_, sema_errors) = crate::sema::analyze(&program);
    assert!(sema_errors.is_empty(), "{sema_errors:?}");
    let (hir, _, type_errors) = crate::typeck::check(&program);
    assert!(type_errors.is_empty(), "{type_errors:?}");
    assert!(matches!(
        &hir.functions[0].body.stmts[1],
        HirStmt::Return {
            value: Some(HirExpr {
                kind: HirExprKind::Ident {
                    use_kind: UseKind::Move,
                    ..
                },
                ..
            })
        }
    ));
}

#[test]
fn hir_exprs_carry_their_type_and_span() {
    let src = r#"
fun main(): i32 {
    val x: i32 = 1
    return x
}
"#;
    let hir = hir_of(src);
    let stmts = &hir.functions[0].body.stmts;
    let HirStmt::VarDecl { value, .. } = &stmts[0] else {
        panic!("expected a var declaration, got {:?}", stmts[0]);
    };
    assert_eq!(value.ty, Ty::i32());

    let HirStmt::Return { value: Some(value) } = &stmts[1] else {
        panic!("expected a return, got {:?}", stmts[1]);
    };
    assert_eq!(value.ty, Ty::i32());
    let span = value.span.expect("identifiers carry their source span");
    assert_eq!(&src[span.start..span.end], "x");
}

#[test]
fn regional_move_block_is_typechecked() {
    let src = r#"
fun main(): i32 {
    val s: String = "hi"
    move (s) {
        val len: i32 = s.length
    }
    return 0
}
"#;
    let hir = hir_of(src);
    let function = &hir.functions[0];
    let HirStmt::MoveBlock { captures, body } = &function.body.stmts[1] else {
        panic!("expected a move block, got {:?}", function.body.stmts[1]);
    };
    assert_eq!(captures.as_deref(), Some(["s".to_string()].as_slice()));
    assert!(matches!(
        body.stmts.as_slice(),
        [HirStmt::VarDecl { name, .. }] if name == "len"
    ));
}

#[test]
fn type_error_inside_move_block_is_reported() {
    let src = r#"
fun main(): i32 {
    val s: String = "hi"
    move (s) {
        val len: i32 = s
    }
    return 0
}
"#;
    let errors = diags_of(src);
    assert!(
        errors
            .iter()
            .any(|error| error.phase == Phase::Type && error.message.contains("initializer")),
        "{errors:?}"
    );
}

#[test]
fn value_if_without_annotation_infers_from_branches() {
    let src = r#"
fun main(n: i32): i32 {
    val picked = if (n > 0) { n } else { 0 }
    return picked
}
"#;
    let hir = hir_of(src);
    assert!(matches!(
        &hir.functions[0].body.stmts[0],
        HirStmt::VarDecl { ty, .. } if *ty == Ty::i32()
    ));
}

#[test]
fn int_literal_adopts_expected_integer_type() {
    let src = r#"
fun main(): i64 {
    return 1
}
"#;
    let hir = hir_of(src);
    let HirStmt::Return { value: Some(value) } = &hir.functions[0].body.stmts[0] else {
        panic!("expected a return");
    };
    assert_eq!(value.ty, Ty::prim(Prim::I64));
}

#[test]
fn arithmetic_adopts_expected_integer_type() {
    let src = r#"
fun main(): i64 {
    return 1 + 2
}
"#;
    let hir = hir_of(src);
    let HirStmt::Return { value: Some(value) } = &hir.functions[0].body.stmts[0] else {
        panic!("expected a return");
    };
    assert_eq!(value.ty, Ty::prim(Prim::I64));
}

#[test]
fn failed_initializer_still_binds_the_name() {
    let src = r#"
fun main(): i32 {
    val x = nope
    return x
}
"#;
    let errors = diags_of(src);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].message.contains("unknown binding `nope`"),
        "{errors:?}"
    );
}

#[test]
fn poisoned_operand_does_not_cascade() {
    let src = r#"
fun main(): i32 {
    val x = nope + 1
    val y: String = x
    return x
}
"#;
    let errors = diags_of(src);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(
        errors[0].message.contains("unknown binding `nope`"),
        "{errors:?}"
    );
}

#[test]
fn comparison_literal_adopts_peer_integer_type() {
    let src = r#"
fun main(): i32 {
    val a: i64 = 3
    if (a > 0) {
        return 1
    } else {
        return 0
    }
}
"#;
    assert!(check(src).is_ok(), "{:?}", check(src).diagnostics);
}

#[test]
fn equality_with_none_uses_peer_nullable_type() {
    let src = r#"
fun main(): i32 {
    val name: String? = None
    if (name == None) {
        return 1
    } else {
        return 0
    }
}
"#;
    assert!(check(src).is_ok(), "{:?}", check(src).diagnostics);
}

#[test]
fn missing_return_on_non_unit_function_is_type_error() {
    let src = r#"
fun main(): i32 {
    val x = 1
}
"#;
    let errors = diags_of(src);
    assert!(
        errors.iter().any(|d| d.phase == Phase::Type
            && d.message.contains("must return a value")),
        "{errors:?}"
    );
}

#[test]
fn if_without_else_missing_return_is_type_error() {
    let src = r#"
fun main(): i32 {
    if (1 > 0) {
        return 1
    }
}
"#;
    let errors = diags_of(src);
    assert!(
        errors.iter().any(|d| d.phase == Phase::Type
            && d.message.contains("must return a value")),
        "{errors:?}"
    );
}

#[test]
fn statements_after_a_type_error_are_still_checked() {
    let src = r#"
fun main(): i32 {
    val x: i32 = "a"
    return "b"
}
"#;
    let errors = diags_of(src);
    assert!(
        errors.iter().any(|e| e.message.contains("initializer")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|e| e.message.contains("return value")),
        "{errors:?}"
    );
}

#[test]
fn samples_check_clean() {
    assert!(check(crate::MVP_SAMPLE).is_ok());
    assert!(check(crate::PROCESS_USER_SAMPLE).is_ok());
}

#[test]
fn unknown_named_type_errors() {
    let errors = diags_of(r#"fun main(): Foo { return 1 }"#);
    assert!(errors.iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn unknown_named_type_is_type_error() {
    let errors = diags_of(r#"fun main(value: Missing): i32 { return 0 }"#);
    assert!(errors
        .iter()
        .any(|error| error.phase == Phase::Type && error.message.contains("unknown named type")));
}

#[test]
fn unknown_string_field_is_type_error() {
    let errors = diags_of(
        r#"fun main(): i32 {
    val value: String = "x"
    return value.missing
}"#,
    );
    assert!(errors
        .iter()
        .any(|error| error.phase == Phase::Type && error.message.contains("unknown field")));
}

#[test]
fn redefining_print_is_type_error() {
    let src = r#"
fun print(x: i32) {}
fun main(): i32 {
    return 0
}
"#;
    let report = check(src);
    assert!(!report.is_ok());
    assert!(report.diagnostics.iter().any(|d| {
        d.phase == Phase::Type
            && d.message.contains("cannot redefine builtin function `print`")
            && d.span.is_some()
    }));
}

#[test]
fn redefining_println_with_other_arity_is_rejected_not_bypassed() {
    let src = r#"
fun println(a: i32, b: i32) {}
fun main(): i32 {
    println("hi")
    return 0
}
"#;
    let report = check(src);
    assert!(!report.is_ok());
    assert!(report.diagnostics.iter().any(|d| d
        .message
        .contains("cannot redefine builtin function `println`")));
}

#[test]
fn logical_and_requires_bool_operands() {
    let src = r#"
fun main() {
    val b: bool = true && 1
}
"#;
    assert!(diags_of(src).iter().any(|d| d.message.contains("logical operands")));
}

#[test]
fn while_break_continue_typecheck() {
    let hir = hir_of(
        r#"
fun main(): i32 {
    var i: i32 = 0
    while (i < 3) {
        if (i == 1) { break }
        i = i + 1
        continue
    }
    return i
}
"#,
    );
    assert!(hir.functions[0].body.stmts.iter().any(|s| matches!(s, HirStmt::While { .. })));
}

#[test]
fn break_outside_loop_is_type_error() {
    let src = r#"
fun main(): i32 {
    break
    return 0
}
"#;
    assert!(diags_of(src).iter().any(|d| d.message.contains("`break` outside")));
}

#[test]
fn class_construction_and_field_typecheck() {
    let hir = hir_of(
        r#"
class Point {
    x: i32
    y: i32
}
fun main(): i32 {
    val p = Point(1, 2)
    return p.x
}
"#,
    );
    assert_eq!(hir.classes.len(), 1);
    assert_eq!(hir.classes[0].name, "Point");
    assert!(matches!(
        &hir.functions[0].body.stmts[0],
        HirStmt::VarDecl {
            ty,
            value: HirExpr { kind: HirExprKind::ObjectConstruct { .. }, .. },
            ..
        } if !ty.is_copy()
    ));
}

#[test]
fn class_string_field_typechecks() {
    let hir = hir_of(
        r#"
class Label { text: String }
fun main(): i32 { return Label("hello").text.length }
"#,
    );
    assert_eq!(hir.classes[0].fields[0].ty, Ty::string(false));
}

#[test]
fn class_name_conflicts_with_function() {
    let src = "class Point { x: i32 }\nfun Point() {}\nfun main() {}";
    assert!(diags_of(src)
        .iter()
        .any(|d| d.message.contains("conflicts with") && d.message.contains("function")));
}

#[test]
fn class_name_conflicts_with_builtin_type() {
    for src in [
        "class String { x: i32 }\nfun main() {}",
        "class i32 { x: i32 }\nfun main() {}",
        "class Int { x: i32 }\nfun main() {}",
    ] {
        assert!(
            diags_of(src)
                .iter()
                .any(|d| d.message.contains("builtin type")),
            "{src}"
        );
    }
}

#[test]
fn class_forward_reference_typechecks() {
    let hir = hir_of(
        r#"
class Outer { inner: Inner }
class Inner { n: i32 }
fun main(): i32 {
    val outer = Outer(Inner(7))
    return outer.inner.n
}
"#,
    );
    assert_eq!(hir.classes.len(), 2);
    assert_eq!(hir.classes[0].fields[0].ty.to_string(), "Inner");
}

#[test]
fn class_owned_layout_cycle_is_type_error() {
    let src = r#"
class A { b: B }
class B { a: A }
fun main() {}
"#;
    assert!(diags_of(src)
        .iter()
        .any(|d| d.message.contains("recursive by-value layout")));
}

#[test]
fn class_parent_var_requires_move() {
    let src = r#"
class Point { x: i32 }
fun main() {
    var p = Point(1)
    { val x = p.x }
}
"#;
    assert!(diags_of(src)
        .iter()
        .any(|d| d.message.contains("`p` is not Copy")));
}

#[test]
fn class_field_assign_through_val_binding_ok() {
    let hir = hir_of(
        r#"
class Point { x: i32 }
fun main(): i32 {
    val p = Point(1)
    p.x = 2
    return p.x
}
"#,
    );
    assert!(matches!(
        &hir.functions[0].body.stmts[1],
        HirStmt::Assign {
            target: crate::hir::HirAssignTarget::Field { field, .. },
            ..
        } if field == "x"
    ));
}

#[test]
fn class_nullable_field_allowed() {
    let hir = hir_of(
        r#"
class Box { n: i32? }
fun main(): i32 {
    val b = Box(None)
    return b.n ?: 0
}
"#,
    );
    assert_eq!(hir.classes[0].fields[0].ty, Ty::i32().with_nullable(true));
}

#[test]
fn class_nullable_field_assign_ok() {
    let hir = hir_of(
        r#"
class Box { n: i32? }
fun main(): i32 {
    val b = Box(None)
    b.n = Some(9)
    return b.n ?: 0
}
"#,
    );
    assert_eq!(hir.classes[0].fields[0].ty, Ty::i32().with_nullable(true));
    assert!(matches!(
        &hir.functions[0].body.stmts[1],
        HirStmt::Assign {
            target: crate::hir::HirAssignTarget::Field { field, .. },
            ..
        } if field == "n"
    ));
}

