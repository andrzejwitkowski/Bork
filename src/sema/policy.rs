//! Cross-arena use classification and move / transfer policy.

use super::env::{Analyzer, EnvBinding};
use super::report::Ownership;
use crate::ast::BindingKind;
use crate::span::Span;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum UseOutcome {
    Observe(Ownership),
    Error { message: String },
}

pub(super) fn classify_use(
    binding: &EnvBinding,
    current_arena: usize,
    current_label: &str,
    name: &str,
) -> UseOutcome {
    if binding.moved {
        return UseOutcome::Error {
            message: format!(
                "use of `{name}` after move from {}",
                binding.arena_label
            ),
        };
    }
    if binding.arena_id == current_arena {
        return UseOutcome::Observe(Ownership::Local);
    }
    let is_copy = binding.ty.is_copy();
    if is_copy {
        return UseOutcome::Observe(Ownership::Copy);
    }
    if matches!(binding.kind, BindingKind::Val) {
        return UseOutcome::Observe(Ownership::Shared {
            from: binding.arena_label.clone(),
        });
    }
    UseOutcome::Error {
        message: format!("`{name}` is not Copy; move it into `{current_label}` with `move`"),
    }
}

#[derive(Clone)]
pub(super) enum TransferSink {
    Binding {
        dest: BindingKind,
        arena_id: usize,
        arena_label: String,
    },
    CallArg { formal: BindingKind },
}

pub(super) fn bare_ident_move_message(
    binding: &EnvBinding,
    name: &str,
    sink: &TransferSink,
) -> Option<String> {
    if binding.ty.is_copy() || binding.moved {
        return None;
    }
    let counterpart_var = match sink {
        TransferSink::Binding { dest, .. } => matches!(dest, BindingKind::Var),
        TransferSink::CallArg { formal } => matches!(formal, BindingKind::Var),
    };
    let src_var = matches!(binding.kind, BindingKind::Var);
    if !(src_var || counterpart_var) {
        return None;
    }
    Some(match sink {
        TransferSink::Binding {
            arena_id,
            arena_label,
            ..
        } if src_var && binding.arena_id != *arena_id => {
            format!("`{name}` is not Copy; move it into `{arena_label}` with `move`")
        }
        TransferSink::CallArg { .. } => format!("use `move {name}` to pass ownership"),
        _ => format!("use `move {name}` to transfer ownership"),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum MoveSourceErr {
    Unknown,
    AlreadyMoved { from: String },
    InLoop,
}

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
    span: Option<Span>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BindingKind, Type};
    use super::super::env::{BindingOrigin, Ty};

    fn binding(arena_id: usize, ty: Ty, kind: BindingKind, moved: bool) -> EnvBinding {
        EnvBinding {
            arena_id,
            arena_label: "parent".into(),
            ty,
            kind,
            moved,
            origin: BindingOrigin::Declared,
        }
    }

    #[test]
    fn same_arena_is_observe_local() {
        let b = binding(1, Ty::Known(Type::from_ident("Int", false)), BindingKind::Val, false);
        assert_eq!(
            classify_use(&b, 1, "here", "x"),
            UseOutcome::Observe(Ownership::Local)
        );
    }

    #[test]
    fn moved_is_error() {
        let b = binding(
            0,
            Ty::Known(Type::from_ident("String", false)),
            BindingKind::Val,
            true,
        );
        assert!(matches!(
            classify_use(&b, 1, "here", "x"),
            UseOutcome::Error { .. }
        ));
    }

    #[test]
    fn copy_type_is_observe_copy() {
        let b = binding(0, Ty::Known(Type::from_ident("Int", false)), BindingKind::Val, false);
        assert_eq!(
            classify_use(&b, 1, "here", "n"),
            UseOutcome::Observe(Ownership::Copy)
        );
    }

    #[test]
    fn val_non_copy_is_shared() {
        let b = binding(
            0,
            Ty::Known(Type::Named {
                name: "String".into(),
                nullable: false,
            }),
            BindingKind::Val,
            false,
        );
        assert!(matches!(
            classify_use(&b, 1, "here", "s"),
            UseOutcome::Observe(Ownership::Shared { .. })
        ));
    }

    #[test]
    fn var_non_copy_is_error() {
        let b = binding(
            0,
            Ty::Known(Type::Named {
                name: "String".into(),
                nullable: false,
            }),
            BindingKind::Var,
            false,
        );
        assert!(matches!(
            classify_use(&b, 1, "Block", "s"),
            UseOutcome::Error { .. }
        ));
    }

    #[test]
    fn bare_var_into_val_binding_needs_move() {
        let b = binding(
            0,
            Ty::Known(Type::Named {
                name: "String".into(),
                nullable: false,
            }),
            BindingKind::Var,
            false,
        );
        let msg = bare_ident_move_message(
            &b,
            "s",
            &TransferSink::Binding {
                dest: BindingKind::Val,
                arena_id: 0,
                arena_label: "fun main".into(),
            },
        );
        assert!(msg.unwrap().contains("move"));
    }

    #[test]
    fn bare_val_into_val_binding_ok() {
        let b = binding(
            0,
            Ty::Known(Type::Named {
                name: "String".into(),
                nullable: false,
            }),
            BindingKind::Val,
            false,
        );
        assert!(bare_ident_move_message(
            &b,
            "s",
            &TransferSink::Binding {
                dest: BindingKind::Val,
                arena_id: 0,
                arena_label: "fun main".into(),
            },
        )
        .is_none());
    }
}
