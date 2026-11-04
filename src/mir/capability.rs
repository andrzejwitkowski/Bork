//! Constructs MIR build or LLVM lowering does not support, each with a stable id (RFC 0006 3.8, 3.10).

use std::fmt;

use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Closure,
    MatchRef,
    PatPresence,
    MoveAggregate,
    FieldAccess,
    SafeFieldEnum,
    IndirectCall,
    UnitElem,
    UnitPayloadBind,
}

impl Capability {
    pub const ALL: [Capability; 9] = [
        Capability::Closure,
        Capability::MatchRef,
        Capability::PatPresence,
        Capability::MoveAggregate,
        Capability::FieldAccess,
        Capability::SafeFieldEnum,
        Capability::IndirectCall,
        Capability::UnitElem,
        Capability::UnitPayloadBind,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Capability::Closure => "closure",
            Capability::MatchRef => "match-ref",
            Capability::PatPresence => "pat-presence",
            Capability::MoveAggregate => "move-aggregate",
            Capability::FieldAccess => "field-access",
            Capability::SafeFieldEnum => "safe-field-enum",
            Capability::IndirectCall => "indirect-call",
            Capability::UnitElem => "unit-elem",
            Capability::UnitPayloadBind => "unit-payload-bind",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// Returned by MIR build for a shape it cannot lower.
#[derive(Debug, Clone, PartialEq)]
pub struct Unsupported {
    pub capability: Capability,
    pub span: Span,
}
