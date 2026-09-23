use lalrpop_util::ParseError;

use crate::sema::SemaError;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Parse,
    Ownership,
    Type,
    Codegen,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub phase: Phase,
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
}

pub fn from_sema(err: &SemaError) -> Diagnostic {
    Diagnostic {
        phase: Phase::Ownership,
        severity: Severity::Error,
        message: err.message.clone(),
        span: err.span,
    }
}

fn expected_suffix(expected: &[String]) -> String {
    if expected.is_empty() {
        String::new()
    } else {
        format!("; expected {}", expected.join(", "))
    }
}

pub fn from_parse(source: &str, err: &crate::Error) -> Diagnostic {
    let eof = Span::new(source.len(), source.len());
    let (span, message) = match err {
        ParseError::InvalidToken { location } => {
            (Span::new(*location, *location), "invalid token".to_string())
        }
        ParseError::UnrecognizedEof { location, expected } => (
            Span::new(*location, *location),
            format!("unexpected end of file{}", expected_suffix(expected)),
        ),
        ParseError::UnrecognizedToken {
            token: (start, token, end),
            expected,
        } => (
            Span::new(*start, *end),
            format!("unexpected token `{token}`{}", expected_suffix(expected)),
        ),
        ParseError::ExtraToken {
            token: (start, token, end),
        } => (
            Span::new(*start, *end),
            format!("unexpected extra token `{token}`"),
        ),
        ParseError::User { error } => (eof, (*error).to_string()),
    };

    Diagnostic {
        phase: Phase::Parse,
        severity: Severity::Error,
        message,
        span: Some(span),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_sema_error() {
        let err = SemaError {
            message: "use after move: x".into(),
            name: Some("x".into()),
            span: Some(Span::new(1, 2)),
        };
        let d = from_sema(&err);
        assert_eq!(d.phase, Phase::Ownership);
        assert_eq!(d.message, "use after move: x");
        assert_eq!(d.span, Some(Span::new(1, 2)));
    }

    #[test]
    fn parse_error_carries_a_span() {
        let source = "fun main() {\n val x = (1 +\n)\n}";
        let err = crate::parse(source).expect_err("snippet does not parse");
        let d = from_parse(source, &err);
        assert_eq!(d.phase, Phase::Parse);
        let span = d.span.expect("parse errors carry a span");
        assert!(span.start <= span.end && span.end <= source.len());
    }
}
