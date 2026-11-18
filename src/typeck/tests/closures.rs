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
