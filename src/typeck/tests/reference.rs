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
