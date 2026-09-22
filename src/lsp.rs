//! Map Bork parse and semantic errors to LSP diagnostics; hover + arena dump helpers.

use crate::dump::dump_arenas;
use crate::sema::{analyze, ArenaNode, ArenaReport, BindingInfo, BindingRole, Ownership, SemaError};
use crate::{parse, Error};
use lalrpop_util::ParseError;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

pub fn byte_offset_to_position(source: &str, byte_offset: usize) -> Position {
    let offset = byte_offset.min(source.len());
    let mut line = 0u32;
    let mut character = 0u32;

    for ch in source[..offset].chars() {
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}

fn byte_range(source: &str, start: usize, end: usize) -> Range {
    let start = start.min(source.len());
    let mut end = end.min(source.len()).max(start);
    if end == start && end < source.len() {
        end += source[end..].chars().next().map_or(0, char::len_utf8);
    }
    Range {
        start: byte_offset_to_position(source, start),
        end: byte_offset_to_position(source, end),
    }
}

fn expected_suffix(expected: &[String]) -> String {
    if expected.is_empty() {
        String::new()
    } else {
        format!("; expected {}", expected.join(", "))
    }
}

pub fn parse_error_to_diagnostic(source: &str, error: &Error) -> Diagnostic {
    let (range, message) = match error {
        ParseError::InvalidToken { location } => {
            (byte_range(source, *location, *location), "invalid token".into())
        }
        ParseError::UnrecognizedEof { location, expected } => (
            byte_range(source, *location, *location),
            format!("unexpected end of file{}", expected_suffix(expected)),
        ),
        ParseError::UnrecognizedToken {
            token: (start, token, end),
            expected,
        } => (
            byte_range(source, *start, *end),
            format!("unexpected token `{token}`{}", expected_suffix(expected)),
        ),
        ParseError::ExtraToken {
            token: (start, token, end),
        } => (
            byte_range(source, *start, *end),
            format!("unexpected extra token `{token}`"),
        ),
        ParseError::User { error } => (
            byte_range(source, source.len(), source.len()),
            (*error).to_string(),
        ),
    };

    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("bork".into()),
        message,
        ..Diagnostic::default()
    }
}

fn sema_error_to_diagnostic(source: &str, error: &SemaError) -> Diagnostic {
    let range = error
        .span
        .map(|s| byte_range(source, s.start, s.end))
        .unwrap_or_else(|| byte_range(source, 0, 0));
    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("bork".into()),
        message: error.message.clone(),
        ..Diagnostic::default()
    }
}

pub enum Analysis {
    ParseError { diagnostics: Vec<Diagnostic> },
    Ok {
        report: ArenaReport,
        diagnostics: Vec<Diagnostic>,
    },
}

pub fn analyze_source(source: &str) -> Analysis {
    match parse(source) {
        Err(error) => Analysis::ParseError {
            diagnostics: vec![parse_error_to_diagnostic(source, &error)],
        },
        Ok(program) => {
            let (report, errors) = analyze(&program);
            Analysis::Ok {
                report,
                diagnostics: errors
                    .iter()
                    .map(|e| sema_error_to_diagnostic(source, e))
                    .collect(),
            }
        }
    }
}

pub fn diagnostics_for_source(source: &str) -> Vec<Diagnostic> {
    match analyze_source(source) {
        Analysis::ParseError { diagnostics } | Analysis::Ok { diagnostics, .. } => diagnostics,
    }
}

pub fn dump_arenas_for_source(source: &str) -> Result<String, String> {
    match analyze_source(source) {
        Analysis::ParseError { diagnostics } => Err(diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_else(|| "parse error".into())),
        Analysis::Ok {
            report,
            diagnostics,
        } => {
            let mut text = dump_arenas(&report);
            if !diagnostics.is_empty() {
                text.push_str("\n# semantic errors\n");
                for d in &diagnostics {
                    text.push_str(&format!("# {}\n", d.message));
                }
            }
            Ok(text)
        }
    }
}

fn word_at(source: &str, position: Position) -> Option<String> {
    let line = source.split('\n').nth(position.line as usize)?;
    let mut utf16 = 0u32;
    let mut byte = 0usize;
    for ch in line.chars() {
        if utf16 >= position.character {
            break;
        }
        utf16 += ch.len_utf16() as u32;
        byte += ch.len_utf8();
    }
    let mut s = byte.min(line.len());
    while s > 0 {
        let prev = line[..s].chars().next_back()?;
        if prev.is_ascii_alphanumeric() || prev == '_' {
            s -= prev.len_utf8();
        } else {
            break;
        }
    }
    let mut e = byte.min(line.len());
    while e < line.len() {
        let c = line[e..].chars().next()?;
        if c.is_ascii_alphanumeric() || c == '_' {
            e += c.len_utf8();
        } else {
            break;
        }
    }
    (s < e).then(|| line[s..e].to_string())
}

fn position_to_byte_offset(source: &str, position: Position) -> Option<usize> {
    let mut line = 0u32;
    let mut character = 0u32;
    for (i, ch) in source.char_indices() {
        if line == position.line && character == position.character {
            return Some(i);
        }
        if ch == '\n' {
            if line == position.line {
                return Some(i);
            }
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }
    (line == position.line && character >= position.character).then_some(source.len())
}

fn span_contains(span: crate::span::Span, offset: usize) -> bool {
    let end = span.end.max(span.start.saturating_add(1));
    offset >= span.start && offset < end
}

fn find_binding<'a>(
    node: &'a ArenaNode,
    name: &str,
    offset: usize,
) -> Option<(&'a str, &'a BindingInfo)> {
    if let Some(hit) = find_span_hit(node, name, offset) {
        return Some(hit);
    }
    let mut best: Option<(&'a str, &'a BindingInfo, usize)> = None;
    walk_local_decls(node, name, offset, &mut best);
    best.map(|(label, info, _)| (label, info))
}

fn find_span_hit<'a>(
    node: &'a ArenaNode,
    name: &str,
    offset: usize,
) -> Option<(&'a str, &'a BindingInfo)> {
    for child in &node.children {
        if let Some(hit) = find_span_hit(child, name, offset) {
            return Some(hit);
        }
    }
    node.bindings.iter().find_map(|b| {
        (b.name == name && b.span.is_some_and(|s| span_contains(s, offset)))
            .then_some((node.label.as_str(), b))
    })
}

fn walk_local_decls<'a>(
    node: &'a ArenaNode,
    name: &str,
    offset: usize,
    best: &mut Option<(&'a str, &'a BindingInfo, usize)>,
) {
    for b in &node.bindings {
        if b.name != name || b.role != BindingRole::Decl || !matches!(b.ownership, Ownership::Local)
        {
            continue;
        }
        let Some(span) = b.span else {
            continue;
        };
        if span.start > offset {
            continue;
        }
        if best.is_none_or(|(_, _, start)| span.start >= start) {
            *best = Some((node.label.as_str(), b, span.start));
        }
    }
    for child in &node.children {
        walk_local_decls(child, name, offset, best);
    }
}

pub fn hover_for_source(source: &str, position: Position) -> Option<String> {
    let name = word_at(source, position)?;
    let offset = position_to_byte_offset(source, position)?;
    let Analysis::Ok { report, .. } = analyze_source(source) else {
        return None;
    };
    for root in &report.roots {
        if let Some((arena, info)) = find_binding(root, &name, offset) {
            let own = info.ownership.hover_label();
            return Some(format!("`{name}` in arena `{arena}`\nOwnership: {own}"));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_offset_maps_to_line_and_character() {
        let source = "fun main() {\n  return 1\n}";
        let offset = source.find('1').unwrap();
        assert_eq!(
            byte_offset_to_position(source, offset),
            Position {
                line: 1,
                character: 9
            }
        );
    }

    #[test]
    fn multibyte_utf8_counts_utf16_units() {
        let source = "val x = \"é𝄞\"\n";
        let offset = source.find('𝄞').unwrap();
        assert_eq!(
            byte_offset_to_position(source, offset),
            Position {
                line: 0,
                character: 10
            }
        );
    }

    #[test]
    fn bad_snippet_produces_nonempty_diagnostic_in_range() {
        let source = "fun main() {\n val x = (1 +\n)\n}";
        let diagnostics = diagnostics_for_source(source);
        assert_eq!(diagnostics.len(), 1);
        assert!(!diagnostics[0].message.is_empty());
        assert!(diagnostics[0].range.start.line <= diagnostics[0].range.end.line);
    }

    #[test]
    fn valid_sample_has_no_diagnostics() {
        assert!(diagnostics_for_source(crate::MVP_SAMPLE).is_empty());
    }

    #[test]
    fn move_error_produces_sema_diagnostic() {
        let source = r#"
fun main() {
    var s: String = "hi"
    {
        val t = s
    }
}
"#;
        let diags = diagnostics_for_source(source);
        assert!(
            diags.iter().any(|d| d.message.contains("not Copy")),
            "{diags:?}"
        );
    }

    #[test]
    fn dump_arenas_for_sample_ok() {
        let text = dump_arenas_for_source(crate::MVP_SAMPLE).unwrap();
        assert!(text.contains("fun main"));
    }

    #[test]
    fn sema_diagnostic_points_at_use_not_decl() {
        let source = "fun main() {\n    var s: String = \"a\"\n    val s2: String = \"b\"\n    {\n        val t = s\n    }\n}\n";
        let diags = diagnostics_for_source(source);
        let d = diags
            .iter()
            .find(|d| d.message.contains("not Copy"))
            .expect("err");
        assert_eq!(d.range.start.line, 4, "{d:?}");
    }

    #[test]
    fn hover_resolves_function_param() {
        let source = "fun add(x: Int): Int {\n    return x\n}\n";
        let x_off = source.find('x').unwrap();
        let pos = byte_offset_to_position(source, x_off);
        let hover = hover_for_source(source, pos).expect("hover");
        assert!(hover.contains('`'), "{hover}");
        assert!(hover.contains("Ownership"), "{hover}");
    }
}
