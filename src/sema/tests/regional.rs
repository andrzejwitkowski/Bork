use super::super::*;
use crate::dump::dump_arenas;
use crate::parse;

#[test]
fn move_consumes_class_owner() {
    let result = crate::frontend::check(
        "class Node { value: i32 }\nfun main() {\n val node = Node(1)\n val taken = move node\n node.value\n}",
    );
    assert!(result.diagnostics.iter().any(|diagnostic| {
        diagnostic.phase == crate::diag::Phase::Ownership
            && diagnostic.message.contains("use of `node` after move")
    }), "{:?}", result.diagnostics);
}

#[test]
fn creating_managed_refs_keeps_class_owner_live() {
    let result = crate::frontend::check(
        r#"
class Node {
    value: i32
    next: Ref<Node>
}
fun take(var reference: Ref<Node>) {}
fun main() {
    var node = Node(1)
    var reference: Ref<Node> = node
    reference = node
    var refs: [Ref<Node>; 2] = [node, None]
    refs[0] = node
    val holder = Node(2, node)
    holder.next = node
    take(node)
    node.value
}
"#,
    );
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn copying_managed_ref_to_child_is_shared() {
    let result = crate::frontend::check(
        "class Node { value: i32 }\nfun main() {\n var reference: Ref<Node> = Node(1)\n { var copy: Ref<Node> = reference }\n}",
    );
    assert!(result.is_ok(), "{:?}", result.diagnostics);
    let report = result.report.unwrap();
    assert!(report.roots[0].children[0].observations.iter().any(|observation| {
        observation.name == "reference" && matches!(observation.ownership, Ownership::Shared { .. })
    }));
}

#[test]
fn class_ref_fields_can_be_assigned_through_presence_bindings() {
    for body in [
        "if val live = root { live.next = target }",
        "when root {\n Some(live) => { live.next = target }\n None => {}\n}",
    ] {
        let result = crate::frontend::check(&format!(
            "class Node {{ next: Ref<Node> }}\nfun main() {{\n val target = Node()\n val root: Ref<Node> = Node()\n {body}\n target.next\n}}",
        ));
        assert!(result.is_ok(), "{body}: {:?}", result.diagnostics);
        let report = result.report.unwrap();
        let live = &report.roots[0].children[0].bindings[0];
        assert!(
            matches!(&live.ownership, Ownership::Borrow { from } if from == "presence guard"),
            "{:?}",
            live,
        );
    }
}

#[test]
fn creating_ref_from_if_result_keeps_owners_live() {
    let result = crate::frontend::check(
        "class Node { value: i32 }\nfun main() {\n var a = Node(1)\n var b = Node(2)\n val reference: Ref<Node> = if (true) { a } else { b }\n a.value\n b.value\n}",
    );
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn bare_var_decl_transfer_emits_one_diagnostic() {
    let src = r#"
fun main() {
    var s: String = "hi"
    {
        var x = s
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert_eq!(errs.len(), 1, "transfer policy must not also diagnose: {errs:?}");
    assert!(errs[0].message.contains("move"), "{errs:?}");
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
fn borrow_to_owned_param_does_not_record_borrow_on_type_error() {
    let src = r#"
fun own(buf: [i32; 2]) {
    buf[0] = 1
}
fun main() {
    var a: [i32; 2] = [1, 2]
    own(&a)
}
"#;
    let prog = parse(src).unwrap();
    let (_, type_diags) = {
        let (_, _, diags) = crate::typeck::check(&prog);
        ((), diags)
    };
    assert!(!type_diags.is_empty(), "expected type error for own(&a)");
    let (report, sema_errs) = analyze(&prog);
    assert!(sema_errs.is_empty(), "{sema_errs:?}");
    let main = &report.roots[0];
    let ghost = main
        .observations
        .iter()
        .chain(main.children.iter().flat_map(|c| c.observations.iter()))
        .any(|o| o.name == "a" && matches!(o.ownership, Ownership::Borrow { .. }));
    assert!(!ghost, "own(&a) must not record Borrow on `a`");
}

#[test]
fn borrow_of_outer_var_is_recorded_and_bare_name_still_errors() {
    let borrowed = r#"
fun main() {
    var s: String = "hi"
    {
        val b = &s
    }
}
"#;
    let (report, errs) = analyze(&parse(borrowed).unwrap());
    assert!(errs.is_empty(), "{errs:?}");
    let block = &report.roots[0].children[0];
    assert!(
        block.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Borrow { .. })
        }),
        "{:?}",
        block.observations
    );

    let bare = r#"
fun main() {
    var s: String = "hi"
    {
        val b = s
    }
}
"#;
    let (_, errs) = analyze(&parse(bare).unwrap());
    assert!(errs.iter().any(|e| e.message.contains("move")), "{errs:?}");
}

#[test]
fn var_from_view_binding_is_rejected() {
    let src = r#"
fun main() {
    var s: String = "hi"
    {
        val b = &s
        var c = b
    }
}
"#;
    let (_, errs) = analyze(&parse(src).unwrap());
    assert!(
        errs.iter().any(|e| e.message.contains("borrow") || e.message.contains("var")),
        "{errs:?}"
    );
}

#[test]
fn move_of_borrow_binding_is_rejected() {
    let src = r#"
fun main() {
    var s: String = "hi"
    {
        val b = &s
        var c = move b
    }
}
"#;
    let (_, errs) = analyze(&parse(src).unwrap());
    assert!(
        errs.iter().any(|e| e.message.contains("borrow")),
        "{errs:?}"
    );
}

#[test]
fn chained_presence_has_one_success_region_and_all_borrows() {
    let checked = crate::frontend::check("class Node { next: Ref<Node> }\nfun use(r: Ref<Node>) { if val (a = r, b = a.next, c = b.next) { c.next } }");
    assert!(checked.is_ok(), "{:?}", checked.diagnostics);
    let report = checked.report.unwrap();
    let some = &report.roots[0].children[0];
    assert_eq!(some.label, "IfValSome");
    assert_eq!(
        some.bindings
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c"]
    );
    assert!(some
        .bindings
        .iter()
        .all(|b| matches!(b.ownership, Ownership::Borrow { .. })));
    assert!(some.children.is_empty());
    assert!(some.codegen_push);
}

#[test]
fn chained_presence_rejects_conditional_owner_consumption() {
    for after in ["else { owner.next }", "\n owner.next"] {
        let source = format!("class Node {{ next: Ref<Node> }}\nfun take(var n: Node): Ref<Node> {{ return n.next }}\nfun use(r: Ref<Node>) {{\n var owner = Node()\n if val (a = r, b = take(move owner)) {{}} {after}\n}}");
        let checked = crate::frontend::check(&source);
        assert!(
            checked
                .diagnostics
                .iter()
                .any(|d| d.message.contains("after move")),
            "{:?}",
            checked.diagnostics
        );
    }
}

#[test]
fn chained_presence_borrow_cannot_move_or_escape() {
    let moved = crate::frontend::check("class Node { next: Ref<Node> }\nfun use(r: Ref<Node>) { if val (a = r, b = a.next) { val owned = move b } }");
    assert!(
        moved
            .diagnostics
            .iter()
            .any(|d| d.message.contains("move") && d.phase == crate::diag::Phase::Ownership),
        "{:?}",
        moved.diagnostics
    );
    let returned = crate::frontend::check("class Node { next: Ref<Node> }\nfun use(r: Ref<Node>): &Node {\n if val (a = r, b = a.next) { return b }\n val fallback = Node()\n return &fallback\n}");
    assert!(
        returned
            .diagnostics
            .iter()
            .any(|d| d.message.contains("returning a reference")),
        "{:?}",
        returned.diagnostics
    );
    let escaped = crate::frontend::check("class Node { next: Ref<Node> }\nfun use(r: Ref<Node>) {\n val outer = Node()\n val view = if val (a = r, b = a.next) { b } else { &outer }\n}");
    assert!(
        escaped
            .diagnostics
            .iter()
            .any(|d| d.message.contains("presence guard reference cannot escape")),
        "{:?}",
        escaped.diagnostics
    );
}

#[test]
fn chained_presence_rejects_moves_on_a_nested_rhs_path() {
    let checked = crate::frontend::check("class Node { next: Ref<Node> }\nfun take(var n: Node): Ref<Node> { return n.next }\nfun use(r: Ref<Node>, flag: bool) {\n var owner = Node()\n if val (a = r, b = if (flag) { take(move owner) } else { r }) {}\n owner.next\n}");
    assert!(
        checked
            .diagnostics
            .iter()
            .any(|d| d.message.contains("after move")),
        "{:?}",
        checked.diagnostics
    );
}

#[test]
fn chained_presence_free_vars_only_capture_external_names() {
    let checked = crate::frontend::check("class Node { next: Ref<Node> }\nfun use(r: Ref<Node>) { move { if val (a = r, b = a.next) { b.next } } }");
    assert!(checked.is_ok(), "{:?}", checked.diagnostics);
    let report = checked.report.unwrap();
    let moved = &report.roots[0].children[0];
    assert_eq!(
        moved
            .bindings
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        ["r"]
    );
}

#[test]
fn chained_presence_keeps_moves_when_header_shadows_consumed_owner() {
    let checked = crate::frontend::check("class Node { next: Ref<Node> }\nfun take(var n: Node): Ref<Node> { return n.next }\nfun use(r: Ref<Node>) {\n var owner = Node()\n if val (a = r, b = take(move owner), owner = r) { owner.next }\n owner.next\n}");
    assert!(
        checked
            .diagnostics
            .iter()
            .any(|d| d.message.contains("use of `owner` after move")),
        "{:?}",
        checked.diagnostics
    );
}

#[test]
fn managed_ref_field_stores_keep_owners_with_complex_receivers() {
    for receiver in ["a", "a.child", "make().child"] {
        let source = format!("class Node {{ value: i32 }}\nclass Child {{ next: Ref<Node> }}\nclass Parent {{ child: Child\n next: Ref<Node> }}\nfun make(): Parent {{ return Parent(Child()) }}\nfun use(r: Ref<Parent>) {{\n var owner = Node(42)\n if val (a = r, b = r) {{ {receiver}.next = owner }}\n owner.value\n}}");
        let checked = crate::frontend::check(&source);
        assert!(checked.is_ok(), "{source}\n{:?}", checked.diagnostics);
    }
}
