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

/// Function params are always Val today (`fun f(x: T)`, no `val`/`var` on params).
/// Cover main→helper calls: val/var args, return into val/var, Shared param inside helper.
#[test]
fn main_calls_helper_val_param_return_to_val() {
    let src = r#"
fun echo(s: String): String {
    return s
}
fun main() {
    val s: String = "hi"
    val out = echo(s)
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(report.roots.len(), 2);
    let echo = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun echo"))
        .expect("echo");
    assert!(
        echo.bindings.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Local)
        }),
        "param s should be Local in echo: {:?}",
        echo.bindings
    );
    let main = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun main"))
        .expect("main");
    assert!(
        main.bindings.iter().any(|b| b.name == "out"),
        "main should bind return into val out: {:?}",
        main.bindings
    );
}

#[test]
fn main_calls_helper_return_into_var() {
    let src = r#"
fun next(n: Int): Int {
    return n + 1
}
fun main() {
    var x = 0
    x = next(x)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn helper_string_param_shared_in_nested_block() {
    let src = r#"
fun use(s: String): Int {
    {
        val t = s
    }
    return 0
}
fun main() {
    val s: String = "hi"
    use(s)
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let use_fn = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun use"))
        .expect("use");
    let nested = &use_fn.children[0];
    assert!(
        nested.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Shared { .. })
        }),
        "param s is Val, so nested read should be Shared: {:?}",
        nested.observations
    );
}

#[test]
fn helper_cannot_share_var_string_param_without_move() {
    // Params are Val-only in the grammar; model "var-like" by rebinding as var inside.
    let src = r#"
fun touch(): Int {
    var s: String = "hi"
    {
        val t = s
    }
    return 0
}
fun main() {
    touch()
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("not Copy")),
        "var String in helper must not cross arenas without move: {errs:?}"
    );
}

#[test]
fn move_outer_binding_inside_loop_errors() {
    let src = r#"
fun main() {
    var s = "Hello, World!"
    for (i in 1..10) {
        move {
            val t = s
        }
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| {
            e.message.contains("inside a loop") && e.message.contains("`s`")
        }),
        "moving outer s each iteration must error: {errs:?}"
    );
}

#[test]
fn move_loop_local_binding_is_ok() {
    let src = r#"
fun main() {
    for (i in 1..10) {
        var s: String = "hi"
        move (s) {
            return 1
        }
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        !errs.iter().any(|e| e.message.contains("inside a loop")),
        "fresh local each iteration may be moved: {errs:?}"
    );
}

/// Gap: calls do not consume non-Copy args (no move-into-callee) and do not
/// treat Copy specially at the call boundary — only a same-arena use in the caller.
#[test]
fn call_does_not_yet_move_non_copy_argument() {
    let src = r#"
fun sink(s: String): Int {
    return 0
}
fun main() {
    val s: String = "hi"
    sink(s)
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.is_empty(),
        "NYI gap: sink(s) must not yet move s in the caller; got {errs:?}"
    );
}

#[test]
fn call_does_not_yet_record_copy_crossing_into_callee() {
    // Callee sees its own Local param; caller only notes a Local use of `n`.
    // There is no Copy observation tied to the call/callee arena yet.
    let src = r#"
fun id(n: Int): Int {
    return n
}
fun main() {
    val n = 1
    val m = id(n)
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
        !main.observations.iter().any(|b| {
            b.name == "n" && matches!(b.ownership, Ownership::Copy)
        }),
        "NYI gap: caller must not yet record Copy for arg `n` at call; got {:?}",
        main.observations
    );
    let id = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun id"))
        .expect("id");
    assert!(
        id.bindings.iter().any(|b| {
            b.name == "n" && matches!(b.ownership, Ownership::Local)
        }),
        "callee still has its own Local param: {:?}",
        id.bindings
    );
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
