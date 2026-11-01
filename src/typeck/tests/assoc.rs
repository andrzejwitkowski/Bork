//! Associated types and their normalization (#59).

fn diags(src: &str) -> Vec<String> {
    crate::frontend::check(src).diagnostics.into_iter().map(|d| d.message).collect()
}

fn has(src: &str, needle: &str) {
    let messages = diags(src);
    assert!(messages.iter().any(|m| m.contains(needle)), "{needle}: {messages:?}");
}

fn clean(src: &str) {
    let messages = diags(src);
    assert!(messages.is_empty(), "{messages:?}");
}

const CONTAINER: &str = "trait Container {\n    type Item\n    fun get(): Self::Item\n}\n\
class IntBox {\n    v: i32\n}\nclass Named {\n    n: String\n}\n\
impl Container for IntBox {\n    type Item = i32\n    fun get(): i32 { return this.v }\n}\n\
impl Container for Named {\n    type Item = String\n    fun get(): String { return this.n }\n}\n\
fun first<T: Container>(c: &T): T::Item { return c.get() }\n";

fn with_main(body: &str) -> String {
    format!("{CONTAINER}fun main() {{\n{body}\n}}\n")
}

#[test]
fn a_projection_of_a_concrete_type_is_the_impls_type() {
    clean(&with_main("val b = IntBox(1)\nval x: i32 = first(&b)\nval y: i32 = b.get()"));
    has(&with_main("val b = IntBox(1)\nval x: String = first(&b)"), "i32");
    has(&with_main("val s = Named(\"a\")\nval x: i32 = first(&s)"), "String");
}

#[test]
fn a_projection_written_on_a_concrete_type_normalizes() {
    clean(&with_main("val x: IntBox::Item = 3"));
    has(&with_main("val x: IntBox::Item = \"s\""), "i32");
}

#[test]
fn a_type_parameters_projection_is_opaque_inside_its_function() {
    clean(&format!("{CONTAINER}fun same<T: Container>(c: &T): T::Item {{ return c.get() }}\nfun main() {{}}\n"));
    has(
        &format!("{CONTAINER}fun bad<T: Container>(c: &T): i32 {{ return c.get() }}\nfun main() {{}}\n"),
        "i32",
    );
}

#[test]
fn an_equality_bound_fixes_the_projection() {
    let src = format!("{CONTAINER}fun total<T: Container<Item = i32>>(c: &T): i32 {{ return c.get() + 1 }}\nfun main() {{}}\n");
    clean(&src);
    has(&format!("{src}fun m(s: &Named) {{ total(s) }}\n"), "expected the associated type to be `i32`, but it is `String`");
}

#[test]
fn a_projection_waits_for_a_literal_type() {
    // `?int` is only defaulted at the end of the body; the projection must wait and then agree.
    let src = "trait Container {\n    type Item\n    fun get(): Self::Item\n}\n\
class Box<T> {\n    v: T\n}\n\
impl Container for Box<i32> {\n    type Item = i32\n    fun get(): i32 { return this.v }\n}\n\
fun first<T: Container>(c: &T): T::Item { return c.get() }\n";
    clean(&format!("{src}fun main() {{\n    val b = Box(1)\n    val x: i32 = first(&b)\n}}\n"));
}

#[test]
fn a_cyclic_associated_type_is_reported_once() {
    let src = "trait Container {\n    type Item\n    fun get(): Self::Item\n}\nclass A {\n    v: i32\n}\n\
impl Container for A {\n    type Item = Self::Item\n    fun get(): i32 { return this.v }\n}\nfun main() {}\n";
    let messages = diags(src);
    assert_eq!(messages.iter().filter(|m| m.contains("refers to itself")).count(), 1, "{messages:?}");
}

#[test]
fn the_trait_that_declares_a_projection_is_kept() {
    // Both traits declare `Item` for `Foo`; the bound `T: Container` must pick `Container`'s.
    let src = "trait Other {\n    type Item\n}\ntrait Container {\n    type Item\n    fun get(): Self::Item\n}\n\
class Foo {\n    v: i32\n}\nimpl Other for Foo {\n    type Item = String\n}\n\
impl Container for Foo {\n    type Item = i32\n    fun get(): i32 { return this.v }\n}\n\
fun first<T: Container>(c: &T): T::Item { return c.get() }\n";
    clean(&format!("{src}fun main() {{\n    val f = Foo(1)\n    val x: i32 = first(&f)\n}}\n"));
    has(&format!("{src}fun main() {{\n    val x: Foo::Item = 1\n}}\n"), "ambiguous");
}

#[test]
fn a_projection_needs_a_bound_that_declares_it() {
    has("trait C {\n    type Item\n}\nfun f<T>(x: &T): T::Item {\n    return x\n}\nfun main() {}\n", "no bound on `T` declares");
    has(
        "trait A {\n    type Item\n}\ntrait B {\n    type Item\n}\nfun f<T: A + B>(x: &T): T::Item {\n    return x\n}\nfun main() {}\n",
        "ambiguous",
    );
}

#[test]
fn equality_bindings_are_checked() {
    let src = "trait C {\n    type Item\n}\n";
    has(&format!("{src}fun f<T: C<Other = i32>>(x: &T) {{}}\nfun main() {{}}\n"), "has no associated type `Other`");
    has(&format!("{src}fun f<T: C<Item = i32, Item = bool>>(x: &T) {{}}\nfun main() {{}}\n"), "duplicate binding");
}

#[test]
fn impl_items_are_checked_against_the_trait() {
    let head = "trait C {\n    type Item\n}\nclass A {\n    v: i32\n}\n";
    has(&format!("{head}impl C for A {{\n}}\nfun main() {{}}\n"), "missing: type `Item`");
    has(&format!("{head}impl C for A {{\n    type Item = i32\n    type Other = i32\n}}\nfun main() {{}}\n"), "not an associated type");
    has(&format!("{head}impl C for A {{\n    type Item = i32\n    type Item = i32\n}}\nfun main() {{}}\n"), "duplicate associated type");
}

#[test]
fn an_equality_binding_may_name_another_projection() {
    let head = format!("{CONTAINER}fun same<A: Container, B: Container<Item = A::Item>>(a: &A, b: &B) {{}}\n");
    clean(&format!("{head}fun main() {{\n    val a = IntBox(1)\n    val b = IntBox(2)\n    same(&a, &b)\n}}\n"));
    has(
        &format!("{head}fun main() {{\n    val a = IntBox(1)\n    val b = Named(\"x\")\n    same(&a, &b)\n}}\n"),
        "expected the associated type to be",
    );
}
