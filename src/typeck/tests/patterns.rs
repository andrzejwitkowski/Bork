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
fn a_ref_payload_binds_as_a_counted_copy_so_it_can_be_matched() {
    let (hir, tcx) = super::checked_of("enum E { A(next: Ref<E>), B }\nfun f(e: &E) {\n    when e {\n        A(next) => {\n            if val n = next { }\n        }\n        B => { }\n    }\n}\nfun main() {}\n");
    let f = hir.functions.iter().find(|f| f.name == "f").unwrap();
    let crate::hir::HirStmt::Expr(m) = f.body.stmts.last().unwrap() else { panic!("{:?}", f.body) };
    let crate::hir::HirExprKind::Match { arms, .. } = &m.kind else { panic!("{m:?}") };
    let crate::hir::HirPatKind::Variant { fields, .. } = &arms[0].pat.kind else { panic!() };
    let crate::hir::HirPatKind::Bind { binding_ty, .. } = &fields[0].kind else { panic!() };
    assert_eq!(tcx.display(*binding_ty).to_string(), "Ref<E>");
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

#[test]
fn bare_name_of_a_payload_variant_is_an_error() {
    has(&in_main("    when s {\n        Circle => { }\n        _ => { }\n    }"), "has 1 field(s); write `Circle(..)`");
}

#[test]
fn a_nullable_enum_is_an_option_so_variant_patterns_name_the_option() {
    has(&format!("{SHAPE}fun f(s: Shape?) {{\n    when s {{\n        Circle(r) => {{ }}\n        _ => {{ }}\n    }}\n}}\nfun main() {{}}\n"), "enum `Option` has no variant `Circle`");
}

#[test]
fn missing_variants_are_reported_with_witnesses() {
    has(&in_main("    when s {\n        Circle(r) => { }\n    }"), "non-exhaustive `when`: missing `Shape.Rect(_, _)`, `Shape.Empty`");
}

#[test]
fn nested_gaps_are_reported() {
    has(
        "enum Shape { Circle(r: f64), Empty }\nenum Pair { Both(a: Shape, b: Shape) }\nfun main() {\n    val p = Pair.Both(Shape.Empty, Shape.Empty)\n    when p {\n        Both(Empty, _) => { }\n        Both(_, Circle(r)) => { }\n    }\n}\n",
        "missing `Pair.Both(Shape.Circle(_), Shape.Empty)`",
    );
}

#[test]
fn integers_and_bools_are_checked() {
    has("fun main() {\n    val x: i64 = 3\n    when x {\n        1 => { }\n        2 => { }\n    }\n}\n", "missing `_`");
    has("fun main() {\n    val b = true\n    when b {\n        true => { }\n    }\n}\n", "missing `false`");
}

#[test]
fn a_lone_some_arm_is_non_exhaustive() {
    has("class N {\n    v: i32\n    next: Ref<N>\n}\nfun main() {\n    val a = N(1)\n    val r: Ref<N> = a.next\n    when r {\n        Some(n) => { }\n    }\n}\n", "missing `None`");
}

#[test]
fn unreachable_arms_are_errors() {
    has(&in_main("    when s {\n        _ => { }\n        Empty => { }\n    }"), "unreachable pattern");
    has("fun main() {\n    val x: i64 = 3\n    when x {\n        1 => { }\n        1 => { }\n        _ => { }\n    }\n}\n", "unreachable pattern");
}

#[test]
fn a_mistyped_unit_variant_shadows_later_arms() {
    has(&in_main("    when s {\n        Circle(r) => { }\n        Rect(w, h) => { }\n        Emtpy => { }\n        Empty => { }\n    }"), "unreachable pattern");
}

#[test]
fn coverage_is_not_checked_when_a_pattern_is_already_wrong() {
    let all = messages(&in_main("    when s {\n        Triangle(x) => { }\n    }"));
    assert_eq!(all.len(), 1, "{all:?}");
}

#[test]
fn full_coverage_passes() {
    clean(&in_main("    when s {\n        Circle(r) => { }\n        Rect(1, h) => { }\n        Rect(w, h) => { }\n        Empty => { }\n    }"));
    clean("enum Box<T> { Full(v: T), Empty }\nfun main() {\n    val b = Box.Full(true)\n    when b {\n        Full(true) => { }\n        Full(false) => { }\n        Empty => { }\n    }\n}\n");
}

#[test]
fn coverage_is_still_checked_when_an_arm_body_has_an_error() {
    has(&in_main("    when s {\n        Circle(r) => { val q: i32 = true }\n        Empty => { }\n    }"), "missing `Shape.Rect(_, _)`");
}

#[test]
fn no_coverage_error_on_an_unknown_scrutinee() {
    let all = messages("fun main() {\n    val x = nope()\n    when x {\n        1 => { }\n    }\n}\n");
    assert!(!all.iter().any(|m| m.contains("non-exhaustive")), "{all:?}");
}

#[test]
fn no_coverage_error_when_the_scrutinee_itself_has_an_error() {
    let all = messages("const N: i64 = 1\nfun main() {\n    when (move N) {\n        1 => { }\n    }\n}\n");
    assert!(all.iter().any(|m| m.contains("constant")), "{all:?}");
    assert!(!all.iter().any(|m| m.contains("non-exhaustive")), "{all:?}");
}
