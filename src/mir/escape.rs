//! Shadow of the object-lifetime part of `src/escape.rs` on MIR (RFC 0006 M4a, #217): a handle to an object
//! allocated in an inner region, stored into an object that outlives that region. Only the cases MIR can
//! see are covered; the rest of `escape.rs` stays (see #243). Not wired into `check`; tests compare it.

use std::collections::{HashMap, HashSet};

use super::body::*;
use crate::region_walk::RegionSite;
use crate::span::Span;

/// `(span of the storing statement, message)` in the wording of `escape.rs`.
pub fn check(body: &Body) -> Vec<(Span, String)> {
    let mut found = Vec::new();
    // shortcut: one visit per block and one depth per local (no join of paths), enough for the corpus; redo as dataflow when #243 moves the remaining rules.
    // depth of the arena a local's object was allocated in (0 = outlives every region) and whether that is a loop.
    let mut alloc: HashMap<Local, (usize, bool)> = HashMap::new();
    let mut seen = HashSet::new();
    let mut work = vec![(BlockId(0), Vec::<RegionId>::new())];
    while let Some((id, mut stack)) = work.pop() {
        if !seen.insert(id) {
            continue;
        }
        let block = &body.blocks[id.0 as usize];
        for s in &block.stmts {
            match &s.kind {
                StmtKind::RegionEnter(r) => stack.push(*r),
                StmtKind::RegionExit(r) => stack.retain(|x| x != r),
                StmtKind::Alloc { dst, kind: AllocKind::Object { .. } | AllocKind::EnumSlot { .. }, arena } if dst.is_bare() => {
                    let depth = match arena {
                        ArenaRef::Current => stack.len(),
                        ArenaRef::Region(r) => stack.iter().position(|x| x == r).map_or(0, |i| i + 1),
                        _ => 0,
                    };
                    let in_loop = stack.iter().any(|r| matches!(body.regions[r.0 as usize].site, RegionSite::For | RegionSite::While));
                    alloc.insert(dst.local, (depth, in_loop));
                }
                StmtKind::Assign(dst, Rvalue::Use(Operand::Copy(src) | Operand::Move(src))) if src.is_bare() => {
                    if dst.is_bare() {
                        // `val n = <temp>` keeps the allocation's depth.
                        if let Some(&a) = alloc.get(&src.local) {
                            alloc.insert(dst.local, a);
                        }
                    } else if let (Some(&(owner, _)), Some(&(value, in_loop))) = (alloc.get(&dst.local), alloc.get(&src.local)) {
                        let name = body.local(src.local).and_then(|l| l.name.clone());
                        if let (true, Some(name)) = (value > owner, name) {
                            let message = if in_loop {
                                format!("`{name}` is declared inside a loop, so it is freed on every iteration; {LOOP_HINT}")
                            } else {
                                format!(
                                    "`{name}` is freed before the value that stores it, but that value would still point at it; \
                                     build it directly in the constructor, or declare it at the top level of the function"
                                )
                            };
                            found.push((s.span, message));
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(t) = &block.term {
            for next in t.kind.successors() {
                work.push((next, stack.clone()));
            }
        }
    }
    found
}

const LOOP_HINT: &str = "declare it before the loop and return it from inside, or build the returned value outside the loop";
