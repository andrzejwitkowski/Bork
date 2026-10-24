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
