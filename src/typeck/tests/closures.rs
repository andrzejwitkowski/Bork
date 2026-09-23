use crate::diag::Phase;

use super::diags_of;

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
