use crate::diag::Phase;
use crate::frontend::check;

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
    assert!(check(src)
        .unwrap_err()
        .iter()
        .any(|d| d.phase == Phase::Type));
}

