//! The prelude (#54): `Option` and `Result` are ordinary generic enums written in Bork, and the
//! explicit conversions between them and `T?` / `Ref<T>` are ordinary generic functions. Nothing
//! converts implicitly.
//!
//! A chunk is added to the parsed program only when the program mentions one of its names and does
//! not define that name itself, so a program that never uses the prelude is checked exactly as
//! before and a user's own `Option` wins. Prelude items are numbered after the user's and kept out
//! of the span index, because their spans point into prelude text, not into the user's file.

use crate::ast::Program;

struct Chunk {
    /// Names that pull the chunk in, and that a user definition of the same name suppresses.
    names: &'static [&'static str],
    /// Chunks whose names this one uses.
    needs: &'static [usize],
    source: &'static str,
}

const OPTION: usize = 0;

const CHUNKS: [Chunk; 3] = [
    Chunk { names: &["Option"], needs: &[], source: include_str!("prelude/option.bork") },
    Chunk { names: &["Result"], needs: &[], source: include_str!("prelude/result.bork") },
    Chunk { names: &["optionFrom", "optionFromRef", "optionToRef"], needs: &[OPTION], source: include_str!("prelude/option_conv.bork") },
];

fn defines(program: &Program, name: &str) -> bool {
    program.classes.iter().any(|c| c.name == name)
        || program.enums.iter().any(|e| e.name.name == name)
        || program.functions.iter().any(|f| f.name == name)
        || program.constants.iter().any(|c| c.name.name == name)
}

fn mentions(source: &str, name: &str) -> bool {
    source
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|word| word == name)
}

/// Appends the prelude chunks `source` uses to `program`, after the user's items. `next_id` is the
/// first free `AstId`.
pub fn add_prelude(program: &mut Program, source: &str, next_id: u32) {
    let mut wanted = [false; CHUNKS.len()];
    for (index, chunk) in CHUNKS.iter().enumerate() {
        let used = chunk.names.iter().any(|name| mentions(source, name));
        let shadowed = chunk.names.iter().any(|name| defines(program, name));
        if used && !shadowed {
            wanted[index] = true;
        }
    }
    for (index, chunk) in CHUNKS.iter().enumerate() {
        if wanted[index] {
            for &need in chunk.needs {
                let shadowed = CHUNKS[need].names.iter().any(|name| defines(program, name));
                wanted[need] = !shadowed;
            }
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
    fn a_program_that_never_names_the_prelude_gets_none_of_it() {
        let result = check("fun main() {\n    val x: i32? = None\n}\n");
        assert!(result.is_ok());
        let hir = result.hir.unwrap();
        assert!(hir.enums.is_empty());
        assert_eq!(hir.functions.len(), 1);
    }

    #[test]
    fn each_chunk_checks_clean_on_its_own() {
        assert_eq!(enum_names("fun f(o: Option<i32>) {}\nfun main() {}\n"), ["Option"]);
        assert_eq!(enum_names("fun f(r: Result<i32, String>) {}\nfun main() {}\n"), ["Result"]);
        let conv = "fun main() {\n    val x: i32? = None\n    val o: Option<i32> = optionFrom::<i32>(x)\n}\n";
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
        assert!(result.hir.unwrap().enums.is_empty());
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
