use super::super::*;
use crate::parse;

#[test]
fn call_bare_val_into_val_param_ok() {
    let src = r#"
fun sink(s: String): Int {
    return 0
}
fun main() {
    val s: String = "hi"
    sink(s)
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn call_bare_val_into_var_param_errors() {
    let src = r#"
fun sink(var s: String): Int { return 0 }
fun main() {
    val s: String = "hi"
    sink(s)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.iter().any(|e| e.message.contains("move")), "{errs:?}");
}

#[test]
fn call_bare_var_into_val_param_errors() {
    let src = r#"
fun sink(s: String): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(s)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.iter().any(|e| e.message.contains("move")), "{errs:?}");
}

#[test]
fn call_bare_var_into_var_param_errors() {
    let src = r#"
fun sink(var s: String): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(s)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("move")),
        "{errs:?}"
    );
}

#[test]
fn call_copy_argument_ok() {
    let src = r#"
fun id(n: Int): Int {
    return n
}
fun main() {
    val n = 1
    val m = id(n)
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let id = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun id"))
        .expect("id");
    assert!(
        id.bindings.iter().any(|b| {
            b.name == "n" && matches!(b.ownership, Ownership::Local)
        }),
        "callee still has its own Local param: {:?}",
        id.bindings
    );
}

#[test]
fn call_copy_into_var_param_ok() {
    let src = r#"
fun sink(var n: Int): Int { return n }
fun main() {
    var n = 1
    sink(n)
    val m = n
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn call_move_into_val_param_consumes() {
    let src = r#"
fun sink(s: String): Int { return 0 }
fun main() {
    val s: String = "hi"
    sink(move s)
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "{errs:?}"
    );
}

#[test]
fn call_move_into_var_param_consumes() {
    let src = r#"
fun sink(var s: String): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(move s)
    val t = s
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "{errs:?}"
    );
}

#[test]
fn call_two_move_into_var_params_marks_callers_and_callee() {
    let src = r#"
fun f(var a: String, var b: String) { }
fun main() {
    var x: String = "X"
    var y: String = "Y"
    f(move x, move y)
    val t = x
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("after move")),
        "x must be moved after call: {errs:?}"
    );
    let f = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun f"))
        .expect("f");
    assert!(
        f.bindings.iter().any(|b| {
            b.name == "a" && matches!(b.ownership, Ownership::Local)
        }),
        "formal a should be Local: {:?}",
        f.bindings
    );
    assert!(
        f.bindings.iter().any(|b| {
            b.name == "b" && matches!(b.ownership, Ownership::Local)
        }),
        "formal b should be Local: {:?}",
        f.bindings
    );
    let main = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun main"))
        .expect("main");
    assert!(
        main.bindings.iter().any(|b| {
            b.name == "x" && matches!(b.ownership, Ownership::Moved { .. })
        }),
        "caller x should be Moved: {:?}",
        main.bindings
    );
    assert!(
        main.bindings.iter().any(|b| {
            b.name == "y" && matches!(b.ownership, Ownership::Moved { .. })
        }),
        "caller y should be Moved: {:?}",
        main.bindings
    );
}

#[test]
fn helper_string_param_shared_in_nested_block() {
    let src = r#"
fun use(s: String): Int {
    {
        val t = s
    }
    return 0
}
fun main() {
    val s: String = "hi"
    use(s)
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    let use_fn = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun use"))
        .expect("use");
    let nested = &use_fn.children[0];
    assert!(
        nested.observations.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Shared { .. })
        }),
        "param s is Val, so nested read should be Shared: {:?}",
        nested.observations
    );
}

#[test]
fn helper_var_param_cannot_share_across_nested_region() {
    let src = r#"
fun touch(var s: String): Int {
    {
        val t = s
    }
    return 0
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("not Copy")),
        "var String in helper must not cross arenas without move: {errs:?}"
    );
}

#[test]
fn main_calls_helper_return_into_var() {
    let src = r#"
fun next(n: Int): Int {
    return n + 1
}
fun main() {
    var x = 0
    x = next(x)
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn main_calls_helper_val_param_return_to_val() {
    let src = r#"
fun echo(s: String): String {
    return s
}
fun main() {
    val s: String = "hi"
    val out = echo(s)
}
"#;
    let prog = parse(src).unwrap();
    let (report, errs) = analyze(&prog);
    assert!(errs.is_empty(), "{errs:?}");
    assert_eq!(report.roots.len(), 2);
    let echo = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun echo"))
        .expect("echo");
    assert!(
        echo.bindings.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Local)
        }),
        "param s should be Local in echo: {:?}",
        echo.bindings
    );
    let main = report
        .roots
        .iter()
        .find(|r| r.label.contains("fun main"))
        .expect("main");
    assert!(
        main.bindings.iter().any(|b| b.name == "out"),
        "main should bind return into val out: {:?}",
        main.bindings
    );
}

#[test]
fn nested_some_bare_var_into_var_param_errors() {
    let src = r#"
fun sink(var s: String?): Int { return 0 }
fun main() {
    var s: String = "hi"
    sink(Some(s))
}
"#;
    let prog = parse(src).unwrap();
    let (_, errs) = analyze(&prog);
    assert!(
        errs.iter().any(|e| e.message.contains("move")),
        "Some(s) into var param needs move: {errs:?}"
    );
}
