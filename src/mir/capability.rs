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

    /// The user-facing diagnostic (what `bork check` and `bork build` report for the construct).
    pub fn message(self) -> &'static str {
        match self {
            Capability::Closure => "trailing closures are not supported by codegen",
            Capability::MatchRef => "`when` over a `Ref` is not supported by codegen yet; use `if val`",
            Capability::PatPresence => "`Some`/`None` patterns in `when` are not supported by codegen yet; use `if val`",
            Capability::MoveAggregate => "moving values of this type is not supported by codegen yet",
            Capability::FieldAccess => "field access is not supported by codegen",
            Capability::SafeFieldEnum => "`?.` of an enum field is not supported by codegen yet",
            Capability::IndirectCall => "calls through a function value are not supported by codegen yet",
            Capability::UnitElem => "a `unit` element is not supported by codegen yet",
            Capability::UnitPayloadBind => "reading a bound `unit` payload is not supported by codegen yet",
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
    /// Overrides [`Capability::message`] when one capability covers several diagnostics.
    pub message: Option<&'static str>,
}

impl Unsupported {
    pub fn message(&self) -> &'static str {
        self.message.unwrap_or_else(|| self.capability.message())
    }
}
