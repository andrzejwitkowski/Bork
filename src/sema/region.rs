//! Region frame and move-capture resolution.

use super::env::{bind, bind_with, BindingOrigin, restore_shadows, Analyzer, Shadow, Ty};
use super::free_vars::free_vars_in_block;
use super::policy::{move_source, report_move_source_err};
use super::report::{ArenaNode, BindingInfo, Ownership};
use crate::ast::{BindingKind, Block};
use crate::span::{Span, SpannedName};

pub(super) struct RegionParam {
    pub(super) name: String,
    pub(super) ty: Ty,
    pub(super) kind: BindingKind,
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
            param.kind,
        ));
        self.node.bindings.push(BindingInfo {
            name: param.name.clone(),
            ownership: Ownership::Local,
            ty: param.ty.as_option(),
            span: param.span,
        });
    }

    pub(super) fn bind_capture(&mut self, az: &mut Analyzer, cap: &SpannedName) {
        let binding = match move_source(az, &cap.name) {
            Ok(b) => b,
            Err(e) => {
                report_move_source_err(az, &cap.name, e, Some(cap.span));
                return;
            }
        };
        self.moved_parents.push(cap.name.clone());
        let label = self.node.label.clone();
        let id = self.node.id;
        self.shadows.push(bind_with(
            az,
            &cap.name,
            id,
            &label,
            binding.ty.clone(),
            binding.kind,
            BindingOrigin::Captured,
        ));
        self.node.bindings.push(BindingInfo {
            name: cap.name.clone(),
            ownership: Ownership::Moved {
                from: binding.arena_label,
            },
            ty: binding.ty.as_option(),
            span: Some(cap.span),
        });
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
            .filter(|n| {
                az.env
                    .get(&n.name)
                    .is_some_and(|b| !b.moved && !b.ty.is_copy())
            })
            .collect(),
    }
}
