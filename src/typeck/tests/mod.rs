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
