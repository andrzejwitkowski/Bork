//! Shared move-source validation for expression and regional moves.

use super::env::{Analyzer, EnvBinding};

#[derive(Debug, PartialEq, Eq)]
pub(super) enum MoveSourceErr {
    Unknown,
    AlreadyMoved { from: String },
    InLoop,
}

/// Snapshot a binding that may be moved from the current env. Does not mark `moved`.
pub(super) fn move_source(az: &Analyzer, name: &str) -> Result<EnvBinding, MoveSourceErr> {
    let Some(binding) = az.env.get(name).cloned() else {
        return Err(MoveSourceErr::Unknown);
    };
    if binding.moved {
        return Err(MoveSourceErr::AlreadyMoved {
            from: binding.arena_label,
        });
    }
    if az.move_banned_in_loop(name) {
        return Err(MoveSourceErr::InLoop);
    }
    Ok(binding)
}

pub(super) fn report_move_source_err(
    az: &mut Analyzer,
    name: &str,
    err: MoveSourceErr,
    span: Option<crate::span::Span>,
) {
    match err {
        MoveSourceErr::Unknown => az.error(
            format!("cannot move unknown name `{name}`"),
            Some(name.into()),
            span,
        ),
        MoveSourceErr::AlreadyMoved { from } => az.error(
            format!("cannot move `{name}`: already moved from {from}"),
            Some(name.into()),
            span,
        ),
        MoveSourceErr::InLoop => az.error_move_in_loop(name, span),
    }
}
