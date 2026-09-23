use super::*;
use crate::ast::{Block, Stmt};
use crate::dump::dump_arenas;
use crate::parse;

#[test]
fn collapses_nested_bare_braces() {
    let nested = Block {
        stmts: vec![Stmt::Block(Block {
            stmts: vec![Stmt::Block(Block {
                stmts: vec![Stmt::Return(None)],
            })],
        })],
    };
    let (b, n) = peel_blocks(&nested);
    assert_eq!(n, 2);
    assert!(matches!(b.stmts[0], Stmt::Return(None)));
}

#[test]
fn mvp_sample_analyzes_without_move_errors() {
    let prog = parse(crate::MVP_SAMPLE).expect("parse");
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    assert!(!report.roots.is_empty());
}

#[test]
fn move_block_marks_binding_moved() {
    let src = r#"
fun main() {
    val s: String = "hi"
    move (s) {
        return 1
    }
    return 0
}
"#;
    let prog = parse(src).expect("parse");
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let main = &report.roots[0];
    let move_child = main
        .children
        .iter()
        .find(|c| c.label.contains("MoveBlock"))
        .expect("move child");
    assert!(move_child.bindings.iter().any(|b| {
        b.name == "s" && matches!(b.ownership, Ownership::Moved { .. })
    }));
}

#[test]
fn use_after_move_errors() {
    let src = r#"
fun main() {
    val s: String = "hi"
    move (s) {
        return 1
    }
    val t = s
}
"#;
    let prog = parse(src).expect("parse");
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "{errs:?}"
    );
}

#[test]
fn non_copy_var_without_move_errors() {
    let src = r#"
fun main() {
    var s: String = "hi"
    {
        val t = s
    }
}
"#;
    let prog = parse(src).expect("parse");
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("not Copy")),
        "{errs:?}"
    );
}

#[test]
fn val_string_shared_across_arenas() {
    let src = r#"
fun main() {
    val s: String = "hi"
    {
        val t = s
    }
}
"#;
    let prog = parse(src).expect("parse");
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let block = &report.roots[0].children[0];
    assert!(
        block.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Shared { .. })
        }),
        "{:?}",
        block.observations
    );
}

#[test]
fn dump_annotates_compacted_braces() {
    let src = r#"
fun main() {
    val s: String = "hi"
    {
        {
            move (s) {
                return 1
            }
        }
    }
}
"#;
    let prog = parse(src).unwrap();
    let (report, _) = analyze(&prog);
    let text = dump_arenas(&report);
    assert!(
        text.contains("compacted"),
        "expected compaction annotation in:\n{text}"
    );
}

#[test]
fn copy_crossing_recorded_in_child() {
    let src = r#"
fun main() {
    val n = 1
    {
        val m = n
    }
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let block = &report.roots[0].children[0];
    assert!(block.observations.iter().any(|b| {
        b.name == "n" && matches!(b.ownership, Ownership::Copy)
    }));
}

#[test]
fn inferred_move_captures_free_parent() {
    let src = r#"
fun main() {
    val s: String = "hi"
    move {
        val t = s
    }
    val u = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "{errs:?}"
    );
}

#[test]
fn inferred_move_does_not_capture_copy() {
    let src = r#"
fun main() {
    val n = 1
    move {
        val m = n
    }
    val k = n
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "Copy n must stay usable after inferred move: {errs:?}");
    let move_child = report.roots[0]
        .children
        .iter()
        .find(|c| c.label.contains("MoveBlock"))
        .expect("move child");
    assert!(
        !move_child.bindings.iter().any(|b| {
            b.name == "n" && matches!(b.ownership, Ownership::Moved { .. })
        }),
        "Copy n must not be moved: {:?}",
        move_child.bindings
    );
    assert!(
        move_child.observations.iter().any(|b| {
            b.name == "n" && matches!(b.ownership, Ownership::Copy)
        }),
        "n should be Copy inside inferred move: {:?}",
        move_child.observations
    );
}

#[test]
fn explicit_empty_captures_do_not_infer() {
    let src = r#"
fun main() {
    val s: String = "hi"
    move () {
        val t = s
    }
    val u = s
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "empty capture list must not move s: {errs:?}");
    let move_child = report.roots[0]
        .children
        .iter()
        .find(|c| c.label.contains("MoveBlock"))
        .expect("move child");
    assert!(
        !move_child.bindings.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Moved { .. })
        }),
        "s must not be moved with explicit empty captures: {:?}",
        move_child.bindings
    );
    assert!(
        move_child.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Shared { .. })
        }),
        "s should be Shared inside move (): {:?}",
        move_child.observations
    );
}

#[test]
fn nested_var_shadow_restores_parent() {
    let src = r#"
fun main() {
    val s: String = "hi"
    {
        val s: String = "inner"
    }
    move {
        val t = s
    }
    val u = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "parent s should still be capturable after inner shadow ends: {errs:?}"
    );
}

#[test]
fn if_branches_do_not_share_move_state() {
    let src = r#"
fun main() {
    var s: String = "hi"
    if (true) {
        move (s) {
            return 1
        }
    } else {
        val t = s
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        !errs.iter().any(|e| e.message.contains("after move")),
        "else must not see then-branch move: {errs:?}"
    );
    assert!(
        errs.iter().any(|e| e.message.contains("not Copy")),
        "else reading var s without move should error: {errs:?}"
    );
}

#[test]
fn if_both_branches_move_marks_after() {
    let src = r#"
fun main() {
    val s: String = "hi"
    if (true) {
        move (s) {
            return 1
        }
    } else {
        move (s) {
            return 2
        }
    }
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "both branches move => use after if is after-move: {errs:?}"
    );
}

#[test]
fn if_then_only_move_without_else_does_not_stick() {
    let src = r#"
fun main() {
    val s: String = "hi"
    if (true) {
        move (s) {
            return 1
        }
    }
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        !errs.iter().any(|e| e.message.contains("after move")),
        "then-only move without else must not stick: {errs:?}"
    );
}

#[test]
fn dump_skips_use_site_bindings() {
    let src = r#"
fun main() {
    val n = 1
    val m = n
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let text = dump_arenas(&report);
    let n_lines: Vec<_> = text.lines().filter(|l| l.contains("n [")).collect();
    assert_eq!(n_lines.len(), 1, "dump should list n once:\n{text}");
    let main = &report.roots[0];
    assert!(
        main.observations.iter().any(|b| {
            b.name == "n" && matches!(b.ownership, Ownership::Local)
        }),
        "analyzer should still record use-site for hover: {:?}",
        main.observations
    );
}

#[test]
fn unknown_type_is_not_treated_as_copy() {
    let src = r#"
fun main() {
    var s: String = "hi"
    var t = s
    {
        val u = t
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("not Copy")),
        "t should inherit non-Copy from s: {errs:?}"
    );
}
