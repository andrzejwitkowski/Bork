//! Constructs MIR build or LLVM lowering does not support, each with a stable id (RFC 0006 3.8, 3.10).

use std::fmt;

use crate::span::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Closure,
    MatchRef,
    PatPresence,
    MoveEscapesArena,
    BorrowedStoreEscapes,
    FieldAccess,
    SafeFieldEnum,
    IndirectCall,
    UnitElem,
    UnitPayloadBind,
    ObjectEquality,
    UnitParam,
    UnitField,
    MainSignature,
}

impl Capability {
    pub const ALL: [Capability; 14] = [
        Capability::Closure,
        Capability::MatchRef,
        Capability::PatPresence,
        Capability::MoveEscapesArena,
        Capability::BorrowedStoreEscapes,
        Capability::FieldAccess,
        Capability::SafeFieldEnum,
        Capability::IndirectCall,
        Capability::UnitElem,
        Capability::UnitPayloadBind,
        Capability::ObjectEquality,
        Capability::UnitParam,
        Capability::UnitField,
        Capability::MainSignature,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Capability::Closure => "closure",
            Capability::MatchRef => "match-ref",
            Capability::PatPresence => "pat-presence",
            Capability::MoveEscapesArena => "move-escapes-arena",
            Capability::BorrowedStoreEscapes => "borrowed-store-escapes",
            Capability::FieldAccess => "field-access",
            Capability::SafeFieldEnum => "safe-field-enum",
            Capability::IndirectCall => "indirect-call",
            Capability::UnitElem => "unit-elem",
            Capability::UnitPayloadBind => "unit-payload-bind",
            Capability::ObjectEquality => "object-equality",
            Capability::UnitParam => "unit-param",
            Capability::UnitField => "unit-field",
            Capability::MainSignature => "main-signature",
        }
    }

    /// The user-facing diagnostic (what `bork check` and `bork build` report for the construct).
    pub fn message(self) -> &'static str {
        match self {
            Capability::Closure => "closures that own or move out their captures (`move` closures, `FnOnce`) are not supported by codegen yet",
            Capability::MatchRef => "`when` over a `Ref` is not supported by codegen yet; use `if val`",
            Capability::PatPresence => "`Some`/`None` patterns in `when` are not supported by codegen yet; use `if val`",
            Capability::MoveEscapesArena => "this `move` would leave the object in an arena that is freed before its new owner; build it where it ends up (the next statement, or the `return`)",
            Capability::BorrowedStoreEscapes => "storing a `String`, array or `Ref` into a borrowed array would allocate it in this function's arena, which is freed on return; build the value where the array lives (the caller) and pass it in",
            Capability::FieldAccess => "field access is not supported by codegen",
            Capability::SafeFieldEnum => "`?.` of an enum field is not supported by codegen yet",
            Capability::IndirectCall => "calls through a function value are not supported by codegen yet",
            Capability::UnitElem => "a `unit` element is not supported by codegen yet",
            Capability::UnitPayloadBind => "reading a bound `unit` payload is not supported by codegen yet",
            Capability::ObjectEquality => "comparing objects with `==` is not supported by codegen yet",
            Capability::UnitParam => "a `unit` parameter is not supported by codegen yet",
            Capability::UnitField => "a `unit` class field is not supported by codegen yet",
            Capability::MainSignature => "`main` must take no parameters and return `i32` or `unit` for codegen",
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
