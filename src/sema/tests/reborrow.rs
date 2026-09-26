use super::super::*;
use crate::dump::dump_arenas;
use crate::parse;

#[test]
fn reference_param_reborrow_in_while_loop() {
    let src = r#"
fun inner(a: &[i32; 3]) {
    a[0] = 7
}
fun outer(a: &[i32; 3], n: i32) {
    var i = 0
    while (i < n) {
        inner(&a)
        i = i + 1
    }
}
fun main() {
    var x: [i32; 3] = [1, 2, 3]
    outer(&x, 1)
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let while_loop = &report.roots[1].children[0];
    assert!(
        while_loop.observations.iter().any(|b| {
            b.name == "a" && matches!(b.ownership, Ownership::Borrow { .. })
        }),
        "expected borrow edge in while:\n{}",
        dump_arenas(&report)
    );
}

#[test]
fn reference_param_reborrow_in_if() {
    let src = r#"
fun inner(a: &[i32; 2]) {
    a[0] = 1
}
fun outer(a: &[i32; 2], flag: bool) {
    if (flag) {
        inner(&a)
    }
}
fun main() {
    var x: [i32; 2] = [1, 2]
    outer(&x, true)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn reference_param_reborrow_in_for_loop() {
    let src = r#"
fun inner(a: &[i32; 2]) {
    a[0] = 1
}
fun outer(a: &[i32; 2]) {
    for (i in 0..1) {
        inner(&a)
    }
}
fun main() {
    var x: [i32; 2] = [1, 2]
    outer(&x)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn recursive_reborrow_inside_if() {
    let src = r#"
fun step(a: &[i32; 4], lo: i32, hi: i32) {
    if (hi - lo > 1) {
        step(&a, lo, hi - 1)
    }
}
fun main() {
    var a: [i32; 4] = [4, 3, 2, 1]
    step(&a, 0, 4)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn nested_val_binding_to_borrow_still_errors() {
    let src = r#"
fun outer(a: &[i32; 2]) {
    if (true) {
        val p = &a
    }
}
fun main() {
    var a: [i32; 2] = [1, 2]
    outer(&a)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("cannot lift borrow")),
        "{errs:?}"
    );
}

#[test]
fn move_borrow_view_in_nested_region_still_errors() {
    let src = r#"
fun sink(a: &[i32; 2]) {}
fun outer(a: &[i32; 2], n: i32) {
    var i = 0
    while (i < n) {
        sink(a)
        i = i + 1
    }
}
fun main() {
    var a: [i32; 2] = [1, 2]
    outer(&a, 1)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("cannot move borrow")),
        "{errs:?}"
    );
}
