//! `when` over patterns (#50).
use super::{diags_of, hir_of};

fn messages(src: &str) -> Vec<String> {
    diags_of(src).into_iter().map(|d| d.message).collect()
}

fn has(src: &str, needle: &str) {
    let all = messages(src);
    assert!(all.iter().any(|m| m.contains(needle)), "{needle}: {all:?}");
}

fn clean(src: &str) {
    hir_of(src);
}

const SHAPE: &str = "enum Shape { Circle(r: f64), Rect(w: i32, h: i32), Empty }\n";

fn in_main(body: &str) -> String {
    format!("{SHAPE}fun main() {{\n    val s = Shape.Circle(1.0)\n{body}\n}}\n")
}

#[test]
fn variant_patterns_bind_payload_fields() {
    clean(&in_main("    val n = when s {\n        Circle(r) => { 1 }\n        Shape.Rect(w, h) => { w + h }\n        Empty => { 0 }\n    }"));
}

#[test]
fn wildcard_and_literal_patterns() {
    clean(&in_main("    val n = when s {\n        Rect(1, _) => { 1 }\n        _ => { 0 }\n    }"));
    clean("fun main() {\n    val b = true\n    when b {\n        true => { }\n        false => { }\n    }\n}\n");
    clean("fun main() {\n    val x: i64 = 3\n    val y = when x {\n        1 => { 10 }\n        _ => { 20 }\n    }\n}\n");
}

#[test]
fn nested_destructuring_typechecks() {
    clean(
        "enum Shape { Circle(r: f64), Empty }\nenum Pair { Both(a: Shape, b: Shape) }\nfun main() {\n    val p = Pair.Both(Shape.Empty, Shape.Circle(2.0))\n    when p {\n        Both(Empty, Circle(r)) => { }\n        Both(_, _) => { }\n    }\n}\n",
    );
}

#[test]
fn generic_payloads_are_instantiated_at_the_scrutinee_type() {
    clean("enum Box<T> { Full(v: T), Empty }\nfun main() {\n    val b = Box.Full(3)\n    val n = when b {\n        Full(v) => { v + 1 }\n        Empty => { 0 }\n    }\n}\n");
}

#[test]
fn place_scrutinee_binds_non_copy_payloads_by_borrow() {
    let (hir, tcx) = super::checked_of("class P { x: i32 }\nenum E { A(p: P), B(n: i32) }\nfun main() {\n    val e = E.A(P(1))\n    when e {\n        A(p) => { }\n        B(n) => { }\n    }\n}\n");
    let main = hir.functions.iter().find(|f| f.name == "main").unwrap();
    let crate::hir::HirStmt::Expr(m) = main.body.stmts.last().unwrap() else { panic!("{:?}", main.body) };
    let crate::hir::HirExprKind::Match { arms, .. } = &m.kind else { panic!("{m:?}") };
    let shown = |arm: &crate::hir::HirArm| {
        let crate::hir::HirPatKind::Variant { fields, .. } = &arm.pat.kind else { panic!() };
        let crate::hir::HirPatKind::Bind { binding_ty, .. } = &fields[0].kind else { panic!() };
        tcx.display(*binding_ty).to_string()
    };
    assert_eq!((shown(&arms[0]).as_str(), shown(&arms[1]).as_str()), ("&P", "i32"));
}

#[test]
fn a_field_through_this_is_a_place() {
    clean("enum E { A(n: i32), B }\nclass C {\n    e: E\n    fun f(): i32 {\n        return when e {\n            A(n) => { n }\n            B => { 0 }\n        }\n    }\n}\nfun main() {}\n");
}

#[test]
fn existing_some_none_when_is_unchanged() {
    let hir = hir_of("class N {\n    v: i32\n    next: Ref<N>\n}\nfun main() {\n    val a = N(1)\n    val r: Ref<N> = a.next\n    when r {\n        Some(n) => { }\n        None => { }\n    }\n}\n");
    let text = format!("{:?}", hir.functions.iter().find(|f| f.name == "main").unwrap().body);
    assert!(text.contains("PresenceMatch") && !text.contains("Match { scrutinee"), "{text}");
}

#[test]
fn some_and_none_patterns_over_a_ref() {
    clean("class N {\n    v: i32\n    next: Ref<N>\n}\nfun main() {\n    val a = N(1)\n    val r: Ref<N> = a.next\n    when r {\n        None => { }\n        Some(n) => { }\n    }\n}\n");
}

#[test]
fn unknown_variant_and_wrong_enum() {
    has(&in_main("    when s {\n        Triangle(x) => { }\n        _ => { }\n    }"), "has no variant `Triangle`");
    has(&format!("enum Other {{ X }}\n{SHAPE}fun main() {{\n    val s = Shape.Empty\n    when s {{\n        Other.X => {{ }}\n        _ => {{ }}\n    }}\n}}\n"), "names enum `Other`");
}

#[test]
fn arity_mismatch_is_reported() {
    has(&in_main("    when s {\n        Rect(w) => { }\n        _ => { }\n    }"), "has 2 field(s), the pattern has 1");
}

#[test]
fn duplicate_binding_is_reported() {
    has(&in_main("    when s {\n        Rect(a, a) => { }\n        _ => { }\n    }"), "duplicate pattern binding 'a'");
}

#[test]
fn literal_type_mismatch_is_reported() {
    has("fun main() {\n    val b = true\n    when b {\n        1 => { }\n        _ => { }\n    }\n}\n", "literal pattern has type i32, expected bool");
}

#[test]
fn variant_pattern_on_a_non_enum_is_reported() {
    has("fun main() {\n    val n = 1\n    when n {\n        Foo(x) => { }\n        _ => { }\n    }\n}\n", "is an enum variant");
}

#[test]
fn arm_types_must_agree_when_the_result_is_used() {
    has(&in_main("    val n: i32 = when s {\n        Circle(r) => { 1 }\n        _ => { true }\n    }"), "`when` arm has type");
}
