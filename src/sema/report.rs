//! Arena report types shared by dump, LSP, and analysis.

use crate::ast::Type;
use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    Local,
    Copy,
    Shared { from: String },
    Moved { from: String },
}

impl Ownership {
    fn label(&self) -> String {
        match self {
            Ownership::Local => "Local".into(),
            Ownership::Copy => "Copy".into(),
            Ownership::Shared { from } => format!("Shared ← {from}"),
            Ownership::Moved { from } => format!("Moved ← {from}"),
        }
    }

    pub fn dump_tag(&self) -> String {
        format!("[{}]", self.label())
    }

    pub fn hover_label(&self) -> String {
        self.label()
    }

    pub fn is_dump_line(&self) -> bool {
        !matches!(self, Ownership::Local)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BindingInfo {
    pub name: String,
    pub ownership: Ownership,
    pub ty: Option<Type>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArenaNode {
    pub id: usize,
    pub label: String,
    pub compacted_braces: usize,
    /// Declarations and move captures (always shown in dump).
    pub bindings: Vec<BindingInfo>,
    /// Use sites and cross-arena Copy/Shared (dump shows non-Local only).
    pub observations: Vec<BindingInfo>,
    pub children: Vec<ArenaNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArenaReport {
    pub roots: Vec<ArenaNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaError {
    pub message: String,
    pub name: Option<String>,
    pub span: Option<Span>,
}
