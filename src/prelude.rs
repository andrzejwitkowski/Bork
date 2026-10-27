//! The prelude (#54): `Option` and `Result` are ordinary generic enums written in Bork, and the
//! explicit conversions between them and `T?` / `Ref<T>` are ordinary generic functions. Nothing
//! converts implicitly.
//!
//! `Option` is always added, because `T?` is `Option<T>`. Every other chunk is added only when the
//! program mentions one of its names, and a chunk is skipped when the program defines that name
//! itself (a program with its own `Option` cannot use `T?`). Prelude items are numbered after the user's and kept out
//! of the span index, because their spans point into prelude text, not into the user's file.

use crate::ast::Program;

struct Chunk {
    path: &'static str,
    /// Names that pull the chunk in, and that a user definition of the same name suppresses.
    names: &'static [&'static str],
    /// Chunks whose names this one uses.
    needs: &'static [usize],
    source: &'static str,
}

const OPTION: usize = 0;

const CHUNKS: [Chunk; 3] = [
    Chunk { path: concat!(env!("CARGO_MANIFEST_DIR"), "/src/prelude/option.bork"), names: &["Option"], needs: &[], source: include_str!("prelude/option.bork") },
    Chunk { path: concat!(env!("CARGO_MANIFEST_DIR"), "/src/prelude/result.bork"), names: &["Result"], needs: &[], source: include_str!("prelude/result.bork") },
    Chunk { path: concat!(env!("CARGO_MANIFEST_DIR"), "/src/prelude/option_conv.bork"), names: &["optionFromRef", "optionToRef"], needs: &[OPTION], source: include_str!("prelude/option_conv.bork") },
];

pub(crate) fn source_file(name: &str) -> Option<&'static str> {
    CHUNKS.iter().find(|c| c.names.contains(&name)).map(|c| c.path)
}

pub(crate) fn source_text(path: &str) -> &'static str {
    CHUNKS.iter().find(|c| c.path == path).expect("prelude source file").source
}

fn defines(program: &Program, name: &str) -> bool {
    program.classes.iter().any(|c| c.name == name)
        || program.enums.iter().any(|e| e.name.name == name)
        || program.functions.iter().any(|f| f.name == name)
        || program.constants.iter().any(|c| c.name.name == name)
}

/// `source` with comments and string literals blanked out, so only code can name a prelude item.
fn code_only(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '/' if chars.peek() == Some(&'/') => {
                while chars.peek().is_some_and(|&n| n != '\n') {
                    chars.next();
                }
            }
            '"' => {
                while let Some(n) = chars.next() {
                    match n {
                        '\\' => {
                            chars.next();
                        }
                        '"' => break,
                        _ => {}
                    }
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

fn mentions(code: &str, name: &str) -> bool {
    code.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|word| word == name)
}

/// Appends the prelude chunks `source` uses to `program`, after the user's items. `next_id` is the
/// first free `AstId`.
pub fn add_prelude(program: &mut Program, source: &str, next_id: u32) {
    let code = code_only(source);
    let shadowed = |chunk: &Chunk| chunk.names.iter().any(|name| defines(program, name));
    let mut wanted = [false; CHUNKS.len()];
    for (index, chunk) in CHUNKS.iter().enumerate() {
        // A chunk is skipped when the user defines its names, or one it builds on: the user's
        // `Option` has its own shape, so the conversions written for the prelude's cannot work.
        let blocked = shadowed(chunk) || chunk.needs.iter().any(|&need| shadowed(&CHUNKS[need]));
        wanted[index] = !blocked && (index == OPTION || chunk.names.iter().any(|name| mentions(&code, name)));
    }
    for (index, chunk) in CHUNKS.iter().enumerate() {
        if wanted[index] {
            chunk.needs.iter().for_each(|&need| wanted[need] = true);
        }
    }
    let mut extra = Program { constants: Vec::new(), classes: Vec::new(), enums: Vec::new(), functions: Vec::new() };
    for (index, chunk) in CHUNKS.iter().enumerate().filter(|(index, _)| wanted[*index]) {
        let parsed = crate::parse_unnumbered(chunk.source)
            .unwrap_or_else(|err| panic!("prelude chunk {index} does not parse: {err:?}"));
        extra.constants.extend(parsed.constants);
        extra.classes.extend(parsed.classes);
        extra.enums.extend(parsed.enums);
        extra.functions.extend(parsed.functions);
    }
    crate::ast_number::number_nodes_from(&mut extra, next_id);
    program.constants.extend(extra.constants);
    program.classes.extend(extra.classes);
    program.enums.extend(extra.enums);
    program.functions.extend(extra.functions);
}

#[cfg(test)]
mod tests {
    use crate::frontend::check;

    fn enum_names(source: &str) -> Vec<String> {
        let result = check(source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.hir.expect("clean").enums.into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn option_is_always_present_because_t_question_is_option_and_result_only_when_named() {
        let result = check("fun main() {\n    val x: i32? = None\n}\n");
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        assert_eq!(result.hir.unwrap().enums.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Option"]);
        assert_eq!(enum_names("fun main() {}\n"), ["Option"]);
    }

    #[test]
    fn each_chunk_checks_clean_on_its_own() {
        assert_eq!(enum_names("fun f(o: Option<i32>) {}\nfun main() {}\n"), ["Option"]);
        assert_eq!(enum_names("fun f(r: Result<i32, String>) {}\nfun main() {}\n"), ["Option", "Result"]);
        let conv = "class Node {\n    v: i32\n}\nfun main() {\n    val n = Node(1)\n    val r: Ref<Node> = n\n    val o = optionFromRef::<Node>(r)\n}\n";
        assert_eq!(enum_names(conv), ["Option"]);
    }

    #[test]
    fn a_conversion_pulls_in_the_enum_it_returns() {
        let names = enum_names("class N {\n    v: i32\n}\nfun main() {\n    val n = N(1)\n    val r: Ref<N> = n\n    val o = optionFromRef::<N>(r)\n}\n");
        assert_eq!(names, ["Option"]);
    }

    #[test]
    fn a_user_definition_wins_and_the_prelude_is_not_added() {
        assert_eq!(enum_names("enum Option {\n    A,\n    B,\n}\nfun main() {\n    val o = Option.A\n}\n"), ["Option"]);
        let result = check("class Result {\n    v: i32\n}\nfun main() {\n    val r = Result(1)\n}\n");
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        assert_eq!(result.hir.unwrap().enums.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["Option"]);
    }

    #[test]
    fn names_in_comments_and_strings_do_not_load_the_conversions() {
        let source = "// optionFromRef and Result are only mentioned here\nfun main() {\n    println(\"Result optionFromRef\")\n}\n";
        assert_eq!(enum_names(source), ["Option"]);
        let result = check(source);
        assert!(result.hir.unwrap().functions.iter().all(|f| !f.name.starts_with("option")));
    }

    #[test]
    fn a_user_option_also_blocks_the_conversions_built_on_the_prelude_one() {
        let source = "enum Option {\n    A,\n}\n// optionFromRef is only a word in this comment\nfun main() {\n    val o = Option.A\n}\n";
        assert_eq!(enum_names(source), ["Option"]);
        let code = "enum Option {\n    A,\n}\nclass Node {\n    v: i32\n}\nfun main() {\n    val n = Node(1)\n    val r: Ref<Node> = n\n    val o = optionFromRef::<Node>(r)\n}\n";
        let result = check(code);
        assert!(!result.is_ok(), "the prelude conversion must not be spliced onto a user `Option`");
    }

    #[test]
    fn prelude_items_stay_out_of_the_span_index() {
        let source = "fun f(o: Option<i32>) {}\nfun main() {}\n";
        let result = check(source);
        assert!(result.is_ok());
        assert!(result.node_index.iter().all(|(span, _)| span.end <= source.len()));
        let ids: std::collections::HashSet<_> = result.node_index.iter().map(|(_, id)| *id).collect();
        assert_eq!(ids.len(), result.node_index.len());
    }

    #[test]
    fn some_and_none_are_variants_where_an_option_is_expected_and_unchanged_for_nullables() {
        let ok = "fun f(n: i32): Option<i32> {\n    if (n > 0) {\n        return Some(n)\n    }\n    return None\n}\nfun g(): i32? {\n    return Some(1)\n}\nfun main() {}\n";
        assert_eq!(enum_names(ok), ["Option"]);
    }
}
