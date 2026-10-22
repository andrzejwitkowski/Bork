//! Generic syntax parses (#42) but typeck rejects it with a span until it is checked.
use super::diags_of;

fn messages(src: &str) -> Vec<String> {
    diags_of(src).into_iter().map(|d| d.message).collect()
}

#[test]
fn generic_function_is_rejected() {
    let m = messages("fun id<T>(x: T): T { return x }\nfun main() {}\n");
    assert!(m.iter().any(|m| m.contains("generic parameter `T` is not supported yet")), "{m:?}");
}

#[test]
fn generic_class_is_rejected() {
    let m = messages("class Box<T> {\n    v: i32\n}\nfun main() {}\n");
    assert!(m.iter().any(|m| m.contains("generic parameter `T` is not supported yet")), "{m:?}");
}

#[test]
fn type_arguments_are_rejected() {
    let m = messages("class Box {\n    v: i32\n}\nfun main() {\n    val b: Box<i32> = Box(1)\n}\n");
    assert!(m.iter().any(|m| m.contains("type arguments on `Box` are not supported yet")), "{m:?}");
}

#[test]
fn turbofish_is_rejected() {
    let m = messages("fun f(a: i32) {}\nfun main() {\n    f::<i32>(1)\n}\n");
    assert!(m.iter().any(|m| m.contains("explicit type arguments")), "{m:?}");
}
