use crate::diag::Phase;
use crate::frontend::check;
use crate::hir::{HirExpr, HirStmt, UseKind};

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
            value: HirExpr::Ident {
                use_kind: UseKind::Move,
                ..
            },
            ..
        }
    ));
}

#[test]
fn samples_check_clean() {
    assert!(check(crate::MVP_SAMPLE).is_ok());
    assert!(check(crate::PROCESS_USER_SAMPLE).is_ok());
}

#[test]
fn unknown_named_type_errors() {
    let src = r#"fun main(): Foo { return 1 }"#;
    assert!(check(src).unwrap_err().iter().any(|d| d.phase == Phase::Type));
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
