//! Enum declarations and variant construction (#49).
use super::{diags_of, hir_of};

fn messages(src: &str) -> Vec<String> {
    diags_of(src).into_iter().map(|d| d.message).collect()
}

fn has(src: &str, needle: &str) {
    let all = messages(src);
    assert!(all.iter().any(|m| m.contains(needle)), "{needle}: {all:?}");
}

#[test]
fn variant_construction_lowers_to_enum_construct() {
    let hir = hir_of("enum Shape { Circle(r: f64), Empty }\nfun main() {\n    val a = Shape.Circle(1.0)\n    val b = Shape.Empty\n}\n");
    let main = hir.functions.iter().find(|f| f.name == "main").unwrap();
    let text = format!("{:?}", main.body);
    assert!(text.contains("EnumConstruct { enum_name: \"Shape\", variant: \"Circle\""), "{text}");
    assert!(text.contains("EnumConstruct { enum_name: \"Shape\", variant: \"Empty\""), "{text}");
}

#[test]
fn a_local_shadows_the_enum_name_in_a_path() {
    has(
        "enum Shape { Empty }\nclass P { x: i32 }\nfun main() {\n    val Shape = P(1)\n    val s = Shape.Empty\n}\n",
        "unknown field `Empty`",
    );
}

#[test]
fn mutual_by_value_recursion_through_a_class_is_rejected() {
    has("class A { e: E }\nenum E { V(a: A) }\nfun main() {}\n", "recursive by-value layout");
}

#[test]
fn ref_to_an_enum_breaks_the_cycle() {
    hir_of("enum List { Nil, Cons(head: i32, tail: Ref<List>) }\nfun main() {}\n");
}

#[test]
fn payload_types_are_checked_when_the_enum_is_declared() {
    has("enum E { A(x: Missing) }\nfun main() {}\n", "unknown named type `Missing`");
    has("enum E { A(x: i32, x: i32) }\nfun main() {}\n", "duplicate field `x` in variant `E.A`");
}

#[test]
fn generic_arity_is_checked_for_enum_types() {
    has("enum Box<T> { V(v: T) }\nfun f(b: Box) {}\nfun main() {}\n", "expects 1 type argument");
}

#[test]
fn enums_are_rejected_by_the_codegen_gate() {
    let result = crate::frontend::check("enum E { A }\nfun main() {\n    val e = E.A\n}\n");
    assert!(result.diagnostics.is_empty());
    let hir = result.hir.unwrap();
    let diagnostics = crate::codegen_gate::gate(&hir, &result.tcx);
    assert!(diagnostics.iter().any(|d| d.message.contains("enums are not supported by codegen yet")), "{diagnostics:?}");
}

#[test]
fn enum_types_in_signatures_and_fields_are_rejected_by_the_gate() {
    for src in [
        "enum E { A }\nfun f(e: E) {}\nfun main() {}\n",
        "enum E { A }\nclass C { e: Ref<E> }\nfun main() {}\n",
    ] {
        let result = crate::frontend::check(src);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        let diagnostics = crate::codegen_gate::gate(&result.hir.unwrap(), &result.tcx);
        assert!(!diagnostics.is_empty(), "{src}");
    }
}

#[test]
fn variant_paths_ignore_safe_access_and_reject_explicit_type_args() {
    has("enum E { A(x: i32) }\nfun main() {\n    val e = E?.A(1)\n}\n", "unknown binding `E`");
    has("enum T<U> { L }\nfun main() {\n    val t: T<i32> = T.L::<i32>()\n}\n", "explicit type arguments on enum variants");
}

#[test]
fn arity_mismatch_is_one_error_and_no_partial_node() {
    assert_eq!(messages("enum E { A(x: i32, y: i32) }\nfun main() {\n    val e = E.A(1)\n}\n").len(), 1);
}

#[test]
fn explicit_type_arguments_on_a_variant_give_one_error() {
    let all = messages("enum T<U> { L }\nfun main() {\n    val t: T<i32> = T.L::<i32>()\n}\n");
    assert_eq!(all.len(), 1, "{all:?}");
}

#[test]
fn the_gate_reports_enums_once_per_function() {
    let result = crate::frontend::check("enum E { A }\nfun main() {\n    val a = E.A\n    val b = E.A\n}\n");
    let diagnostics = crate::codegen_gate::gate(&result.hir.unwrap(), &result.tcx);
    assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
}
