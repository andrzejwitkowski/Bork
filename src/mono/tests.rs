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
    let (result, _) = run(src);
    let diagnostics = result.expect_err("polymorphic recursion must be rejected");
    assert!(diagnostics.iter().any(|d| d.message.contains("polymorphic recursion")), "{diagnostics:?}");
}

#[test]
fn a_parameter_missing_from_the_signature_is_reported() {
    let (result, _) = run("fun f<T>(): i32 {\n    return 1\n}\nfun main() {\n    f::<i32>()\n}\n");
    let diagnostics = result.expect_err("an undeterminable parameter must be reported");
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
