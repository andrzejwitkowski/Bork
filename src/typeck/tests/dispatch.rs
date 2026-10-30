//! Trait method calls and impl bodies (#58).

const HEAD: &str = "trait Show {\n    fun show(): String\n}\ntrait Name {\n    fun show(): String\n}\nclass Circle {\n    r: f64\n    fun show(): String { return \"inherent\" }\n}\nclass Square {\n    s: f64\n}\nclass Box<T> {\n    v: T\n}\nenum Maybe<T> {\n    Some(v: T)\n    None\n}\nimpl Show for Square {\n    fun show(): String { return \"square\" }\n}\nimpl<T: Show> Show for Box<T> {\n    fun show(): String { return v.show() }\n}\nimpl<T> Show for Maybe<T> {\n    fun show(): String { return \"maybe\" }\n}\nimpl Show for Circle {\n    fun show(): String { return \"trait\" }\n}\n";

fn run(body: &str) -> Vec<String> {
    let program = crate::parse(&format!("{HEAD}{body}")).expect("parses");
    crate::typeck::check_with_members(&program).diagnostics.into_iter().map(|d| d.message).collect()
}

fn clean(body: &str) {
    assert_eq!(run(body), Vec::<String>::new(), "{body}");
}

fn rejected(body: &str, needle: &str) {
    let found = run(body);
    assert!(found.iter().any(|m| m.contains(needle)), "{needle}: {found:?}");
}

#[test]
fn a_bound_provides_the_method() {
    clean("fun f<T: Show>(t: &T): String { return t.show() }\nfun main() {}\n");
}

#[test]
fn an_impl_provides_the_method_on_classes_and_enums() {
    clean("fun main() {\n    val s = Square(1.0)\n    val a = s.show()\n    val b = Box(Square(2.0))\n    val c = b.show()\n    val m = Maybe.Some(1)\n    val d = m.show()\n}\n");
}

#[test]
fn an_inherent_method_shadows_a_trait_method() {
    clean("fun main() {\n    val c = Circle(1.0)\n    val a = c.show()\n}\n");
}

#[test]
fn the_method_must_be_in_the_bounds() {
    rejected("fun f<T: Name>(t: &T): String { return t.nope() }\nfun main() {}\n", "type parameter `T` has no method `nope`; its bounds are: `Name`");
    rejected("fun f<T: Name>(t: &T): String { return t.show() }\nfun g<T>(t: &T): String { return t.show() }\nfun main() {}\n", "only named functions can be called");
}

#[test]
fn two_traits_with_one_method_name_are_ambiguous() {
    rejected("fun f<T: Show + Name>(t: &T): String { return t.show() }\nfun main() {}\n", "ambiguous method `show`: it is provided by the traits `Show`, `Name`");
}

#[test]
fn a_class_without_the_method_still_says_so() {
    rejected("fun main() {\n    val c = Circle(1.0)\n    val a = c.nope()\n}\n", "class `Circle` has no method `nope`");
}

#[test]
fn impl_bodies_are_type_checked() {
    let head = "trait Show {\n    fun show(): String\n}\nclass C {\n    r: f64\n}\n";
    for (body, needle) in [
        ("fun show(): String { return 5 }", "has type i32, expected String"),
        ("fun show(): String { return nope }", "unknown binding `nope`"),
        ("fun show(): String { return r }", "has type f64, expected String"),
    ] {
        let program = crate::parse(&format!("{head}impl Show for C {{\n    {body}\n}}\nfun main() {{}}\n")).unwrap();
        let found: Vec<String> = crate::typeck::check_with_members(&program).diagnostics.into_iter().map(|d| d.message).collect();
        assert!(found.iter().any(|m| m.contains(needle)), "{needle}: {found:?}");
    }
}

#[test]
fn fields_are_reachable_by_bare_name_in_a_generic_impl() {
    clean("class P<T> {\n    v: T\n}\ntrait Get {\n    fun get(): i32\n}\nimpl Get for P<i32> {\n    fun get(): i32 { return v }\n}\nfun main() {}\n");
}

#[test]
fn trait_arguments_and_method_generics_are_inferred_from_the_call() {
    clean("trait Conv<U> {\n    fun conv<V>(u: U, v: V): V\n}\nclass B {\n    n: i32\n}\nimpl Conv<i32> for B {\n    fun conv<W>(u: i32, v: W): W { return v }\n}\nfun main() {\n    val b = B(1)\n    val x = b.conv(5, true)\n}\n");
}

#[test]
fn an_unsatisfied_receiver_is_reported_once() {
    let found = run("fun f<T: Show>(t: &T): String { return t.show() }\nfun main() {\n    val n = 5\n    val x = f(&n)\n}\n");
    assert_eq!(found.iter().filter(|m| m.contains("trait bound")).count(), 1, "{found:?}");
}
