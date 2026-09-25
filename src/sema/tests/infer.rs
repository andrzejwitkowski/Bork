use super::super::*;
use crate::parse;

#[test]
fn arithmetic_binding_is_copy_without_annotation() {
    let src = r#"
fun main() {
    var s0 = 1
    var lo = s0 + 0
    var t = lo + 1
    val u = t
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().all(|e| !e.message.contains("move")),
        "arithmetic bindings must be Copy: {errs:?}"
    );
}

#[test]
fn annotated_i32_binding_still_copy() {
    let src = r#"
fun main() {
    var lo: i32 = 1 + 0
    var t = lo + 1
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn if_initializer_is_copy_without_annotation() {
    let src = r#"
fun main() {
    var sp = 0
    var lo = if (sp < 1) { 1 } else { 2 }
    var t = lo + 1
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().all(|e| !e.message.contains("move")),
        "if initializer must be Copy: {errs:?}"
    );
}

#[test]
fn index_initializer_is_copy_without_annotation() {
    let src = r#"
fun main() {
    var a: [i32; 3] = [1, 2, 3]
    var x = a[0]
    var t = x + 1
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().all(|e| !e.message.contains("move")),
        "index of i32 array must be Copy: {errs:?}"
    );
}

#[test]
fn call_return_type_is_copy_without_annotation() {
    let src = r#"
fun one(): i32 {
    return 1
}
fun main() {
    var x = one()
    var t = x + 1
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().all(|e| !e.message.contains("move")),
        "i32 call result must be Copy: {errs:?}"
    );
}
