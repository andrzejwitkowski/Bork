use super::super::*;
use crate::parse;

fn errors(src: &str) -> Vec<SemaError> {
    analyze(&parse(src).expect("parse")).1
}

#[test]
fn method_call_borrows_receiver_without_moving_it() {
    let errs = errors(
        "class P {\n    x: i32\n    fun get(): i32 {\n        return this.x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    val a = p.get()\n    val q = move p\n    return a\n}",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn method_call_after_move_is_error() {
    let errs = errors(
        "class P {\n    x: i32\n    fun get(): i32 {\n        return this.x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    val q = move p\n    return p.get()\n}",
    );
    assert!(errs.iter().any(|e| e.message.contains("use of `p` after move")), "{errs:?}");
}

#[test]
fn method_call_in_loop_reborrows_this() {
    let errs = errors(
        "class P {\n    x: i32\n    fun one(): i32 {\n        return 1\n    }\n    fun sum(): i32 {\n        var t = 0\n        var i = 0\n        while (i < 3) {\n            if (i > 0) {\n                t = t + this.one() + one()\n            }\n            i = i + 1\n        }\n        return t\n    }\n}\nfun main() {}",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn receiver_and_borrow_arg_same_name() {
    let errs = errors(
        "class P {\n    x: i32\n    fun merge(o: &P): i32 {\n        return this.x + o.x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.merge(&p)\n}",
    );
    assert!(errs.is_empty(), "{errs:?}");
}

#[test]
fn this_is_a_borrow_param_of_the_method_region() {
    let (report, errs) = analyze(&parse(
        "class P {\n    x: i32\n    fun get(): i32 {\n        return this.x\n    }\n}\nfun main() {}",
    ).unwrap());
    assert!(errs.is_empty(), "{errs:?}");
    let method = report.roots.iter().find(|r| r.label == "fun P.get").expect("method root");
    assert!(method.bindings.iter().any(|b| b.name == "this"));
}

#[test]
fn bare_field_use_observes_this() {
    let (report, errs) = analyze(&parse(
        "class P {\n    name: String\n    fun f(): i32 {\n        name = \"b\"\n        return name.length\n    }\n}\nfun main() {}",
    ).unwrap());
    assert!(errs.is_empty(), "{errs:?}");
    let method = report.roots.iter().find(|r| r.label == "fun P.f").unwrap();
    assert!(method.observations.iter().any(|o| o.name == "this"), "{:?}", method.observations);
}

#[test]
fn bare_field_assign_requires_move_like_this_field() {
    let class = "class P {\n    name: String\n    fun f(s: String) {\n        ";
    let tail = "\n    }\n}\nfun main() {}";
    let bare = errors(&format!("{class}name = s{tail}"));
    let via_this = errors(&format!("{class}this.name = s{tail}"));
    assert!(via_this.iter().any(|e| e.message.contains("move")), "{via_this:?}");
    assert!(bare.iter().any(|e| e.message.contains("move")), "{bare:?}");
    let ok = errors(&format!("{class}name = move s{tail}"));
    assert!(ok.is_empty(), "{ok:?}");
}
