//! Lexical environment and shadow stack for region analysis.

use super::report::SemaError;
use crate::ast::{BindingKind, Type};
use crate::span::Span;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindingOrigin {
    /// `val`/`var` decl or function/closure/for param.
    Declared,
    /// Brought in by a regional `move` capture list.
    Captured,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Ty {
    Known(Type),
    Unknown,
}

impl Ty {
    pub(super) fn from_option(ty: Option<Type>) -> Self {
        match ty {
            Some(t) => Ty::Known(t),
            None => Ty::Unknown,
        }
    }

    pub(super) fn as_option(&self) -> Option<Type> {
        match self {
            Ty::Known(t) => Some(t.clone()),
            Ty::Unknown => None,
        }
    }

    pub(super) fn is_copy(&self) -> bool {
        matches!(self, Ty::Known(t) if t.is_copy())
    }
}

#[derive(Clone)]
pub(super) struct EnvBinding {
    pub(super) arena_id: usize,
    pub(super) arena_label: String,
    pub(super) ty: Ty,
    pub(super) kind: BindingKind,
    pub(super) moved: bool,
    pub(super) origin: BindingOrigin,
}

pub(super) fn moved_names(env: &HashMap<String, EnvBinding>) -> HashSet<String> {
    env.iter()
        .filter(|(_, b)| b.moved)
        .map(|(n, _)| n.clone())
        .collect()
}

pub(super) fn restore_moved_flags(env: &mut HashMap<String, EnvBinding>, moved: &HashSet<String>) {
    for (name, binding) in env.iter_mut() {
        binding.moved = moved.contains(name);
    }
}

pub(super) fn apply_moved_merge(
    env: &mut HashMap<String, EnvBinding>,
    before_moved: &HashSet<String>,
    then_moved: &HashSet<String>,
    else_moved: Option<&HashSet<String>>,
) {
    for (name, binding) in env.iter_mut() {
        let then = then_moved.contains(name);
        binding.moved = match else_moved {
            Some(else_set) => before_moved.contains(name) || (then && else_set.contains(name)),
            None => before_moved.contains(name),
        };
    }
}

pub(super) struct Analyzer {
    pub(super) next_id: usize,
    pub(super) errors: Vec<SemaError>,
    pub(super) env: HashMap<String, EnvBinding>,
    pub(super) fun_sigs: HashMap<String, Vec<BindingKind>>,
    /// Names that existed when each enclosing `for` was entered; moves of those
    /// names inside the loop are rejected (would be spent on later iterations).
    pub(super) loop_move_ban: Vec<HashSet<String>>,
}

impl Analyzer {
    pub(super) fn new() -> Self {
        Self {
            next_id: 0,
            errors: Vec::new(),
            env: HashMap::new(),
            fun_sigs: HashMap::new(),
            loop_move_ban: Vec::new(),
        }
    }

    pub(super) fn alloc_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(super) fn error(
        &mut self,
        message: impl Into<String>,
        name: Option<String>,
        span: Option<Span>,
    ) {
        self.errors.push(SemaError {
            message: message.into(),
            name,
            span,
        });
    }

    pub(super) fn move_banned_in_loop(&self, name: &str) -> bool {
        self.loop_move_ban.iter().any(|s| s.contains(name))
    }
}

pub(super) struct Shadow(pub(super) String, pub(super) Option<EnvBinding>);

pub(super) fn shadow_insert(az: &mut Analyzer, name: String, binding: EnvBinding) -> Shadow {
    let prev = az.env.insert(name.clone(), binding);
    Shadow(name, prev)
}

pub(super) fn bind(
    az: &mut Analyzer,
    name: &str,
    arena_id: usize,
    arena_label: &str,
    ty: Ty,
    kind: BindingKind,
) -> Shadow {
    bind_with(
        az,
        name,
        arena_id,
        arena_label,
        ty,
        kind,
        BindingOrigin::Declared,
    )
}

pub(super) fn bind_with(
    az: &mut Analyzer,
    name: &str,
    arena_id: usize,
    arena_label: &str,
    ty: Ty,
    kind: BindingKind,
    origin: BindingOrigin,
) -> Shadow {
    shadow_insert(
        az,
        name.to_string(),
        EnvBinding {
            arena_id,
            arena_label: arena_label.to_string(),
            ty,
            kind,
            moved: false,
            origin,
        },
    )
}

pub(super) fn shadow_restore(az: &mut Analyzer, Shadow(name, prev): Shadow) {
    match prev {
        Some(prev) => {
            az.env.insert(name, prev);
        }
        None => {
            az.env.remove(&name);
        }
    }
}

pub(super) fn restore_shadows(az: &mut Analyzer, shadows: Vec<Shadow>) {
    for s in shadows.into_iter().rev() {
        shadow_restore(az, s);
    }
}
