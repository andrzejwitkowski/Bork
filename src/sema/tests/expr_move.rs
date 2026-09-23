use super::super::*;
use crate::dump::dump_arenas;
use crate::parse;

#[test]
fn bare_var_assign_requires_move() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("move")),
        "bare var RHS must error: {errs:?}"
    );
}

#[test]
fn expr_move_already_moved_name_errors() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = move s
    var y = move s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].message.contains("already moved"), "{errs:?}");
}

#[test]
fn expr_move_of_copy_keeps_source_usable() {
    let src = r#"
fun main() {
    var n = 1
    var m = move n
    val t = n
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "Copy n must remain usable: {errs:?}");
}

#[test]
fn expr_move_rebinding_dump_shows_local_dest_and_moved_source() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = move s
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let main = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun main"))
        .expect("main");
    assert!(
        main.bindings.iter().any(|b| {
            b.name == "x" && matches!(b.ownership, Ownership::Local)
        }),
        "x should be Local: {:?}",
        main.bindings
    );
    assert!(
        main.bindings.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Moved { .. })
        }),
        "s should be Moved after expression move: {:?}",
        main.bindings
    );
    let text = dump_arenas(&report);
    assert!(text.contains("s [Moved"), "dump should show moved source:\n{text}");
    assert!(text.contains("x [Local]"), "dump should show local dest:\n{text}");
}

#[test]
fn expr_move_rebinding_marks_source_moved() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = move s
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "s must be moved: {errs:?}"
    );
}

#[test]
fn expr_move_unknown_name_errors() {
    let src = r#"
fun main() {
    var x = move missing
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(errs[0].message.contains("unknown name `missing`"), "{errs:?}");
}

#[test]
fn inferred_move_with_expr_move_does_not_pre_capture() {
    // `move s` must transfer the outer binding, not first become a regional capture.
    let src = r#"
fun main() {
    var s: String = "hi"
    move {
        var t = move s
    }
    val u = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "outer s should be moved by expression move: {errs:?}"
    );
    assert!(
        !errs.iter().any(|e| e.message.contains("capture")),
        "must not reject as already-captured: {errs:?}"
    );
}

#[test]
fn move_of_regional_capture_errors() {
    let src = r#"
fun main() {
    var a: String = "A"
    move (a) {
        var t = move a
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter()
            .any(|e| e.message.contains("capture") || e.message.contains("already")),
        "second move of capture must error: {errs:?}"
    );
}

#[test]
fn nested_expr_move_dump_shows_moved_source() {
    let src = r#"
fun main() {
    var s: String = "hi"
    {
        var x = move s
    }
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let nested = &report.roots[0].children[0];
    assert!(
        nested.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Moved { .. })
        }),
        "nested move should be visible: {:?}",
        nested.observations
    );
    let text = dump_arenas(&report);
    assert!(text.contains("s [Moved"), "dump should show nested move:\n{text}");
}

#[test]
fn regional_capture_can_move_onward_into_nested_arena() {
    let src = r#"
fun main() {
    var a: String = "A"
    move (a) {
        if (true) {
            var t = move a
        }
    }
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let move_region = &report.roots[0].children[0];
    let nested = &move_region.children[0];
    assert!(nested.observations.iter().any(|b| {
        b.name == "a" && matches!(b.ownership, Ownership::Moved { .. })
    }));
}

#[test]
fn regional_capture_use_without_inner_move_ok() {
    let src = r#"
fun main() {
    var a: String = "A"
    move (a) {
        val t = a
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn shared_then_move_keeps_moved_observation() {
    let src = r#"
fun main() {
    val s: String = "hi"
    {
        val a = s
        var b = move s
    }
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let nested = &report.roots[0].children[0];
    assert!(
        nested.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Shared { .. })
        }),
        "shared read should remain: {:?}",
        nested.observations
    );
    assert!(
        nested.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Moved { .. })
        }),
        "later move must not be dropped: {:?}",
        nested.observations
    );
}

#[test]
fn nested_some_var_assign_requires_move() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = Some(s)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("move")),
        "Some(s) must require move: {errs:?}"
    );
}

#[test]
fn nested_some_move_ok() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var x = Some(move s)
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "{errs:?}"
    );
}
