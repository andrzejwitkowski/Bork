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

pub fn from_parse(err: &crate::Error) -> Diagnostic {
    Diagnostic {
        phase: Phase::Parse,
        severity: Severity::Error,
        message: err.to_string(),
        span: None,
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
}
