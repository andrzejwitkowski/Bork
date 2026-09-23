use crate::diag::Phase;
use crate::frontend::check;

use super::diags_of;

#[test]
fn elvis_requires_nullable_lhs() {
    let src = r#"
fun main(): String {
    val s: String = "a"
    return s ?: "b"
}
"#;
    assert!(diags_of(src).iter().any(|d| d.phase == Phase::Type));
}

#[test]
fn elvis_requires_nullable_lhs_for_primitives_too() {
    let src = r#"
fun main(score: i32): i32 {
    return score ?: 0
}
"#;
    let errors = diags_of(src);
    assert!(
        errors
            .iter()
            .any(|error| error.phase == Phase::Type && error.message.contains("must be nullable")),
        "{errors:?}"
    );
}

#[test]
fn ordered_comparison_rejects_nullable_operand() {
    let src = r#"
fun main(name: String?): i32 {
    if (name?.length > 0) {
        return 1
    }
    return 0
}
"#;
    let errors = diags_of(src);
    assert!(
        errors.iter().any(|error| error.phase == Phase::Type
            && error.message.contains("ordered comparison operands")),
        "{errors:?}"
    );
}

#[test]
fn ordered_comparison_accepts_narrowed_operand() {
    let src = r#"
fun main(name: String?): i32 {
    val length: i32 = name?.length ?: 0
    if (length > 0) {
        return 1
    }
    return 0
}
"#;
    assert!(check(src).is_ok());
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
