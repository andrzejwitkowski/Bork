//! Region entry: ordinary vs move arenas.

use super::env::{bind, restore_shadows, Analyzer, Shadow};
use super::free_vars::free_vars_in_block;
use super::peel_blocks;
use super::report::{ArenaNode, BindingInfo, Ownership};
use crate::ast::{BindingKind, Block, Type};
use crate::span::SpannedName;

pub(super) struct RegionFrame {
    shadows: Vec<Shadow>,
    moved_parents: Vec<String>,
    node: ArenaNode,
}

impl RegionFrame {
    fn new(id: usize, label: String, compacted: usize) -> Self {
        Self {
            shadows: Vec::new(),
            moved_parents: Vec::new(),
            node: ArenaNode {
                id,
                label,
                compacted_braces: compacted,
                bindings: Vec::new(),
                children: Vec::new(),
            },
        }
    }

    fn bind_param(&mut self, az: &mut Analyzer, name: &str, ty: Option<Type>) {
        let label = self.node.label.clone();
        let id = self.node.id;
        self.shadows
            .push(bind(az, name, id, &label, ty.clone(), BindingKind::Val));
        self.node.bindings.push(BindingInfo {
            name: name.to_string(),
            ownership: Ownership::Local,
            ty,
            span: None,
        });
    }

    fn bind_capture(&mut self, az: &mut Analyzer, cap: &SpannedName) {
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
                    ty: b.ty,
                    span: Some(cap.span),
                });
            }
        }
    }

    fn finish(self, az: &mut Analyzer) -> ArenaNode {
        restore_shadows(az, self.shadows);
        for cap in self.moved_parents {
            if let Some(b) = az.env.get_mut(&cap) {
                b.moved = true;
            }
        }
        self.node
    }
}

fn resolve_move_captures(
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

pub(super) fn open_ordinary(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    params: &[(String, Option<Type>)],
    walk: impl FnOnce(&mut Analyzer, &Block, &mut ArenaNode, &mut Vec<Shadow>),
) -> ArenaNode {
    let (body, compacted) = peel_blocks(body);
    let id = az.alloc_id();
    let mut frame = RegionFrame::new(id, label.to_string(), compacted);
    for (name, ty) in params {
        frame.bind_param(az, name, ty.clone());
    }
    walk(az, body, &mut frame.node, &mut frame.shadows);
    frame.finish(az)
}

pub(super) fn open_move(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    explicit_captures: Option<&[SpannedName]>,
    params: &[(String, Option<Type>)],
    walk: impl FnOnce(&mut Analyzer, &Block, &mut ArenaNode, &mut Vec<Shadow>),
) -> ArenaNode {
    let (peeled, compacted) = peel_blocks(body);
    let id = az.alloc_id();
    let param_names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
    let captures = resolve_move_captures(az, peeled, explicit_captures, &param_names);
    let mut frame = RegionFrame::new(id, format!("{label} (move)"), compacted);
    for (name, ty) in params {
        frame.bind_param(az, name, ty.clone());
    }
    for cap in &captures {
        frame.bind_capture(az, cap);
    }
    walk(az, peeled, &mut frame.node, &mut frame.shadows);
    frame.finish(az)
}
