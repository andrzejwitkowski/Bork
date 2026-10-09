use crate::hir::{HirAssignTarget, HirExprKind, HirStmt, TyKind};

use super::{diags_of, hir_of};

#[test]
fn records_construct_and_coerce_objects_into_ref_fields() {
    let hir = hir_of(
        r#"
class Node {
    name: String
    parent: Ref<Node>
    next: Ref<Node>
}

fun main() {
    val a = Node("A")
    val b = Node("B")
    a.next = b
    b.parent = a
    val copy: Ref<Node> = a.next
    if val node = copy { node.name } else { "root" }
    when copy {
        Some(node) => { node.name }
        None => { "root" }
    }
}
"#,
    );

    assert_eq!(hir.classes.len(), 1);
    assert!(matches!(
        hir.classes[0].fields[1].ty.kind,
        TyKind::ManagedRef(_)
    ));
    assert!(matches!(
        &hir.functions[0].body.stmts[0],
        HirStmt::VarDecl {
            value: crate::hir::HirExpr {
                kind: HirExprKind::ObjectConstruct { .. },
                ..
            },
            ..
        }
    ));
    assert!(matches!(
        &hir.functions[0].body.stmts[2],
        HirStmt::Assign {
            target: HirAssignTarget::Field { field, .. },
            value: crate::hir::HirExpr {
                kind: HirExprKind::RefCreate(_),
                ..
            },
        } if field == "next"
    ));
    assert!(matches!(
        &hir.functions[0].body.stmts[5],
        HirStmt::Expr(crate::hir::HirExpr {
            kind: HirExprKind::PresenceMatch { .. },
            ..
        })
    ));
}

#[test]
fn mutually_recursive_ref_fields_typecheck() {
    let hir = hir_of(
        r#"
class Parent { child: Ref<Child> }
class Child { parent: Ref<Parent> }
fun main() {
    val parent = Parent()
    val child = Child(parent)
    parent.child = child
}
"#,
    );
    assert_eq!(hir.classes.len(), 2);
    assert!(hir.classes.iter().all(|class| matches!(
        class.fields[0].ty.kind,
        TyKind::ManagedRef(_)
    )));
}

#[test]
fn ref_elvis_and_ref_arrays_typecheck() {
    let hir = hir_of(
        r#"
class Box { value: i32 }
fun main() {
    val item = Box(1)
    val missing: Ref<Box> = None
    val present = missing ?: item
    val first: Ref<Box> = item
    val refs = [first, None]
    val taken = refs[0]
}
"#,
    );
    assert!(!hir.functions.is_empty());
}

#[test]
fn managed_ref_requires_presence_match_before_field_access() {
    let diagnostics = diags_of(
        r#"
class Node { name: String }
fun main() {
    val node = Node("A")
    val ref: Ref<Node> = node
    ref.name
}
"#,
    );

    assert!(diagnostics.iter().any(|diagnostic| diagnostic
        .message
        .contains("requires `if val`, `when`, or `?.`")));
}

#[test]
fn managed_ref_target_must_be_nonnullable_class() {
    for source in [
        "fun main() { val x: Ref<String> = None }",
        "fun main() { val x: Ref<[i32; 2]> = None }",
        "class Box { value: i32 }\nfun main() { val x: Ref<Box?> = None }",
        "fun main() { val x: Ref<i32> = None }",
        "class Box { value: i32 }\nfun main() { val x: Ref<Ref<Box>> = None }",
    ] {
        let result = crate::frontend::check(source);
        let diagnostics = result.diagnostics;
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("managed Ref target must be a non-nullable class")),
            "missing managed Ref target diagnostic for {source:?}: {diagnostics:?}"
        );
    }

    let hir = hir_of(
        "class Box { value: i32 }\nfun main() {\n val x: Ref<Box> = Box(7)\n val y: Ref<Box> = None\n}",
    );
    assert_eq!(hir.classes.len(), 1);

    assert_eq!(
        hir_of("fun main() {\n val text: String? = None\n text?.length\n}").classes.len(),
        0
    );
}

#[test]
fn safe_class_field_requires_managed_ref() {
    for source in [
        "class Box { value: i32 }\nfun main() {\n val box = Box(1)\n box?.value\n}",
        "class Box { value: i32 }\nfun main() {\n val item = Box(1)\n val reference: Ref<Box> = item\n if val live = reference { live?.value }\n}",
        "class Box { value: i32 }\nfun main() {\n val maybe: Box? = None\n maybe?.value\n}",
    ] {
        let result = crate::frontend::check(source);
        assert!(
            result
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic
                    .message
                    .contains("?. on a class field requires Ref<T>")),
            "missing safe class field diagnostic for {source:?}: {:?}",
            result.diagnostics
        );
    }

    let valid = hir_of(
        "class Box { value: i32 }\nfun main() {\n val item = Box(1)\n val reference: Ref<Box> = item\n reference?.value\n}",
    );
    assert_eq!(valid.classes.len(), 1);
}
