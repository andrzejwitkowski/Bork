use bork::frontend;
use bork::diag::Phase;

#[test]
fn parses_array_type_and_literal() {
    let source = r#"
fun main(): i32 {
    val a: [i32; 3] = [1, 2, 3]
    return a[1]
}
"#;
    let result = frontend::check(source);
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn slice_and_length_typecheck() {
    let source = r#"
fun main(): i32 {
    val a = [10, 20, 30]
    val b = a[0..2]
    return b.length
}
"#;
    let result = frontend::check(source);
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn array_literal_length_mismatch_errors() {
    let source = r#"
fun main() {
    val a: [i32; 2] = [1, 2, 3]
}
"#;
    let result = frontend::check(source);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("3 elements") && d.message.contains("expected 2")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn var_array_string_element_is_shared_read() {
    let source = r#"
fun main() {
    var outer: [String; 2] = ["a", "b"]
    val inner = outer[0]
    println(inner)
}
"#;
    let result = frontend::check(source);
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn whole_array_move() {
    let source = r#"
fun main() {
    var outer: [String; 2] = ["a", "b"]
    val inner = move outer
    println(inner[0])
}
"#;
    let result = frontend::check(source);
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn val_array_element_is_shared_read() {
    let source = r#"
fun main() {
    val outer: [String; 2] = ["a", "b"]
    val inner = outer[0]
    println(inner)
}
"#;
    let result = frontend::check(source);
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn rejects_slice_escaping_inner_region() {
    let source = r#"
fun main() {
    var s: [String; 2] = ["p", "q"]
    {
        var inner: [String; 2] = ["x", "y"]
        s = inner[0..2]
    }
}
"#;
    let result = frontend::check(source);
    assert!(
        result.diagnostics.iter().any(|d| {
            d.phase == Phase::Ownership && d.message.contains("inner region")
        }),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn whole_array_assign_requires_matching_length() {
    let source = r#"
fun main() {
    var outer: [i32; 2] = [1, 2]
    val inner: [i32; 3] = [1, 2, 3]
    outer = inner
}
"#;
    let result = frontend::check(source);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("[i32; 3]") && d.message.contains("[i32; 2]")),
        "{:?}",
        result.diagnostics
    );
}
