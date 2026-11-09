//! M3 shadow mode (RFC 0006 step 5, #216): derives the memory plan, region events and push flags from MIR and
//! compares them with what `memory.rs`, `codegen/regions.rs` and `region_walk` decide. Test and
//! `BORK_MIR_SHADOW=1` use only; deleted together with the old passes (M4b, M6).

use std::collections::HashMap;

use super::body::{AllocKind, Body, StmtKind};
use crate::hir::NodeId;
use crate::memory::{ArenaClass, LifetimeDomain};
use crate::mono::Mono;
use crate::sema::ArenaNode;

/// `(kind, arena id)`; kind 0 = push, 1 = reset, 2 = pop.
pub type Event = (u8, usize);

/// Class and "program-lived" flag of every object / enum allocation, keyed by node.
pub fn derive_plan(bodies: &[Body]) -> HashMap<NodeId, (ArenaClass, bool)> {
    let mut out = HashMap::new();
    for body in bodies {
        // An allocation is dynamic iff its own node created a dynamic arena; a nested payload merely lives in one.
        let dynamic: HashMap<NodeId, bool> = body
            .blocks
            .iter()
            .flat_map(|b| &b.stmts)
            .filter_map(|s| match (&s.kind, s.node) {
                (StmtKind::ArenaCreateDynamic { scoped, .. }, Some(n)) => Some((n, !scoped)),
                _ => None,
            })
            .collect();
        for s in body.blocks.iter().flat_map(|b| &b.stmts) {
            let (StmtKind::Alloc { kind: AllocKind::Object { .. } | AllocKind::EnumSlot { .. }, .. }, Some(node)) = (&s.kind, s.node) else { continue };
            let derived = match dynamic.get(&node) {
                Some(&program) => (ArenaClass::Dynamic, program),
                None => (ArenaClass::Lexical, false),
            };
            out.insert(node, derived);
        }
    }
    out
}

/// Sorted push / reset / pop events of one body (one pop per pushed region, see the static schedule).
pub fn derive_events(body: &Body) -> Vec<Event> {
    let mut events = Vec::new();
    for s in body.blocks.iter().flat_map(|b| &b.stmts) {
        match s.kind {
            StmtKind::RegionEnter(r) => {
                let arena = body.regions[r.0 as usize].arena;
                events.extend([(0, arena), (2, arena)]);
            }
            StmtKind::RegionReset { region, .. } => events.push((1, body.regions[region.0 as usize].arena)),
            _ => {}
        }
    }
    events.sort_unstable();
    events
}

fn flagged(node: &ArenaNode, out: &mut Vec<usize>) {
    if node.codegen_push {
        out.push(node.id);
    }
    node.children.iter().for_each(|c| flagged(c, out));
}

/// Compares everything the MIR can say with the old passes; `Err` describes the first difference.
pub fn compare(mono: &Mono, bodies: &[Body]) -> Result<(), String> {
    let plan = derive_plan(bodies);
    for a in &mono.memory_plan.allocations {
        let Some(&(class, program)) = plan.get(&a.node) else { return Err(format!("plan allocation {:?} has no MIR Alloc", a.node)) };
        let want = (a.class, a.lifetime_domain == LifetimeDomain::Program);
        if (class, program) != want {
            return Err(format!("allocation {:?} `{}`: plan {want:?}, MIR {:?}", a.node, a.class_name, (class, program)));
        }
    }
    if bodies.len() != mono.report.roots.len() {
        return Err(format!("{} MIR bodies for {} report roots", bodies.len(), mono.report.roots.len()));
    }
    for (body, root) in bodies.iter().zip(&mono.report.roots) {
        let mut want = Vec::new();
        flagged(root, &mut want);
        let entered: Vec<usize> = body.blocks.iter().flat_map(|b| &b.stmts).filter_map(|s| match s.kind {
            StmtKind::RegionEnter(r) => Some(body.regions[r.0 as usize].arena),
            _ => None,
        }).collect();
        if let Some(missing) = want.iter().find(|id| !entered.contains(id)) {
            return Err(format!("`{}`: report region {missing} is flagged codegen_push but MIR never enters it", body.name));
        }
        if let Some(extra) = entered.iter().find(|id| !want.contains(id)) {
            return Err(format!("`{}`: MIR enters region {extra} that is not flagged codegen_push", body.name));
        }
    }
    Ok(())
}

/// Splits the global static schedule into per-function sorted event lists (a function ends when depth is 0).
#[cfg(feature = "codegen")]
pub fn schedule_events(events: &[crate::codegen::RegionEvent]) -> Vec<Vec<Event>> {
    use crate::codegen::RegionEvent::*;
    let (mut out, mut cur, mut depth) = (Vec::new(), Vec::new(), 0usize);
    for e in events {
        match *e {
            Push { arena } => {
                depth += 1;
                cur.push((0, arena));
            }
            Reset { arena } => cur.push((1, arena)),
            Pop { arena } => {
                depth -= 1;
                cur.push((2, arena));
                if depth == 0 {
                    cur.sort_unstable();
                    out.push(std::mem::take(&mut cur));
                }
            }
        }
    }
    out
}

/// Full shadow check including the independent region schedule.
#[cfg(feature = "codegen")]
pub fn compare_all(mono: &Mono, tcx: &crate::ty::TyCtx, bodies: &[Body]) -> Result<(), String> {
    compare(mono, bodies)?;
    let events = crate::codegen::schedule(&mono.hir, tcx, &mono.report).map_err(|e| format!("schedule: {e:?}"))?;
    let want = schedule_events(&events);
    if want.len() != bodies.len() {
        return Err(format!("schedule has {} functions, MIR {}", want.len(), bodies.len()));
    }
    for (body, want) in bodies.iter().zip(&want) {
        let got = derive_events(body);
        if &got != want {
            return Err(format!("`{}`: schedule {want:?}, MIR {got:?}", body.name));
        }
    }
    Ok(())
}
