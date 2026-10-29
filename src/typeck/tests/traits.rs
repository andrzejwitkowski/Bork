//! Trait and impl declarations (#55).

fn diags(src: &str) -> Vec<String> {
    let program = crate::parse(src).expect("parses");
    crate::typeck::check_with_members(&program).diagnostics.into_iter().map(|d| d.message).collect()
}

fn has(src: &str, needle: &str) {
    let messages = diags(src);
    assert!(messages.iter().any(|m| m.contains(needle)), "{needle}: {messages:?}");
}

const SHAPE: &str = "trait Shape {\n    fun area(): f64\n    fun scaled(k: f64): f64\n}\nclass C {\n    r: f64\n}\n";

#[test]
fn matching_impls_are_clean() {
    let src = format!(
        "{SHAPE}impl Shape for C {{\n    fun area(): f64 {{ return 1.0 }}\n    fun scaled(k: f64): f64 {{ return k }}\n}}\nfun main() {{}}\n"
    );
    assert_eq!(diags(&src), Vec::<String>::new());
}

#[test]
fn generic_impls_trait_args_and_method_generics_match() {
    let src = "trait Conv<U> {\n    fun conv<V>(u: U, v: V): V\n}\nclass Box<T> {\n    v: T\n}\nenum E<T> {\n    A(v: T)\n}\nimpl<T> Conv<T> for Box<T> {\n    fun conv<W>(u: T, v: W): W { return v }\n}\nimpl<T> Conv<i32> for E<T> {\n    fun conv<W>(u: i32, v: W): W { return v }\n}\nfun main() {}\n";
    assert_eq!(diags(src), Vec::<String>::new());
}

#[test]
fn impl_item_errors() {
    let body = |items: &str| format!("{SHAPE}impl Shape for C {{\n{items}}}\nfun main() {{}}\n");
    has(&body("    fun area(): f64 { return 1.0 }\n"), "missing: `scaled`");
    has(&body(""), "missing: `area`, `scaled`");
    has(
        &body("    fun area(): f64 { return 1.0 }\n    fun scaled(k: f64): f64 { return k }\n    fun x() {}\n"),
        "method `x` is not a member of trait `Shape`",
    );
    has(&body("    fun area(): f64 { return 1.0 }\n    fun scaled(): f64 { return 1.0 }\n"), "has 0 parameters but trait `Shape` declares 1");
    has(&body("    fun area(): f64 { return 1.0 }\n    fun scaled(k: i32): f64 { return 1.0 }\n"), "parameter `k` of `scaled` has type `i32` but trait `Shape` declares `f64`");
    has(&body("    fun area(): i32 { return 1 }\n    fun scaled(k: f64): f64 { return k }\n"), "`area` returns `i32` but trait `Shape` declares `f64`");
    has(&body("    fun area(): f64 { return 1.0 }\n    fun area(): f64 { return 1.0 }\n    fun scaled(k: f64): f64 { return k }\n"), "duplicate method `area`");
    has(&body("    fun area<T>(): f64 { return 1.0 }\n    fun scaled(k: f64): f64 { return k }\n"), "has 1 type parameter(s) but trait `Shape` declares 0");
}

#[test]
fn self_in_an_impl_means_the_impl_type() {
    let src = "trait Dup {\n    fun dup(other: Self): Self\n}\nclass C {\n    r: f64\n}\nclass D {\n    r: f64\n}\nimpl Dup for C {\n    fun dup(other: Self): Self { return other }\n}\nfun main() {}\n";
    assert_eq!(diags(src), Vec::<String>::new());
    let wrong = src.replace("fun dup(other: Self): Self { return other }", "fun dup(other: Self): D { return other }");
    has(&wrong, "`dup` returns `D` but trait `Dup` declares `C`");
}

#[test]
fn header_errors() {
    has("class C {\n    r: f64\n}\nimpl Nope for C {\n}\nfun main() {}\n", "unknown trait `Nope`");
    has("trait T<U> {\n}\nclass C {\n    r: f64\n}\nimpl T for C {\n}\nfun main() {}\n", "takes 1 type argument(s) but 0");
    has("trait T {\n}\nimpl T for i32 {\n}\nfun main() {}\n", "only classes and enums");
    has("trait T {\n}\ntrait T {\n}\nfun main() {}\n", "conflicts with another type, trait or function");
    has("class T {\n    r: f64\n}\ntrait T {\n}\nfun main() {}\n", "conflicts with another type, trait or function");
    has("trait T {\n    fun a()\n    fun a()\n}\nfun main() {}\n", "duplicate method `a` in trait `T`");
}

#[test]
fn trait_and_impl_def_ids() {
    let program = crate::parse(&format!("{SHAPE}impl Shape for C {{\n    fun area(): f64 {{ return 1.0 }}\n    fun scaled(k: f64): f64 {{ return k }}\n}}\nfun main() {{}}\n")).unwrap();
    let defs = crate::defs::DefTable::build(&program);
    let t = defs.lookup_trait("Shape").unwrap();
    assert_eq!(defs.get(t).kind, crate::defs::DefKind::Trait);
    let m = defs.lookup_trait_item(t, "area").unwrap();
    assert_eq!(defs.get(m).kind, crate::defs::DefKind::TraitMethod);
    assert_eq!(defs.get(m).owner, Some(t));
    assert_eq!(defs.get(defs.impl_id(0)).kind, crate::defs::DefKind::Impl);
    assert_eq!(defs.get(defs.impl_method_id(0, 1)).kind, crate::defs::DefKind::ImplMethod);
}
