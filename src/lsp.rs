//! Map Bork parse errors to LSP diagnostics.

use crate::{parse, Error};
use lalrpop_util::ParseError;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// UTF-8 byte offset → LSP [`Position`] (UTF-16 code units).
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

/// Map a LALRPOP parse error to one LSP diagnostic.
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

/// Parse `source`; empty vec when it succeeds.
pub fn diagnostics_for_source(source: &str) -> Vec<Diagnostic> {
    match parse(source) {
        Ok(_) => Vec::new(),
        Err(error) => vec![parse_error_to_diagnostic(source, &error)],
    }
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
        // 'é' is one UTF-16 unit; '𝄞' (U+1D11E) is two.
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
}
