use crate::parse;
use crate::typeck::check;

#[test]
fn reference_param_requires_borrow_at_call_site() {
    let src = r#"
fun bump(buf: &rw [i32; 2]) {
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
fun bump(buf: &rw [i32; 2]) {
    buf[0] = 1
}
fun main() {
    var a: [i32; 2] = [1, 2]
    bump(&rw a)
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
        val p: &rw [i32; 2] = &rw a
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
fun inner(buf: &rw [i32; 2]) {
    buf[0] = 1
}
fun outer(p: &rw [i32; 2]) {
    inner(&rw p)
}
fun main() {
    var a: [i32; 2] = [1, 2]
    outer(&rw a)
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
fun write(buf: &rw [i32; 3]) {
    buf[2] = 9
}
fun main(): i32 {
    var a: [i32; 3] = [1, 2, 3]
    write(&rw a)
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

/// Asserts `body` reports the borrow rule exactly `times` for `owner`.
fn rejects_times(body: &str, owner: &str, times: usize) {
    let m = messages(&format!("{PRELUDE}enum E<T> {{\n    A(v: T)\n}}\n{body}"));
    let want = format!("cannot be a type argument of `{owner}`");
    assert_eq!(m.iter().filter(|d| d.contains(&want)).count(), times, "{owner}: {m:?}");
}

fn rejects(body: &str, owner: &str) {
    rejects_times(body, owner, 1);
}

#[test]
fn inferred_borrow_type_argument_is_rejected() {
    rejects("fun main() {\n    val p = P(1)\n    val q = id(&p)\n}\n", "id");
    rejects("fun main() {\n    val p = P(1)\n    val h = Holder(&p)\n}\n", "Holder");
}

#[test]
fn a_borrow_nested_in_an_inferred_type_argument_is_rejected() {
    // `T = Option<?X>` with `?X` bound to `&P`: found only by resolving the whole type.
    rejects("fun main() {\n    val p = P(1)\n    val h = Holder(Some(&p))\n}\n", "Holder");
}

#[test]
fn a_callback_taking_a_borrow_is_not_a_stored_borrow() {
    let m = messages(&format!("{PRELUDE}fun f(h: Holder<(&P) -> i32>) {{\n}}\nfun main() {{\n}}\n"));
    assert!(!m.iter().any(|d| d.contains("cannot be a type argument")), "{m:?}");
}

#[test]
fn an_annotated_borrow_type_argument_is_reported_at_the_annotation_and_the_constructor() {
    rejects_times("fun main() {\n    val p = P(1)\n    val h: Holder<&P> = Holder(&p)\n}\n", "Holder", 2);
}

#[test]
fn enum_variant_borrow_type_argument_is_rejected() {
    let m = messages(&format!("{PRELUDE}enum E<T> {{\n    A(v: T)\n}}\nfun main() {{\n    val p = P(1)\n    val e = E.A(&p)\n}}\n"));
    assert!(m.iter().any(|d| d.contains("cannot be a type argument of `E`")), "{m:?}");
}

#[test]
fn a_projection_that_resolves_to_a_borrow_is_a_rejected_type_argument() {
    let src = "trait Tr {\n    type Item\n}\nclass P {\n    x: i32\n}\nclass Holder<T> {\n    v: T\n}\nclass W {\n    n: i32\n}\nimpl Tr for W {\n    type Item = &P\n}\nfun f(h: Holder<W::Item>) {\n}\nfun main() {\n}\n";
    let m = messages(src);
    assert!(m.iter().any(|d| d.contains("cannot be a type argument of `Holder`")), "{m:?}");
}

fn rw_messages(items: &str, main_body: &str) -> Vec<String> {
    messages(&format!("class P {{\n    x: i32\n}}\nclass H<T> {{\n    v: T\n}}\n{items}fun main() {{\n{main_body}}}\n"))
}

#[test]
fn an_editable_borrow_is_accepted_where_it_is_passed_on_unchanged() {
    let m = rw_messages("fun w(buf: &rw [i32; 2]) {\n    buf[0] = 1\n}\n", "    var a: [i32; 2] = [1, 2]\n    w(&rw a)\n");
    assert!(m.is_empty(), "{m:?}");
}

#[test]
fn shared_and_editable_borrows_are_different_types() {
    let m = rw_messages("fun w(buf: &rw [i32; 2]) {\n}\n", "    var a: [i32; 2] = [1, 2]\n    w(&a)\n");
    assert!(m.iter().any(|d| d.contains("expected &rw [i32; 2]")), "{m:?}");
    let m = rw_messages("fun r(buf: &[i32; 2]) {\n}\n", "    var a: [i32; 2] = [1, 2]\n    r(&rw a)\n");
    assert!(m.iter().any(|d| d.contains("expected &[i32; 2]")), "{m:?}");
}

#[test]
fn every_borrow_restriction_also_covers_the_editable_borrow() {
    let rows: &[(&str, &str, &str)] = &[
        ("class C {\n    r: &rw P\n}\n", "", "cannot be stored in class fields"),
        ("enum E {\n    A(v: &rw P)\n}\n", "", "cannot be stored in enum payloads"),
        ("fun f(p: &rw P): &rw P {\n    return p\n}\n", "", "returning a reference is not supported"),
        ("", "    var a: [i32; 2] = [1, 2]\n    var x = &rw a\n", "to a reference type; use `val`"),
        ("fun f(v: Vec<&rw P>) {\n}\n", "", "`Vec` element must be"),
        ("fun f(v: Ref<&rw P>) {\n}\n", "", "managed Ref target must be"),
        ("fun f(h: H<&rw P>) {\n}\n", "", "cannot be a type argument of `H`"),
        ("fun id<T>(x: T): T {\n    return x\n}\n", "    var a: [i32; 2] = [1, 2]\n    val q = id::<&rw [i32; 2]>(&rw a)\n", "cannot be a type argument of `id`"),
    ];
    for (items, body, want) in rows {
        let m = rw_messages(items, body);
        assert!(m.iter().any(|d| d.contains(want)), "{want}: {m:?}");
    }
}

fn borrow_messages(items: &str, main_body: &str) -> Vec<String> {
    messages(&format!("{items}fun main() {{\n{main_body}}}\n"))
}

#[test]
fn a_write_needs_an_editable_borrow_on_a_var() {
    let m = borrow_messages("fun w(p: &[i32; 2]) {\n    p[0] = 1\n}\n", "");
    assert!(m.iter().any(|d| d.contains("cannot index-assign through the read-only borrow `p`")), "{m:?}");
    let m = borrow_messages("fun w(v: &Vec<i32>) {\n    v.push(1)\n}\n", "");
    assert!(m.iter().any(|d| d.contains("cannot mutate through the read-only borrow `v`")), "{m:?}");
    let m = borrow_messages("fun w(p: &rw [i32; 2]) {\n}\n", "    val a: [i32; 2] = [1, 2]\n    w(&rw a)\n");
    assert!(m.iter().any(|d| d.contains("cannot borrow `val` `a` as `&rw`; declare it `var`")), "{m:?}");
    let m = borrow_messages(
        "fun w(p: &rw [i32; 2]) {\n    p[0] = 1\n}\nfun v(p: &Vec<i32>) {\n}\n",
        "    var a: [i32; 2] = [1, 2]\n    w(&rw a)\n    var b = Vec<i32>()\n    v(&b)\n",
    );
    assert!(m.is_empty(), "{m:?}");
}

#[test]
fn a_val_array_can_be_borrowed_read_only() {
    let m = borrow_messages("fun r(p: &[i32; 2]): i32 {\n    return p[0]\n}\n", "    val a: [i32; 2] = [1, 2]\n    val x = r(&a)\n");
    assert!(m.is_empty(), "{m:?}");
}

#[test]
fn an_editable_borrow_can_be_passed_on_as_editable_or_downgraded_but_not_upgraded() {
    let readers = "fun r(p: &[i32; 2]): i32 {\n    return p[0]\n}\nfun w(p: &rw [i32; 2]) {\n    p[0] = 1\n}\n";
    let m = borrow_messages(&format!("{readers}fun f(p: &rw [i32; 2]): i32 {{\n    w(&rw p)\n    return r(&p)\n}}\n"), "");
    assert!(m.is_empty(), "{m:?}");
    let m = borrow_messages(&format!("{readers}fun f(p: &[i32; 2]) {{\n    w(&rw p)\n}}\n"), "");
    assert!(m.iter().any(|d| d.contains("cannot take `&rw` of the read-only borrow `p`")), "{m:?}");
}

#[test]
fn a_vec_field_cannot_be_mutated_through_a_read_only_borrow() {
    let holder = "class H {\n    items: Vec<i32>\n}\n";
    let m = borrow_messages(&format!("{holder}fun f(h: &H) {{\n    h.items.push(1)\n}}\n"), "");
    assert!(m.iter().any(|d| d.contains("read-only borrow `h` (through a field)")), "{m:?}");
    let m = borrow_messages(&format!("{holder}fun f(h: &rw H) {{\n    h.items.push(1)\n}}\n"), "");
    assert!(m.is_empty(), "{m:?}");
}

#[test]
fn borrowing_where_an_editable_borrow_is_expected_hints_at_rw() {
    let m = borrow_messages("fun w(p: &rw [i32; 2]) {\n}\n", "    var a: [i32; 2] = [1, 2]\n    w(&a)\n");
    assert!(m.iter().any(|d| d.contains("write `&rw` to borrow it editable")), "{m:?}");
}
