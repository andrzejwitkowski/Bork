//! Generic declarations (#43): scopes, validation, and `Param` types in HIR. Bodies and
//! instantiation are #44, so every generic item still ends with a diagnostic.
use crate::ast::BindingKind;
use crate::frontend;
use crate::ty::{ParamIdx, TyData};

fn run(src: &str) -> (crate::typeck::Typeck, Vec<String>) {
    let program = crate::parse(src).expect("parses");
    let typeck = crate::typeck::check_with_members(&program);
    let messages = typeck.diagnostics.iter().map(|d| d.message.clone()).collect();
    (typeck, messages)
}

fn has(messages: &[String], needle: &str) {
    assert!(messages.iter().any(|m| m.contains(needle)), "{needle}: {messages:?}");
}

#[test]
fn generic_function_signature_uses_param_types() {
    let (t, messages) = run("fun pair<A, B>(a: A, b: B): B { return b }\nfun main() {}\n");
    has(&messages, "body of generic `pair` is not checked yet");
    let f = t.hir.functions.iter().find(|f| f.name == "pair").unwrap();
    assert_eq!(f.generics, ["A", "B"]);
    let kinds: Vec<_> = f.params.iter().map(|p| t.tcx.data(p.ty).clone()).collect();
    assert_eq!(kinds, [TyData::Param(ParamIdx(0)), TyData::Param(ParamIdx(1))]);
    assert_eq!(*t.tcx.data(f.return_ty), TyData::Param(ParamIdx(1)));
    assert!(f.params.iter().all(|p| p.kind == BindingKind::Val));
}

#[test]
fn generic_class_fields_and_methods_use_param_types() {
    let src = "class Box<T> {\n    v: T\n    fun get(): T { return v }\n    fun swap<U>(u: U): U { return u }\n}\nfun main() {}\n";
    let (t, messages) = run(src);
    has(&messages, "generic class `Box` is declaration-only");
    let class = &t.hir.classes[0];
    assert_eq!(class.generics, ["T"]);
    assert_eq!(*t.tcx.data(class.fields[0].ty), TyData::Param(ParamIdx(0)));
    let get = t.hir.functions.iter().find(|f| f.name == "Box.get").unwrap();
    assert_eq!(get.generics, ["T"]);
    assert_eq!(*t.tcx.data(get.return_ty), TyData::Param(ParamIdx(0)));
    let swap = t.hir.functions.iter().find(|f| f.name == "Box.swap").unwrap();
    assert_eq!(swap.generics, ["T", "U"]);
    assert_eq!(*t.tcx.data(swap.return_ty), TyData::Param(ParamIdx(1)));
}

#[test]
fn param_works_under_ref_vec_array_and_nullable() {
    let (t, messages) = run("fun f<T>(a: &T, b: Vec<T>, c: [T; 2], d: T?) {}\nfun main() {}\n");
    assert_eq!(messages.len(), 1, "{messages:?}");
    assert_eq!(t.hir.functions[0].params.len(), 4);
}

#[test]
fn declaration_errors_are_reported() {
    for (src, needle) in [
        ("fun f<T, T>() {}\nfun main() {}\n", "duplicate type parameter `T`"),
        ("class C<T> {\n    fun m<T>() {}\n}\nfun main() {}\n", "shadows the class type parameter `T`"),
        ("fun f<i32>() {}\nfun main() {}\n", "conflicts with the type `i32`"),
        ("class P {\n    n: i32\n}\nfun f<P>() {}\nfun main() {}\n", "conflicts with the type `P`"),
        ("fun f(x: U) {}\nfun main() {}\n", "unknown named type `U`"),
        ("fun f<T>(x: T<i32>) {}\nfun main() {}\n", "type parameter `T` takes no type arguments"),
        ("class P {\n    n: i32\n}\nfun f(x: P<i32>) {}\nfun main() {}\n", "`P` takes no type arguments"),
        ("class B<T> {\n    v: T\n}\nfun f(x: B) {}\nfun main() {}\n", "`B` expects 1 type argument, got 0"),
        ("class B<T> {\n    v: T\n}\nfun f(x: B<i32, i32>) {}\nfun main() {}\n", "`B` expects 1 type argument, got 2"),
        ("class B<T> {\n    v: T\n}\nfun f(x: B<i32>) {}\nfun main() {}\n", "instantiating generic types is not supported yet"),
    ] {
        let (_, messages) = run(src);
        has(&messages, needle);
    }
}

#[test]
fn generic_programs_never_reach_sema() {
    let result = frontend::check("fun id<T>(x: T): T { return x }\nfun main() {}\n");
    assert!(result.hir.is_none() && !result.diagnostics.is_empty());
}

#[test]
fn type_arguments_on_a_plain_class_and_turbofish_are_rejected() {
    let (_, m) = run("class Box {\n    v: i32\n}\nfun main() {\n    val b: Box<i32> = Box(1)\n}\n");
    has(&m, "`Box` takes no type arguments");
    let (_, m) = run("fun f(a: i32) {}\nfun main() {\n    f::<i32>(1)\n}\n");
    has(&m, "explicit type arguments");
}
