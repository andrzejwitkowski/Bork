//! Coherence: overlapping impls are rejected at the declaration (#57).

const HEAD: &str = "trait Show {\n}\ntrait Eq {\n}\ntrait Conv<U> {\n}\nclass Circle {\n    r: f64\n}\nclass Square {\n    s: f64\n}\nclass Box<T> {\n    v: T\n}\nclass Pair<A, B> {\n    a: A\n    b: B\n}\nenum Opt<T> {\n    Some(v: T)\n    None\n}\n";

fn diags(impls: &str) -> Vec<(String, Option<crate::span::Span>)> {
    let src = format!("{HEAD}{impls}fun main() {{}}\n");
    let program = crate::parse(&src).expect("parses");
    crate::typeck::check_with_members(&program).diagnostics.into_iter().map(|d| (d.message, d.span)).collect()
}

fn accepted(impls: &str) {
    assert_eq!(diags(impls), vec![], "{impls}");
}

fn overlaps(impls: &str) {
    let found = diags(impls);
    assert_eq!(found.len(), 1, "{impls}: {found:?}");
    assert!(found[0].0.contains("conflicting implementations of trait"), "{found:?}");
}

#[test]
fn identical_and_generic_overlaps_are_rejected() {
    overlaps("impl Show for Circle {\n}\nimpl Show for Circle {\n}\n");
    overlaps("impl<T> Show for Box<T> {\n}\nimpl Show for Box<Circle> {\n}\n");
    overlaps("impl Show for Box<Circle> {\n}\nimpl<T> Show for Box<T> {\n}\n");
    overlaps("impl<T> Show for Opt<T> {\n}\nimpl Show for Opt<Square> {\n}\n");
}

#[test]
fn overlap_needs_unification_not_equal_text() {
    overlaps("impl<T> Show for Box<Box<T>> {\n}\nimpl<U> Show for Box<U> {\n}\n");
    overlaps("impl<T> Show for Pair<T, T> {\n}\nimpl Show for Pair<Circle, Circle> {\n}\n");
    overlaps("impl<A, B> Show for Pair<A, B> {\n}\nimpl<T> Show for Pair<T, T> {\n}\n");
}

#[test]
fn trait_arguments_take_part_in_the_overlap() {
    overlaps("impl<T> Conv<i32> for Box<T> {\n}\nimpl Conv<i32> for Box<Circle> {\n}\n");
    overlaps("impl<T> Conv<T> for Box<T> {\n}\nimpl Conv<Circle> for Box<Circle> {\n}\n");
}

#[test]
fn bounds_are_not_considered() {
    overlaps("impl<T: Show> Eq for Box<T> {\n}\nimpl<T: Eq> Eq for Box<T> {\n}\n");
    let found = diags("impl<T: Show> Eq for Box<T> {\n}\nimpl<T: Eq> Eq for Box<T> {\n}\n");
    assert!(found[0].0.contains("`Eq for Box<T> where T: Eq` overlaps with `Eq for Box<T> where T: Show`"), "{found:?}");
}

#[test]
fn disjoint_impls_are_accepted() {
    accepted("impl Show for Box<Circle> {\n}\nimpl Show for Box<Square> {\n}\n");
    accepted("impl<T> Show for Pair<T, T> {\n}\nimpl Show for Pair<Circle, Square> {\n}\n");
    accepted("impl<T> Conv<i32> for Box<T> {\n}\nimpl<T> Conv<bool> for Box<T> {\n}\n");
    accepted("impl Show for Circle {\n}\nimpl Eq for Circle {\n}\n");
    accepted("impl<T> Show for Box<T> {\n}\nimpl<T> Show for Opt<T> {\n}\nimpl Show for Circle {\n}\n");
}

#[test]
fn later_impls_are_reported_in_source_order_and_name_both() {
    let impls = "impl<T> Show for Box<T> {\n}\nimpl Eq for Circle {\n}\nimpl Show for Box<Circle> {\n}\nimpl Eq for Circle {\n}\nimpl Show for Box<Square> {\n}\n";
    let found = diags(impls);
    assert_eq!(found.len(), 3, "{found:?}");
    assert!(found[0].0.contains("`Show for Box<Circle>` overlaps with `Show for Box<T>`"), "{found:?}");
    assert!(found[1].0.contains("`Eq for Circle` overlaps with `Eq for Circle`"), "{found:?}");
    assert!(found[2].0.contains("`Show for Box<Square>` overlaps with `Show for Box<T>`"), "{found:?}");
    let src = format!("{HEAD}{impls}fun main() {{}}\n");
    let starts: Vec<usize> = found.iter().map(|(_, span)| span.unwrap().start).collect();
    assert!(starts.windows(2).all(|w| w[0] < w[1]), "{starts:?}");
    assert!(src[starts[0]..].starts_with("Show for Box<Circle>"), "points at the later impl");
    assert_eq!(diags(impls), found, "the order is the same on every run");
}

#[test]
fn an_overlap_does_not_cascade_into_the_use_site() {
    let found = diags("impl<T> Show for Box<T> {\n}\nimpl Show for Box<Circle> {\n}\n");
    assert_eq!(found.len(), 1, "{found:?}");
    let src = format!(
        "{HEAD}impl<T> Show for Box<T> {{\n}}\nimpl Show for Box<Circle> {{\n}}\nfun need<T: Show>(t: &T) {{ }}\nfun main() {{\n    val b = Box(Circle(1.0))\n    need(&b)\n}}\n"
    );
    let program = crate::parse(&src).unwrap();
    let messages: Vec<String> = crate::typeck::check_with_members(&program).diagnostics.into_iter().map(|d| d.message).collect();
    assert_eq!(messages.len(), 1, "{messages:?}");
}
