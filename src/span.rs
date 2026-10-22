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

    /// A fact keyed by an unnumbered node would silently collide with every other one, so this
    /// panics (in release too). `typeck::check` and `sema::analyze` need a numbered AST: use
    /// `parse`, or call [`crate::ast_number::number_nodes`] on a hand-built `Program`.
    pub fn numbered(self) -> AstId {
        assert!(self != AstId::DUMMY, "fact recorded for an unnumbered AST node; run ast_number::number_nodes");
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

#[cfg(test)]
mod tests {
    use super::AstId;

    #[test]
    #[should_panic(expected = "unnumbered AST node")]
    fn dummy_id_cannot_key_a_fact() {
        AstId::DUMMY.numbered();
    }

    #[test]
    fn numbered_id_passes_through() {
        assert_eq!(AstId(3).numbered(), AstId(3));
    }
}
