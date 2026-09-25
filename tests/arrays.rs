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
fn index_assign_i32_typechecks() {
    let source = r#"
fun main(): i32 {
    var a: [i32; 3] = [1, 2, 3]
    var i = 1
    a[i] = 9
    return a[1]
}
"#;
    let result = frontend::check(source);
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn index_assign_to_val_array_is_rejected() {
    let source = r#"
fun main() {
    val a: [i32; 3] = [1, 2, 3]
    a[1] = 9
}
"#;
    let result = frontend::check(source);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("immutable") && d.message.contains("val")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn index_assign_string_requires_move() {
    let source = r#"
fun main() {
    var a: [String; 1] = ["a"]
    var s = "b"
    a[0] = s
}
"#;
    let result = frontend::check(source);
    assert!(
        result.diagnostics.iter().any(|d| d.message.contains("move")),
        "{:?}",
        result.diagnostics
    );
}

#[test]
fn index_assign_moved_string_typechecks() {
    let source = r#"
fun main() {
    var a: [String; 1] = ["a"]
    var s = "b"
    a[0] = move s
    println(a[0])
}
"#;
    let result = frontend::check(source);
    assert!(result.is_ok(), "{:?}", result.diagnostics);
}

#[test]
fn index_assign_rejects_bad_index_and_value_types() {
    let bad_index = r#"
fun main() {
    var a: [i32; 1] = [1]
    a["x"] = 2
}
"#;
    let index_result = frontend::check(bad_index);
    assert!(
        index_result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("index") && d.message.contains("i32")),
        "{:?}",
        index_result.diagnostics
    );

    let bad_value = r#"
fun main() {
    var a: [i32; 1] = [1]
    a[0] = "no"
}
"#;
    let value_result = frontend::check(bad_value);
    assert!(
        value_result
            .diagnostics
            .iter()
            .any(|d| d.message.contains("element") && d.message.contains("i32")),
        "{:?}",
        value_result.diagnostics
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
