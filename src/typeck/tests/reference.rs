use crate::parse;
use crate::typeck::check;

#[test]
fn reference_param_requires_borrow_at_call_site() {
    let src = r#"
fun bump(buf: &[i32; 2]) {
    buf[0] = 1
}
fun main() {
    var a: [i32; 2] = [1, 2]
    bump(a)
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(
        diags.iter().any(|d| d.message.contains("expects borrow")),
        "{diags:?}"
    );
}

#[test]
fn borrow_argument_to_owned_param_is_rejected() {
    let src = r#"
fun own(buf: [i32; 2]) {
    buf[0] = 1
}
fun main() {
    var a: [i32; 2] = [1, 2]
    own(&a)
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(
        diags.iter().any(|d| {
            d.message.contains("borrow") && d.message.contains("owned")
        }),
        "{diags:?}"
    );
}

#[test]
fn reference_call_with_ampersand_typechecks() {
    let src = r#"
fun bump(buf: &[i32; 2]) {
    buf[0] = 1
}
fun main() {
    var a: [i32; 2] = [1, 2]
    bump(&a)
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(
        !diags.iter().any(|d| d.message.contains("expects borrow")),
        "{diags:?}"
    );
}

#[test]
fn val_local_with_ref_type_and_borrow_init() {
    let src = r#"
fun main() {
    var a: [i32; 2] = [1, 2]
    {
        val p: &[i32; 2] = &a
        p[0] = 9
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn inferred_view_rejects_index_assign() {
    let src = r#"
fun main() {
    var a: [i32; 2] = [1, 2]
    {
        val b = &a
        b[0] = 9
    }
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(
        diags
            .iter()
            .any(|d| d.message.contains("inferred view")),
        "{diags:?}"
    );
}

#[test]
fn reference_param_can_reborrow_to_nested_call() {
    let src = r#"
fun inner(buf: &[i32; 2]) {
    buf[0] = 1
}
fun outer(p: &[i32; 2]) {
    inner(&p)
}
fun main() {
    var a: [i32; 2] = [1, 2]
    outer(&a)
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(
        diags
            .iter()
            .all(|d| !d.message.contains("borrow has type") && !d.message.contains("expects borrow")),
        "{diags:?}"
    );
}

#[test]
fn reference_param_index_read_and_assign() {
    let src = r#"
fun read(buf: &[i32; 3], i: i32): i32 {
    return buf[i]
}
fun write(buf: &[i32; 3]) {
    buf[2] = 9
}
fun main(): i32 {
    var a: [i32; 3] = [1, 2, 3]
    write(&a)
    return read(&a, 1)
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(
        diags
            .iter()
            .all(|d| !d.message.contains("indexing requires an array type")),
        "{diags:?}"
    );
}

#[test]
fn string_ref_param_rejects_reassignment() {
    let src = r#"
fun reseat(msg: &String) {
    msg = "other"
}
fun main() {
    var s: String = "hi"
    reseat(&s)
}
"#;
    let prog = parse(src).unwrap();
    let (_, _, diags) = check(&prog);
    assert!(
        diags.iter().any(|d| d.message.contains("borrow binding")),
        "{diags:?}"
    );
}

fn messages(src: &str) -> Vec<String> {
    crate::frontend::check(src).diagnostics.into_iter().map(|d| d.message).collect()
}

const PRELUDE: &str = "class P {\n    x: i32\n}\nclass Holder<T> {\n    v: T\n}\nfun id<T>(x: T): T {\n    return x\n}\n";

#[test]
fn explicit_borrow_type_argument_is_rejected() {
    let m = messages(&format!("{PRELUDE}fun main() {{\n    val p = P(1)\n    val q = id::<&P>(&p)\n}}\n"));
    assert!(m.iter().any(|d| d.contains("cannot be a type argument of `id`")), "{m:?}");
}

#[test]
fn written_borrow_type_argument_is_rejected() {
    for ty in ["Holder<&P>", "Option<&P>"] {
        let m = messages(&format!("{PRELUDE}fun f(h: {ty}) {{\n}}\nfun main() {{\n}}\n"));
        assert!(m.iter().any(|d| d.contains("cannot be a type argument")), "{ty}: {m:?}");
    }
}

#[test]
fn inferred_borrow_type_argument_is_rejected() {
    let m = messages(&format!("{PRELUDE}fun main() {{\n    val p = P(1)\n    val q = id(&p)\n}}\n"));
    assert!(!m.is_empty(), "{m:?}");
    let m = messages(&format!("{PRELUDE}fun main() {{\n    val p = P(1)\n    val h = Holder(&p)\n}}\n"));
    assert!(m.iter().any(|d| d.contains("cannot be a type argument of `Holder`")), "constructor: {m:?}");
}

#[test]
fn enum_variant_borrow_type_argument_is_rejected() {
    let m = messages(&format!("{PRELUDE}enum E<T> {{\n    A(v: T)\n}}\nfun main() {{\n    val p = P(1)\n    val e = E.A(&p)\n}}\n"));
    assert!(m.iter().any(|d| d.contains("cannot be a type argument of `E`")), "{m:?}");
}
