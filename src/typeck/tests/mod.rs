use crate::diag::Phase;
use crate::frontend::check;

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
fn trailing_closure_reports_not_typed_yet() {
    let src = r#"
fun f(): i32 { return 1 }
fun main(): i32 { return f() { -> 1 } }
"#;
    assert!(check(src)
        .unwrap_err()
        .iter()
        .any(|d| d.phase == Phase::Type && d.message == "trailing closures not typed yet"));
}
