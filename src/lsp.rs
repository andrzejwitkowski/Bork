//! Helpers for mapping Bork parse errors into LSP diagnostics.

use crate::{parse, Error};
use lalrpop_util::ParseError;
use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, Position, Range};

/// Convert a UTF-8 byte offset into an LSP [`Position`] (UTF-16 code units).
pub fn byte_offset_to_position(source: &str, byte_offset: usize) -> Position {
    let offset = byte_offset.min(source.len());
    let mut line = 0u32;
    let mut character = 0u32;
    let mut scanned = 0usize;

    for ch in source[..offset].chars() {
        let byte_len = ch.len_utf8();
        if scanned + byte_len > offset {
            break;
        }
        scanned += byte_len;
        if ch == '\n' {
            line += 1;
            character = 0;
        } else {
            character += ch.len_utf16() as u32;
        }
    }

    Position { line, character }
}

fn clamp_range(source: &str, start: usize, end: usize) -> Range {
    let start = start.min(source.len());
    let end = end.min(source.len()).max(start);
    let mut range = Range {
        start: byte_offset_to_position(source, start),
        end: byte_offset_to_position(source, end),
    };
    // Empty ranges are hard to see; widen zero-width to one UTF-16 unit when possible.
    if range.start == range.end {
        if end < source.len() {
            range.end = byte_offset_to_position(source, (end + 1).min(source.len()));
            if range.start == range.end {
                range.end.character = range.end.character.saturating_add(1);
            }
        } else if range.end.character > 0 {
            range.start.character = range.end.character.saturating_sub(1);
        } else if range.end.line > 0 {
            range.start.line = range.end.line.saturating_sub(1);
        } else {
            range.end.character = 1;
        }
    }
    range
}

/// Map a LALRPOP parse error to a single LSP diagnostic for `source`.
pub fn parse_error_to_diagnostic(source: &str, error: &Error) -> Diagnostic {
    let (range, message) = match error {
        ParseError::InvalidToken { location } => (
            clamp_range(source, *location, *location),
            "invalid token".to_string(),
        ),
        ParseError::UnrecognizedEof { location, expected } => {
            let mut message = "unexpected end of file".to_string();
            if !expected.is_empty() {
                message.push_str("; expected ");
                message.push_str(&expected.join(", "));
            }
            (clamp_range(source, *location, *location), message)
        }
        ParseError::UnrecognizedToken {
            token: (start, token, end),
            expected,
        } => {
            let mut message = format!("unexpected token `{token}`");
            if !expected.is_empty() {
                message.push_str("; expected ");
                message.push_str(&expected.join(", "));
            }
            (clamp_range(source, *start, *end), message)
        }
        ParseError::ExtraToken {
            token: (start, token, end),
        } => (
            clamp_range(source, *start, *end),
            format!("unexpected extra token `{token}`"),
        ),
        ParseError::User { error } => (
            clamp_range(source, source.len(), source.len()),
            (*error).to_string(),
        ),
    };

    Diagnostic {
        range,
        severity: Some(DiagnosticSeverity::ERROR),
        source: Some("bork".to_string()),
        message,
        ..Diagnostic::default()
    }
}

/// Parse `source` and return diagnostics (empty when parse succeeds).
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
        let offset = source.find('1').expect("digit present");
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
        let offset = source.find('𝄞').expect("clef present");
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
        let diagnostic = &diagnostics[0];
        assert!(!diagnostic.message.is_empty());
        assert!(diagnostic.range.start.line <= diagnostic.range.end.line);
        assert!(diagnostic.range.end.line < source.lines().count() as u32 + 1);
    }

    #[test]
    fn valid_sample_has_no_diagnostics() {
        assert!(diagnostics_for_source(crate::MVP_SAMPLE).is_empty());
    }
}
