use super::*;
use crate::frontend::check;

fn run(src: &str) -> (Result<Mono, Vec<Diagnostic>>, TyCtx) {
    let mut checked = check(src);
    assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
    let hir = checked.hir.take().expect("clean check produces HIR");
    let report = checked.report.take().expect("clean check produces a report");
    let result = monomorphize(&hir, &mut checked.tcx, &report);
    (result, checked.tcx)
}

/// The diagnostics `bork check` reports for a program whose instances cannot be built. Both
/// `check` and `monomorphize` go through `collect_instances`, so this covers `bork build` too.
fn rejected(src: &str) -> Vec<Diagnostic> {
    let checked = check(src);
    assert!(!checked.diagnostics.is_empty(), "expected the program to be rejected");
    assert!(checked.hir.is_none(), "a rejected program has no HIR");
    checked.diagnostics
}

fn mono(src: &str) -> Mono {
    run(src).0.unwrap_or_else(|d| panic!("{d:?}"))
}

fn names(mono: &Mono) -> Vec<&str> {
    mono.instances.iter().map(|i| i.name.as_str()).collect()
}

fn function_names(mono: &Mono) -> Vec<&str> {
    mono.hir.functions.iter().map(|f| f.name.as_str()).collect()
}

const ID: &str = "fun id<T>(x: T): T {\n    return x\n}\n";
const PAIR: &str = "class Pair<A, B> {\n    a: A\n    b: B\n    fun first(): A {\n        return a\n    }\n}\n";

#[test]
fn each_distinct_argument_list_is_one_instance() {
    let m = mono(&format!("{ID}fun main() {{\n    val a = id(1)\n    val b = id(2)\n    val c = id(\"s\")\n}}\n"));
    assert_eq!(names(&m), ["main", "id<i32>", "id<String>"]);
    assert_eq!(function_names(&m), names(&m));
}

#[test]
fn unreachable_generics_produce_no_instance() {
    let m = mono(&format!("{ID}fun unused<T>(x: T) {{}}\nfun other() {{}}\nfun main() {{}}\n"));
    assert_eq!(names(&m), ["main"]);
}

#[test]
fn generic_calls_generic() {
    let src = format!("{ID}fun twice<T>(x: T): T {{\n    return id(id(x))\n}}\nfun main() {{\n    val a = twice(1)\n}}\n");
    assert_eq!(names(&mono(&src)), ["main", "twice<i32>", "id<i32>"]);
}

#[test]
fn mutual_recursion_terminates() {
    let src = "fun ping<T>(x: T, n: i32) {\n    if (n > 0) {\n        pong(x, n - 1)\n    }\n}\n\
               fun pong<T>(x: T, n: i32) {\n    if (n > 0) {\n        ping(x, n - 1)\n    }\n}\n\
               fun main() {\n    ping(1, 3)\n}\n";
    assert_eq!(names(&mono(src)), ["main", "ping<i32>", "pong<i32>"]);
}

#[test]
fn methods_and_classes_are_instantiated_per_argument_list() {
    let src = format!(
        "{PAIR}fun main() {{\n    val p = Pair(1, \"a\")\n    val q = Pair(true, true)\n    val x = p.first()\n    val y = q.first()\n}}\n"
    );
    let m = mono(&src);
    assert_eq!(names(&m), ["main", "Pair.first<i32,String>", "Pair.first<bool,bool>"]);
    let mut classes: Vec<&str> = m.hir.classes.iter().map(|c| c.name.as_str()).collect();
    classes.sort();
    assert_eq!(classes, ["Pair<bool,bool>", "Pair<i32,String>"]);
    assert!(m.hir.classes.iter().all(|c| c.generics.is_empty()));
}

#[test]
fn instance_bodies_use_the_instance_names() {
    let m = mono(&format!("{ID}fun main() {{\n    val a = id(1)\n}}\n"));
    let main = &m.hir.functions[0];
    let crate::hir::HirStmt::VarDecl { value, .. } = &main.body.stmts[0] else { panic!("expected a declaration") };
    let HirExprKind::Call { callee, .. } = &value.kind else { panic!("expected a call") };
    assert!(matches!(&callee.kind, HirExprKind::Ident { name, .. } if name == "id<i32>"));
}

#[test]
fn output_has_one_report_root_per_function_and_unique_node_ids() {
    let mut m = mono(&format!("{ID}fun main() {{\n    val a = id(1)\n    val b = id(\"s\")\n}}\n"));
    let labels: Vec<&str> = m.report.roots.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(labels, ["fun main", "fun id<i32>", "fun id<String>"]);
    let mut ids = Vec::new();
    for function in &mut m.hir.functions {
        hir::rewrite(&mut function.body, &mut |ty| ty, &mut |e| ids.push(e.id));
    }
    let unique: HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len(), "NodeIds must be unique across instances");
    assert!(!ids.contains(&crate::hir::NodeId::DUMMY));
}

#[test]
fn polymorphic_recursion_is_rejected() {
    let src = "class Box<T> {\n    v: T\n}\nfun f<T>(var x: T) {\n    f(Box(move x))\n}\nfun main() {\n    f(1)\n}\n";
    let diagnostics = rejected(src);
    assert!(diagnostics.iter().any(|d| d.message.contains("polymorphic recursion")), "{diagnostics:?}");
}

#[test]
fn a_parameter_missing_from_the_signature_is_reported() {
    let diagnostics = rejected("fun f<T>(): i32 {\n    return 1\n}\nfun main() {\n    f::<i32>()\n}\n");
    assert!(diagnostics.iter().any(|d| d.message.contains("not determined by its signature")), "{diagnostics:?}");
}

#[test]
fn no_type_parameter_survives() {
    let src = format!("{PAIR}{ID}fun main() {{\n    val p = Pair(1, \"a\")\n    val x = id(p.first())\n}}\n");
    let mut checked = check(&src);
    let (hir, report) = (checked.hir.take().unwrap(), checked.report.take().unwrap());
    let mut m = monomorphize(&hir, &mut checked.tcx, &report).unwrap();
    assert!(no_params_left(&mut m.hir, &checked.tcx));
    // The generic input still has them, so the check is not vacuous.
    let mut generic = hir.clone();
    assert!(!no_params_left(&mut generic, &checked.tcx));
}

#[test]
fn class_arguments_inside_a_plain_class_are_emitted() {
    let src = format!("{PAIR}class Holder {{\n    p: Pair<i32, i32>\n}}\nfun main() {{\n    val h = Holder(Pair(1, 2))\n}}\n");
    let m = mono(&src);
    let classes: Vec<&str> = m.hir.classes.iter().map(|c| c.name.as_str()).collect();
    assert!(classes.contains(&"Pair<i32,i32>") && classes.contains(&"Holder"), "{classes:?}");
}

#[test]
fn doubling_polymorphic_recursion_is_rejected_with_a_bounded_message() {
    let src = "class Two<A, B> {\n    a: A\n    b: B\n}\nfun f<T>(x: T) {\n    f(Two(x, x))\n}\nfun main() {\n    f(1)\n}\n";
    let started = std::time::Instant::now();
    let diagnostics = rejected(src);
    assert!(diagnostics.iter().any(|d| d.message.contains("polymorphic recursion")), "{diagnostics:?}");
    assert!(diagnostics.iter().all(|d| d.message.len() < 500), "message is not bounded");
    assert!(started.elapsed().as_secs() < 5, "took {:?}", started.elapsed());
}

#[test]
fn class_instance_name_reaches_as_far_as_record_name() {
    let src = format!("{PAIR}fun main() {{\n    val p = Pair(1, \"a\")\n    val r: Ref<Pair<i32, String>> = p\n}}\n");
    let mut checked = check(&src);
    let (hir, report) = (checked.hir.take().unwrap(), checked.report.take().unwrap());
    let main = hir.functions.iter().find(|f| f.name == "main").unwrap();
    let decl_ty = |index: usize| match &main.body.stmts[index] {
        crate::hir::HirStmt::VarDecl { ty, .. } => *ty,
        other => panic!("expected a declaration, got {other:?}"),
    };
    let (object, managed) = (decl_ty(0), decl_ty(1));
    let _ = monomorphize(&hir, &mut checked.tcx, &report).unwrap();
    let tcx = &checked.tcx;
    assert_eq!(class_instance_name(tcx, object).as_deref(), Some("Pair<i32,String>"));
    assert_eq!(class_instance_name(tcx, managed), None, "a managed Ref has no record name");
    assert_eq!(tcx.record_name(managed), None);
}

#[test]
fn an_instance_that_makes_ref_of_a_non_class_is_rejected() {
    let src = "fun g<T>(x: T): Ref<T> {\n    return None\n}\nfun main() {\n    val r = g(1)\n}\n";
    let diagnostics = rejected(src);
    assert!(
        diagnostics.iter().any(|d| d.message.contains("g<i32>") && d.message.contains("managed Ref target")),
        "{diagnostics:?}"
    );
    assert!(diagnostics.iter().all(|d| d.span.is_some()), "{diagnostics:?}");
}

#[test]
fn an_instance_that_makes_vec_of_a_class_is_rejected() {
    let src = "class P {\n    n: i32\n}\nfun f<T>(x: T) {\n    var v = Vec<T>()\n}\nfun main() {\n    f(P(1))\n}\n";
    let diagnostics = rejected(src);
    assert!(diagnostics.iter().any(|d| d.message.contains("`Vec` element")), "{diagnostics:?}");
}

#[test]
fn a_class_instance_with_an_ill_formed_field_is_rejected() {
    let src = "class Cell<T> {\n    next: Ref<T>\n}\nfun main() {\n    val c: Cell<i32> = Cell(None)\n}\n";
    let diagnostics = rejected(src);
    assert!(diagnostics.iter().any(|d| d.message.contains("managed Ref target")), "{diagnostics:?}");
}

#[test]
fn well_formed_ref_and_vec_instances_are_still_accepted() {
    let src = "class P {\n    n: i32\n}\nfun g<T>(x: T): Ref<T> {\n    return None\n}\nfun f<T>(x: T) {\n    var v = Vec<T>()\n}\nfun main() {\n    val r = g(P(1))\n    f(1)\n    f(\"s\")\n}\n";
    let m = mono(src);
    assert!(names(&m).contains(&"g<P>") && names(&m).contains(&"f<i32>") && names(&m).contains(&"f<String>"));
}

#[test]
fn every_ill_formed_type_of_one_instance_is_reported() {
    let src = "class P {\n    n: i32\n}\nfun f<A, B>(a: A, b: B): Ref<A> {\n    var v = Vec<B>()\n    return None\n}\nfun main() {\n    val r = f(1, P(1))\n}\n";
    let diagnostics = rejected(src);
    let in_instance = |needle: &str| diagnostics.iter().filter(|d| d.message.contains("f<i32,P>") && d.message.contains(needle)).count();
    assert_eq!((in_instance("managed Ref target"), in_instance("`Vec` element")), (1, 1), "{diagnostics:?}");
}

#[test]
fn generic_enums_are_instantiated_per_argument_list() {
    let m = mono(
        "enum Maybe<T> {\n    Nothing,\n    Just(value: T),\n}\n\
         fun main() {\n    val a = Maybe.Just(1)\n    val b: Maybe<String> = Maybe.Just(\"s\")\n    val c: Maybe<i32> = Maybe.Nothing\n}\n",
    );
    let mut enums: Vec<&str> = m.hir.enums.iter().map(|e| e.name.as_str()).collect();
    enums.sort();
    assert_eq!(enums, ["Maybe<String>", "Maybe<i32>"]);
    let string = m.hir.enums.iter().find(|e| e.name == "Maybe<String>").unwrap();
    assert!(string.generics.is_empty());
    assert_eq!(string.variants[1].name, "Just");
    assert_eq!(string.variants[1].fields.len(), 1);
}

#[test]
fn enum_payloads_instantiate_the_classes_they_use() {
    let m = mono(&format!(
        "{PAIR}enum Holder {{\n    Has(p: Pair<i32, String>),\n    Empty,\n}}\nfun main() {{\n    val h = Holder.Empty\n}}\n"
    ));
    assert!(m.hir.classes.iter().any(|c| c.name == "Pair<i32,String>"));
    assert_eq!(m.hir.enums.len(), 1);
}

#[test]
fn a_parameter_that_only_appears_in_a_bound_is_not_monomorphized_yet() {
    // Typeck accepts `g::<C>()`; mono cannot recover `T` from the signature until dispatch (#58).
    let src = "trait Show {\n}\nclass C {\n    r: f64\n}\nimpl Show for C {\n}\nfun g<T: Show>() { }\nfun main() {\n    g::<C>()\n}\n";
    let diags = rejected(src);
    assert!(diags.iter().any(|d| d.message.contains("is not determined by its signature")), "{diags:?}");
}
