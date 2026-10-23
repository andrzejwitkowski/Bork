//! Generic declarations (#43), generic functions (#44) and generic classes and methods (#45).
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
fn generic_class_fields_and_methods_use_param_types() {
    let src = "class Box<T> {\n    v: T\n    fun get(): T { return v }\n    fun swap<U>(u: U): U { return u }\n}\nfun main() {}\n";
    let t = clean(src);
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
    ] {
        let (_, messages) = run(src);
        has(&messages, needle);
    }
}

#[test]
fn type_arguments_on_a_plain_class_and_turbofish_are_rejected() {
    let (_, m) = run("class Box {\n    v: i32\n}\nfun main() {\n    val b: Box<i32> = Box(1)\n}\n");
    has(&m, "`Box` takes no type arguments");
    let (_, m) = run("fun f(a: i32) {}\nfun main() {\n    f::<i32>(1)\n}\n");
    has(&m, "explicit type arguments");
}

#[test]
fn reference_parameters_of_generic_items_must_be_val() {
    let (_, m) = run("fun f<T>(var x: &T) {}\nfun main() {}\n");
    has(&m, "reference parameter must be `val`");
    let (_, m) = run("class B<T> {\n    v: T\n    fun m(var x: &T) {}\n}\nfun main() {}\n");
    has(&m, "reference parameter must be `val`");
}

fn clean(src: &str) -> crate::typeck::Typeck {
    let (typeck, messages) = run(src);
    assert!(messages.is_empty(), "{messages:?}");
    typeck
}

fn main_decl_ty(src: &str, index: usize) -> String {
    let typeck = clean(src);
    let main = typeck.hir.functions.iter().find(|f| f.name == "main").unwrap();
    let crate::hir::HirStmt::VarDecl { ty, .. } = &main.body.stmts[index] else {
        panic!("expected a declaration");
    };
    typeck.tcx.display(*ty).to_string()
}

const ID: &str = "fun id<T>(x: T): T {\n    return x\n}\nclass Point {\n    x: i32\n}\n";

#[test]
fn generic_function_signature_uses_param_types() {
    let t = clean("fun pair<A, B>(a: A, b: B): B { return b }\nfun main() {}\n");
    let f = t.hir.functions.iter().find(|f| f.name == "pair").unwrap();
    assert_eq!(f.generics, ["A", "B"]);
    let kinds: Vec<_> = f.params.iter().map(|p| t.tcx.data(p.ty).clone()).collect();
    assert_eq!(kinds, [TyData::Param(ParamIdx(0)), TyData::Param(ParamIdx(1))]);
    assert_eq!(*t.tcx.data(f.return_ty), TyData::Param(ParamIdx(1)));
}

#[test]
fn param_works_under_ref_vec_array_and_nullable() {
    clean("fun f<T>(a: &T, b: Vec<T>, c: [T; 2], d: T?) {}\nfun main() {}\n");
}

#[test]
fn identity_instantiates_per_call() {
    let head = format!("{ID}fun main() {{\n");
    assert_eq!(main_decl_ty(&format!("{head}    val a = id(1)\n}}\n"), 0), "i32");
    assert_eq!(main_decl_ty(&format!("{head}    val a = id(\"s\")\n}}\n"), 0), "String");
    assert_eq!(main_decl_ty(&format!("{head}    val p = Point(1)\n    val a = id(p)\n}}\n"), 1), "Point");
}

#[test]
fn generic_body_can_name_its_parameters() {
    clean("fun f<T>(x: T): T {\n    val y: T = x\n    return y\n}\nfun main() {}\n");
}

#[test]
fn explicit_and_inferred_arguments_agree() {
    let head = format!("{ID}fun main() {{\n");
    assert_eq!(main_decl_ty(&format!("{head}    val a = id::<i64>(1)\n}}\n"), 0), "i64");
    assert_eq!(main_decl_ty(&format!("{head}    val a: i64 = id(1)\n}}\n"), 0), "i64");
}

#[test]
fn explicit_argument_must_match_the_value() {
    let (_, m) = run(&format!("{ID}fun main() {{\n    id::<i64>(\"s\")\n}}\n"));
    has(&m, "argument 1 to `id` has type String, expected i64");
}

#[test]
fn explicit_argument_count_is_checked() {
    let (_, m) = run(&format!("{ID}fun main() {{\n    id::<i32, i32>(1)\n}}\n"));
    has(&m, "`id` expects 1 type argument, got 2");
}

#[test]
fn turbofish_on_a_non_generic_function_is_rejected() {
    let (_, m) = run("fun f(a: i32) {}\nfun main() {\n    f::<i32>(1)\n}\n");
    has(&m, "`f` is not generic; it takes no explicit type arguments");
}

#[test]
fn unconstrained_type_parameter_cannot_be_inferred() {
    // The recursive call in `make` is fixed by the return type; only the one in `main` is open.
    let (_, m) = run("fun make<T>(): T {\n    return make()\n}\nfun main() {\n    make()\n}\n");
    has(&m, "cannot infer type parameter `T` of `make`; specify it explicitly");
    assert_eq!(m.iter().filter(|m| m.contains("cannot infer")).count(), 1, "{m:?}");
}

#[test]
fn inference_from_the_expected_type_and_later_use() {
    let src = "fun make<T>(): T {\n    return make()\n}\nfun take(a: u8) {}\nfun main() {\n    val x = make()\n    take(x)\n}\n";
    assert_eq!(main_decl_ty(src, 0), "u8");
}

#[test]
fn two_parameters_infer_independently() {
    let src = "fun first<A, B>(a: A, b: B): A { return a }\nfun main() {\n    val v = first(1, \"s\")\n}\n";
    assert_eq!(main_decl_ty(src, 0), "i32");
}

#[test]
fn opaque_parameter_rejects_operators_and_members() {
    for (body, needle) in [
        ("return x + x", "arithmetic operands must have the same numeric type, got T and T"),
        ("val b = x < x\n    return x", "ordered comparison operands"),
        ("val b = x == x\n    return x", "equality operands must have the same type, got T and T"),
        ("val n = x.size\n    return x", "unknown field `size` on type T"),
    ] {
        let (_, m) = run(&format!("fun f<T>(x: T): T {{\n    {body}\n}}\nfun main() {{}}\n"));
        has(&m, needle);
        has(&m, "a type parameter has no bounds yet");
    }
}

#[test]
fn nullable_of_nullable_instantiation_is_an_error() {
    let src = "fun opt<T>(x: T?): T? { return x }\nfun main() {\n    opt::<String?>(None)\n}\n";
    let (_, m) = run(src);
    has(&m, "make a nullable of a nullable");
}

#[test]
fn generic_functions_do_not_pass_the_codegen_gate() {
    let typeck = clean("fun id<T>(x: T): T { return x }\nfun main() {}\n");
    let gate = crate::codegen_gate::gate(&typeck.hir, &typeck.tcx);
    assert!(gate.iter().any(|d| d.message.contains("generic function `id` is not compiled yet")), "{gate:?}");
}

#[test]
fn generic_classes_do_not_pass_the_codegen_gate() {
    let typeck = clean("class Box<T> {\n    v: T\n}\nfun main() {}\n");
    let gate = crate::codegen_gate::gate(&typeck.hir, &typeck.tcx);
    assert!(gate.iter().any(|d| d.message.contains("generic class `Box` is not compiled yet")), "{gate:?}");
}

const PAIR: &str = "class Pair<A, B> {\n    a: A\n    b: B\n    fun first(): A { return a }\n    fun with<C>(c: C): C { return c }\n}\n";

#[test]
fn constructor_infers_class_arguments() {
    let src = format!("{PAIR}fun main() {{\n    val p = Pair(1, \"a\")\n}}\n");
    assert_eq!(main_decl_ty(&src, 0), "Pair<i32, String>");
}

#[test]
fn expected_type_fixes_class_arguments() {
    let src = format!("{PAIR}fun main() {{\n    val p: Pair<i64, String> = Pair(1, \"a\")\n}}\n");
    assert_eq!(main_decl_ty(&src, 0), "Pair<i64, String>");
}

#[test]
fn field_and_method_types_are_substituted() {
    let src = format!(
        "{PAIR}fun main() {{\n    val p = Pair(1, \"a\")\n    val x = p.a\n    val s = p.b\n    val f = p.first()\n    val w = p.with(true)\n}}\n"
    );
    assert_eq!(main_decl_ty(&src, 1), "i32");
    assert_eq!(main_decl_ty(&src, 2), "String");
    assert_eq!(main_decl_ty(&src, 3), "i32");
    assert_eq!(main_decl_ty(&src, 4), "bool");
}

#[test]
fn mismatched_class_arguments_are_rejected() {
    let src = format!("{PAIR}fun main() {{\n    val p: Pair<String, String> = Pair(1, \"a\")\n}}\n");
    let (_, m) = run(&src);
    assert!(!m.is_empty(), "expected a mismatch");
}

#[test]
fn constructor_with_unfixed_parameter_cannot_infer() {
    let (_, m) = run("class Holder<T> {\n    v: T?\n}\nfun main() {\n    val h = Holder(None)\n}\n");
    has(&m, "cannot infer type parameter `T` of `Holder`");
}

#[test]
fn recursion_through_ref_is_allowed() {
    clean("class Node<T> {\n    v: T\n    next: Ref<Node<T>>\n}\nfun main() {}\n");
}

#[test]
fn by_value_recursion_reports_the_cycle_path() {
    let (_, m) = run("class Bad<T> {\n    b: Bad<T>\n}\nfun main() {}\n");
    has(&m, "class `Bad<T>` has a recursive by-value layout (Bad<T> -> Bad<T>)");
    let (_, m) = run("class A<T> {\n    b: B<T>\n}\nclass B<T> {\n    a: A<T>\n}\nfun main() {}\n");
    has(&m, "A<T> -> B<T> -> A<T>");
}

#[test]
fn recursion_through_a_generic_instantiation_is_found() {
    let (_, m) = run("class Box<T> {\n    v: T\n}\nclass A {\n    b: Box<A>\n}\nfun main() {}\n");
    has(&m, "class `A` has a recursive by-value layout (A -> Box<A> -> A)");
}

#[test]
fn expanding_generic_recursion_terminates() {
    let (_, m) = run("class N<T> {\n    x: N<N<T>>\n}\nfun main() {}\n");
    has(&m, "recursive by-value layout");
}
