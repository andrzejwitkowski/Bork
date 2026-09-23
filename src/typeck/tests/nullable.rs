use crate::diag::Phase;
use crate::frontend::check;

#[test]
fn elvis_requires_nullable_lhs() {
    let src = r#"
fun main(): String {
    val s: String = "a"
    return s ?: "b"
}
"#;
    assert!(check(src)
        .unwrap_err()
        .iter()
        .any(|d| d.phase == Phase::Type));
}

#[test]
fn elvis_ok() {
    let src = r#"
fun main(): String {
    val s: String? = None
    return s ?: "Guest"
}
"#;
    assert!(check(src).is_ok());
}

#[test]
fn not_null_assert_ok() {
    let src = r#"
fun main(): String {
    val s: String? = Some("x")
    return s!!
}
"#;
    assert!(check(src).is_ok());
}
