//! Byte offsets into the source string passed to the parser.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }
}

/// Identity of an AST node that typeck records a fact about. Assigned by
/// [`crate::ast_number::number_nodes`] right after parsing; the grammar leaves `DUMMY`.
/// Not [`crate::hir::NodeId`]: that numbers HIR expressions after hoisting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AstId(pub u32);

impl AstId {
    pub const DUMMY: AstId = AstId(u32::MAX);

    /// A fact keyed by an unnumbered node would collide with every other one.
    pub fn numbered(self) -> AstId {
        debug_assert!(self != AstId::DUMMY, "fact recorded for an unnumbered AST node");
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpannedName {
    pub name: String,
    pub span: Span,
    pub id: AstId,
}

impl SpannedName {
    pub fn new(name: impl Into<String>, span: Span) -> Self {
        Self {
            name: name.into(),
            span,
            id: AstId::DUMMY,
        }
    }
}
