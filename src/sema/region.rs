//! Region frame and move-capture resolution.

use super::env::{bind, restore_shadows, Analyzer, Shadow, Ty};
use super::free_vars::free_vars_in_block;
use super::report::{ArenaNode, BindingInfo, Ownership};
use crate::ast::{BindingKind, Block};
use crate::span::{Span, SpannedName};

pub(super) struct RegionParam {
    pub(super) name: String,
    pub(super) ty: Ty,
    pub(super) span: Option<Span>,
}

pub(super) struct RegionFrame {
    pub(super) shadows: Vec<Shadow>,
    moved_parents: Vec<String>,
    pub(super) node: ArenaNode,
}

impl RegionFrame {
    pub(super) fn new(id: usize, label: String, compacted: usize) -> Self {
        Self {
            shadows: Vec::new(),
            moved_parents: Vec::new(),
            node: ArenaNode {
                id,
                label,
                compacted_braces: compacted,
                bindings: Vec::new(),
                observations: Vec::new(),
                children: Vec::new(),
            },
        }
    }

    pub(super) fn bind_param(&mut self, az: &mut Analyzer, param: &RegionParam) {
        let label = self.node.label.clone();
        let id = self.node.id;
        self.shadows.push(bind(
            az,
            &param.name,
            id,
            &label,
            param.ty.clone(),
            BindingKind::Val,
        ));
        self.node.bindings.push(BindingInfo {
            name: param.name.clone(),
            ownership: Ownership::Local,
            ty: param.ty.as_option(),
            span: param.span,
        });
    }

    pub(super) fn bind_capture(&mut self, az: &mut Analyzer, cap: &SpannedName) {
        match az.env.get(&cap.name).cloned() {
            None => az.error(
                format!("cannot move unknown name `{}`", cap.name),
                Some(cap.name.clone()),
                Some(cap.span),
            ),
            Some(b) if b.moved => az.error(
                format!(
                    "cannot move `{}`: already moved from {}",
                    cap.name, b.arena_label
                ),
                Some(cap.name.clone()),
                Some(cap.span),
            ),
            Some(b) => {
                self.moved_parents.push(cap.name.clone());
                let label = self.node.label.clone();
                let id = self.node.id;
                self.shadows
                    .push(bind(az, &cap.name, id, &label, b.ty.clone(), BindingKind::Val));
                self.node.bindings.push(BindingInfo {
                    name: cap.name.clone(),
                    ownership: Ownership::Moved {
                        from: b.arena_label,
                    },
                    ty: b.ty.as_option(),
                    span: Some(cap.span),
                });
            }
        }
    }

    pub(super) fn finish(self, az: &mut Analyzer) -> ArenaNode {
        restore_shadows(az, self.shadows);
        for cap in self.moved_parents {
            if let Some(b) = az.env.get_mut(&cap) {
                b.moved = true;
            }
        }
        self.node
    }
}

pub(super) fn resolve_move_captures(
    az: &Analyzer,
    body: &Block,
    explicit_captures: Option<&[SpannedName]>,
    param_names: &[String],
) -> Vec<SpannedName> {
    match explicit_captures {
        Some(caps) => caps.to_vec(),
        None => free_vars_in_block(body)
            .into_iter()
            .filter(|n| !param_names.contains(&n.name))
            .filter(|n| az.env.get(&n.name).is_some_and(|b| !b.moved))
            .collect(),
    }
}
