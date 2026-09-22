//! Lexical environment and shadow stack for region analysis.

use super::report::SemaError;
use crate::ast::{BindingKind, Type};
use crate::span::Span;
use std::collections::{HashMap, HashSet};

#[derive(Clone)]
pub(super) struct EnvBinding {
    pub(super) arena_id: usize,
    pub(super) arena_label: String,
    pub(super) ty: Option<Type>,
    pub(super) kind: BindingKind,
    pub(super) moved: bool,
}

pub(super) fn moved_names(env: &HashMap<String, EnvBinding>) -> HashSet<String> {
    env.iter()
        .filter(|(_, b)| b.moved)
        .map(|(n, _)| n.clone())
        .collect()
}

pub(super) struct Analyzer {
    pub(super) next_id: usize,
    pub(super) errors: Vec<SemaError>,
    pub(super) env: HashMap<String, EnvBinding>,
}

impl Analyzer {
    pub(super) fn new() -> Self {
        Self {
            next_id: 0,
            errors: Vec::new(),
            env: HashMap::new(),
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
    ty: Option<Type>,
    kind: BindingKind,
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
