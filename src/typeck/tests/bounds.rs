//! Bounds, predicates and the trait solver (#56).

use crate::ty::{ParamIdx, TyData};

const PRELUDE: &str = "trait Show {\n    fun show(): String\n}\nclass Circle {\n    r: f64\n}\nclass Square {\n    s: f64\n}\nclass Box<T> {\n    v: T\n}\nimpl Show for Circle {\n    fun show(): String { return \"c\" }\n}\nimpl<T: Show> Show for Box<T> {\n    fun show(): String { return \"b\" }\n}\nfun need<T: Show>(t: &T) { }\n";

fn run(body: &str) -> (crate::typeck::Typeck, Vec<String>) {
    let program = crate::parse(&format!("{PRELUDE}{body}")).expect("parses");
    let typeck = crate::typeck::check_with_members(&program);
    let messages = typeck.diagnostics.iter().map(|d| d.message.clone()).collect();
    (typeck, messages)
}

fn clean(body: &str) {
    let (_, messages) = run(body);
    assert!(messages.is_empty(), "{messages:?}");
}

fn rejected(body: &str, needle: &str) {
    let (_, messages) = run(body);
    assert!(messages.iter().any(|m| m.contains(needle)), "{needle}: {messages:?}");
}

#[test]
fn bounds_are_lowered_into_the_signature() {
    let program = crate::parse(&format!("{PRELUDE}fun both<T, U>(t: &T, u: &U) where T: Show, U: Show {{ }}\nfun main() {{}}\n")).unwrap();
    let mut diagnostics = Vec::new();
    let defs = crate::defs::DefTable::build(&program);
    let mut tcx = crate::ty::TyCtx::new(&defs);
    let traits = super::super::traits::build_traits(&program, &defs, &mut tcx, &mut diagnostics);
    let sigs = super::super::collect_fun_sigs(&program, &defs, &traits, &mut tcx, &mut diagnostics);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let both = &sigs[&defs.lookup_fn("both").unwrap()];
    assert_eq!(both.predicates.len(), 2);
    assert_eq!(*tcx.data(both.predicates[0].self_ty), TyData::Param(ParamIdx(0)));
    assert_eq!(*tcx.data(both.predicates[1].self_ty), TyData::Param(ParamIdx(1)));
    assert!(sigs[&defs.lookup_fn("need").unwrap()].predicates.len() == 1);
}

#[test]
fn satisfied_calls_are_clean() {
    clean("fun main() {\n    val c = Circle(1.0)\n    need(&c)\n    val b = Box(Circle(2.0))\n    need(&b)\n    need::<Circle>(&c)\n}\n");
}

#[test]
fn unsatisfied_calls_name_the_bound_and_the_impls() {
    rejected("fun main() {\n    val s = Square(1.0)\n    need(&s)\n}\n", "the trait bound `Square: Show` is not satisfied; it is implemented for: Circle, Box<T>");
    rejected("fun main() {\n    val n = 5\n    need(&n)\n}\n", "the trait bound `i32: Show` is not satisfied");
}

#[test]
fn an_impl_bound_becomes_a_nested_obligation() {
    rejected("fun main() {\n    val b = Box(Square(1.0))\n    need(&b)\n}\n", "the trait bound `Square: Show` is not satisfied");
    clean("fun main() {\n    val b = Box(Box(Circle(1.0)))\n    need(&b)\n}\n");
}

#[test]
fn a_bound_in_scope_discharges_a_call_inside_a_generic_body() {
    clean("fun pass<T: Show>(t: &T) { need(&t) }\nfun main() {}\n");
    rejected("fun forgot<T>(t: &T) { need(&t) }\nfun main() {}\n", "the trait bound `T: Show` is not satisfied");
}

#[test]
fn a_requirement_waits_until_its_type_is_known() {
    // `make()` has no type of its own: the `val` annotation fixes it, and without one nothing does.
    clean("fun make<U>(): U { return make() }\nfun main() {\n    val c: Circle = make()\n    need(&c)\n}\n");
    rejected("fun make<U>(): U { return make() }\nfun main() {\n    val x = make()\n    need(&x)\n}\n", "cannot infer");
}

#[test]
fn literals_are_defaulted_before_a_pending_bound_is_judged() {
    rejected("fun id<T: Show>(t: T): T { return t }\nfun main() {\n    val a = id(5)\n}\n", "the trait bound `i32: Show` is not satisfied");
}

#[test]
fn recursive_requirements_stop_at_the_depth_limit() {
    let mut expr = "Circle(1.0)".to_string();
    for _ in 0..40 {
        expr = format!("Box({expr})");
    }
    rejected(&format!("fun main() {{\n    val b = {expr}\n    need(&b)\n}}\n"), "overflow evaluating the requirement");
}

#[test]
fn an_unknown_type_does_not_cascade_into_a_bound_error() {
    let (_, messages) = run("fun main() {\n    val b = Nope(1)\n    need(&b)\n}\n");
    assert!(messages.iter().all(|m| !m.contains("trait bound")), "{messages:?}");
}

#[test]
fn bounds_declaration_errors() {
    rejected("fun f<T: Nope>(t: &T) { }\nfun main() {}\n", "unknown trait `Nope`");
    rejected("fun f<T: Show<i32>>(t: &T) { }\nfun main() {}\n", "takes 0 type argument(s) but 1");
    rejected("class H<T: Show> {\n    v: T\n}\nfun main() {}\n", "bounds on the type parameters of a class");
    rejected("enum E<T: Show> {\n    A(v: T)\n}\nfun main() {}\n", "bounds on the type parameters of an enum");
    rejected("trait U<T: Show> {\n}\nfun main() {}\n", "bounds on the type parameters of a trait");
    rejected("trait U {\n    fun m<T: Show>(t: &T)\n}\nfun main() {}\n", "bounds on the type parameters of a trait method");
}

#[test]
fn method_and_impl_bounds_are_checked() {
    clean("class Holder {\n    n: i32\n    fun put<T: Show>(t: &T) { }\n}\nfun main() {\n    val h = Holder(1)\n    val c = Circle(1.0)\n    h.put(&c)\n}\n");
    rejected(
        "class Holder {\n    n: i32\n    fun put<T: Show>(t: &T) { }\n}\nfun main() {\n    val h = Holder(1)\n    val s = Square(1.0)\n    h.put(&s)\n}\n",
        "the trait bound `Square: Show` is not satisfied",
    );
}

#[test]
fn a_repeated_bound_is_an_error() {
    rejected("fun f<T: Show + Show>(t: &T) { }\nfun main() {}\n", "duplicate bound `Show` on `T`");
    rejected("fun f<T: Show>(t: &T) where T: Show { }\nfun main() {}\n", "duplicate bound `Show` on `T`");
}
