//! Cross-arena use classification for ownership checking.

use super::env::EnvBinding;
use super::report::Ownership;
use crate::ast::BindingKind;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum UseOutcome {
    Ignore,
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
        return UseOutcome::Ignore;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BindingKind, Type};
    use super::super::env::Ty;

    fn binding(arena_id: usize, ty: Ty, kind: BindingKind, moved: bool) -> EnvBinding {
        EnvBinding {
            arena_id,
            arena_label: "parent".into(),
            ty,
            kind,
            moved,
        }
    }

    #[test]
    fn same_arena_is_ignore() {
        let b = binding(1, Ty::Known(Type::from_ident("Int", false)), BindingKind::Val, false);
        assert_eq!(classify_use(&b, 1, "here", "x"), UseOutcome::Ignore);
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
}
