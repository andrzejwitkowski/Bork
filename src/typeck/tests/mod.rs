use crate::diag::Phase;
use crate::frontend::check;
use crate::hir::{HirExpr, HirExprKind, HirStmt, Ty, UseKind};

mod closures;
mod nullable;

#[test]
fn assign_to_val_is_type_error() {
    let src = r#"
fun main(): i32 {
    val x: i32 = 1
    x = 2
    return x
}
"#;
    let err = check(src).unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn return_type_mismatch() {
    let src = r#"
fun main(): i32 {
    return "nope"
}
"#;
    let err = check(src).unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn adds_i32() {
    assert!(check(r#"fun main(): i32 { return 1 + 2 }"#).is_ok());
}

#[test]
fn rejects_add_string_int() {
    let err = check(r#"fun main(): i32 { return 1 + "a" }"#).unwrap_err();
    assert!(err.iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn if_cond_must_be_bool() {
    let src = r#"fun main(): i32 { if (1) { return 1 } else { return 0 } }"#;
    assert!(check(src)
        .unwrap_err()
        .iter()
        .any(|d| d.phase == Phase::Type));
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
    assert!(check(src)
        .unwrap_err()
        .iter()
        .any(|d| d.phase == Phase::Type));
}

#[test]
fn call_arg_type_mismatch() {
    let src = r#"
fun f(x: i32): i32 { return x }
fun main(): i32 { return f("a") }
"#;
    assert!(check(src)
        .unwrap_err()
        .iter()
        .any(|d| d.phase == Phase::Type));
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
    let hir = check(src).expect("range loop should typecheck");
    let function = &hir.functions[0];
    assert!(matches!(
        &function.body.stmts[1],
        HirStmt::For { region, .. } if *region != function.region
    ));
}

#[test]
fn move_expr_preserves_type_and_use_kind() {
    let src = r#"
fun main(): String {
    var s: String = "hi"
    val t = move s
    return t
}
"#;
    let hir = check(src).expect("move expression should typecheck");
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
fn hir_exprs_carry_their_type_and_span() {
    let src = r#"
fun main(): i32 {
    val x: i32 = 1
    return x
}
"#;
    let hir = check(src).expect("program should typecheck");
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
    let hir = check(src).expect("move block should typecheck");
    let function = &hir.functions[0];
    let HirStmt::MoveBlock {
        captures,
        body,
        region,
    } = &function.body.stmts[1]
    else {
        panic!("expected a move block, got {:?}", function.body.stmts[1]);
    };
    assert_eq!(captures.as_deref(), Some(["s".to_string()].as_slice()));
    assert_ne!(*region, function.region);
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
    let errors = check(src).unwrap_err();
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
    let hir = check(src).expect("branches unify to i32");
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
    let hir = check(src).expect("literal should take the expected integer type");
    let HirStmt::Return { value: Some(value) } = &hir.functions[0].body.stmts[0] else {
        panic!("expected a return");
    };
    assert_eq!(
        value.ty,
        Ty::Primitive {
            name: "i64".into(),
            nullable: false
        }
    );
}

#[test]
fn samples_check_clean() {
    assert!(check(crate::MVP_SAMPLE).is_ok());
    assert!(check(crate::PROCESS_USER_SAMPLE).is_ok());
}

#[test]
fn unknown_named_type_errors() {
    let src = r#"fun main(): Foo { return 1 }"#;
    assert!(check(src)
        .unwrap_err()
        .iter()
        .any(|d| d.phase == Phase::Type));
}

#[test]
fn unknown_named_type_is_type_error() {
    let errors = check(r#"fun main(value: Missing): i32 { return 0 }"#).unwrap_err();
    assert!(errors
        .iter()
        .any(|error| error.phase == Phase::Type && error.message.contains("unknown named type")));
}

#[test]
fn unknown_string_field_is_type_error() {
    let errors = check(
        r#"fun main(): i32 {
    val value: String = "x"
    return value.missing
}"#,
    )
    .unwrap_err();
    assert!(errors
        .iter()
        .any(|error| error.phase == Phase::Type && error.message.contains("unknown field")));
}
