//! Map Bork parse and semantic errors to LSP diagnostics; hover + arena dump helpers.

use crate::dump::dump_arenas;
use crate::frontend;
use crate::sema::{ArenaNode, ArenaReport, BindingInfo};
use tower_lsp::lsp_types::{Diagnostic as LspDiagnostic, DiagnosticSeverity, Position, Range};

/// Line starts for LSP UTF-16 positions.
pub struct SourceMap<'a> {
    source: &'a str,
    line_starts: Vec<usize>,
}

impl<'a> SourceMap<'a> {
    pub fn new(source: &'a str) -> Self {
        let mut line_starts = vec![0];
        for (i, ch) in source.char_indices() {
            if ch == '\n' {
                line_starts.push(i + ch.len_utf8());
            }
        }
        Self {
            source,
            line_starts,
        }
    }

    fn line_range(&self, line: usize) -> Option<(usize, usize)> {
        let start = *self.line_starts.get(line)?;
        let end = self
            .line_starts
            .get(line + 1)
            .copied()
            .unwrap_or(self.source.len());
        Some((start, end))
    }

    fn line_body(&self, line: usize) -> Option<(usize, &'a str)> {
        let (start, end) = self.line_range(line)?;
        let body = self.source[start..end]
            .strip_suffix('\n')
            .unwrap_or(&self.source[start..end]);
        Some((start, body))
    }

    pub fn offset_to_position(&self, byte_offset: usize) -> Position {
        let offset = byte_offset.min(self.source.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let start = self.line_starts[line];
        let mut character = 0u32;
        for ch in self.source[start..offset].chars() {
            if ch == '\n' {
                break;
            }
            character += ch.len_utf16() as u32;
        }
        Position {
            line: line as u32,
            character,
        }
    }

    pub fn position_to_offset(&self, position: Position) -> Option<usize> {
        let (start, line_body) = self.line_body(position.line as usize)?;
        let mut utf16 = 0u32;
        for (i, ch) in line_body.char_indices() {
            if utf16 >= position.character {
                return Some(start + i);
            }
            utf16 += ch.len_utf16() as u32;
        }
        if position.character <= utf16 {
            Some(start + line_body.len())
        } else if position.line as usize + 1 >= self.line_starts.len() {
            Some(self.source.len())
        } else {
            None
        }
    }

    pub fn word_at(&self, position: Position) -> Option<String> {
        let (_, line_text) = self.line_body(position.line as usize)?;
        let mut utf16 = 0u32;
        let mut byte = 0usize;
        for ch in line_text.chars() {
            if utf16 >= position.character {
                break;
            }
            utf16 += ch.len_utf16() as u32;
            byte += ch.len_utf8();
        }
        let mut s = byte.min(line_text.len());
        while s > 0 {
            let prev = line_text[..s].chars().next_back()?;
            if prev.is_ascii_alphanumeric() || prev == '_' {
                s -= prev.len_utf8();
            } else {
                break;
            }
        }
        let mut e = byte.min(line_text.len());
        while e < line_text.len() {
            let c = line_text[e..].chars().next()?;
            if c.is_ascii_alphanumeric() || c == '_' {
                e += c.len_utf8();
            } else {
                break;
            }
        }
        (s < e).then(|| line_text[s..e].to_string())
    }
}

pub fn byte_offset_to_position(source: &str, byte_offset: usize) -> Position {
    SourceMap::new(source).offset_to_position(byte_offset)
}

fn byte_range(source: &str, start: usize, end: usize) -> Range {
    let map = SourceMap::new(source);
    let start = start.min(source.len());
    let mut end = end.min(source.len()).max(start);
    if end == start && end < source.len() {
        end += source[end..].chars().next().map_or(0, char::len_utf8);
    }
    Range {
        start: map.offset_to_position(start),
        end: map.offset_to_position(end),
    }
}

fn frontend_diagnostic_to_lsp(source: &str, diagnostic: &crate::diag::Diagnostic) -> LspDiagnostic {
    let range = diagnostic
        .span
        .map(|s| byte_range(source, s.start, s.end))
        .unwrap_or_else(|| byte_range(source, 0, 0));
    let phase = match diagnostic.phase {
        crate::diag::Phase::Parse => "parse",
        crate::diag::Phase::Ownership => "ownership",
        crate::diag::Phase::Type => "type",
        crate::diag::Phase::Codegen => "codegen",
    };
    let severity = match diagnostic.severity {
        crate::diag::Severity::Error => DiagnosticSeverity::ERROR,
        crate::diag::Severity::Warning => DiagnosticSeverity::WARNING,
    };
    LspDiagnostic {
        range,
        severity: Some(severity),
        source: Some("bork".into()),
        message: format!("{phase}: {}", diagnostic.message),
        ..LspDiagnostic::default()
    }
}

/// One `frontend::check` run, rendered for LSP.
pub struct Analysis {
    report: Option<ArenaReport>,
    diagnostics: Vec<LspDiagnostic>,
}

impl Analysis {
    pub fn diagnostics(&self) -> &[LspDiagnostic] {
        &self.diagnostics
    }

    pub fn report(&self) -> Option<&ArenaReport> {
        self.report.as_ref()
    }
}

pub fn analyze_source(source: &str) -> Analysis {
    let result = frontend::check(source);
    Analysis {
        report: result.report,
        diagnostics: result
            .diagnostics
            .iter()
            .map(|diagnostic| frontend_diagnostic_to_lsp(source, diagnostic))
            .collect(),
    }
}

pub fn diagnostics_for_source(source: &str) -> Vec<LspDiagnostic> {
    analyze_source(source).diagnostics
}

pub fn dump_from_analysis(analysis: &Analysis) -> Result<String, String> {
    let Some(report) = analysis.report() else {
        return Err(analysis
            .diagnostics
            .first()
            .map(|d| d.message.clone())
            .unwrap_or_else(|| "parse error".into()));
    };
    let mut text = dump_arenas(report);
    if !analysis.diagnostics.is_empty() {
        text.push_str("\n# semantic errors\n");
        for d in &analysis.diagnostics {
            text.push_str(&format!("# {}\n", d.message));
        }
    }
    Ok(text)
}

pub fn dump_arenas_for_source(source: &str) -> Result<String, String> {
    dump_from_analysis(&analyze_source(source))
}

fn span_contains(span: crate::span::Span, offset: usize) -> bool {
    let end = span.end.max(span.start.saturating_add(1));
    offset >= span.start && offset < end
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
    let in_list = |list: &'a [BindingInfo]| {
        list.iter().find_map(|b| {
            (b.name == name && b.span.is_some_and(|s| span_contains(s, offset)))
                .then_some((node.label.as_str(), b))
        })
    };
    in_list(&node.observations).or_else(|| in_list(&node.bindings))
}

pub fn hover_for_analysis(
    report: &ArenaReport,
    source: &str,
    position: Position,
) -> Option<String> {
    let map = SourceMap::new(source);
    let name = map.word_at(position)?;
    let offset = map.position_to_offset(position)?;
    for root in &report.roots {
        if let Some((arena, info)) = find_span_hit(root, &name, offset) {
            let own = info.ownership.hover_label();
            return Some(format!("`{name}` in arena `{arena}`\nOwnership: {own}"));
        }
    }
    None
}

pub fn hover_for_source(source: &str, position: Position) -> Option<String> {
    hover_for_analysis(analyze_source(source).report()?, source, position)
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
    fn parse_error_points_at_the_offending_token() {
        let source = "fun main() {\n val x = (1 +\n)\n}";
        let diagnostics = diagnostics_for_source(source);
        let d = &diagnostics[0];
        assert!(d.message.starts_with("parse:"), "{d:?}");
        assert_eq!(d.range.start.line, 2, "{d:?}");
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
    fn ownership_and_type_diagnostics_are_reported_together() {
        let source = r#"
fun main(): i32 {
    var s: String = "a"
    val t = move s
    val u = s
    return 1 + "x"
}
"#;
        let diagnostics = diagnostics_for_source(source);
        assert!(
            diagnostics
                .iter()
                .any(|d| d.message.starts_with("ownership:")),
            "{diagnostics:?}"
        );
        assert!(
            diagnostics.iter().any(|d| d.message.starts_with("type:")),
            "{diagnostics:?}"
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
    fn hover_for_analysis_uses_cached_report() {
        let source = "fun add(x: Int): Int {\n    return x\n}\n";
        let analysis = analyze_source(source);
        let report = analysis.report().expect("source parses");
        let x_off = source.find('x').unwrap();
        let pos = byte_offset_to_position(source, x_off);
        let hover = hover_for_analysis(report, source, pos).expect("hover");
        assert!(hover.contains("Ownership"), "{hover}");
    }

    #[test]
    fn hover_resolves_for_loop_binder() {
        let source = "fun main() {\n    for (i in 0..3) {\n        val x = i\n    }\n}\n";
        let i_off = source.find("(i ").unwrap() + 1;
        let pos = byte_offset_to_position(source, i_off);
        let hover = hover_for_source(source, pos).expect("hover");
        assert!(hover.contains("Ownership"), "{hover}");
    }
}
