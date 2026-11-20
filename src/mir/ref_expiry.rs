//! Warning for a `Ref` that is stored into a longer-lived object and dies with its block (#308).
//!
//! The runtime makes a reference from a newer arena to an older one weak (docs/references.md, "Wygasły weak"), so
//! `p = P(r)` inside a block, with `r` declared in that block and `p` outside, leaves `p.r` empty once the block ends.
//! The language rule stays; this only tells the user.
//!
//! shortcut: one pass over the statements in block order, no dataflow; a `Ref` local that points at an object living
//! in an outer arena is warned about too. Refine when false warnings are reported.

use std::collections::HashMap;

use super::body::*;
use crate::span::Span;

/// `(span of the assignment that outlives the Ref, message)`.
pub fn check(body: &Body) -> Vec<(Span, String)> {
    let mut found = Vec::new();
    // A temp copy of a named `Ref` is the same handle.
    let mut alias: HashMap<Local, Local> = HashMap::new();
    // Named `Ref` locals an object (temp or variable) stores in its fields.
    let mut held: HashMap<Local, Vec<Local>> = HashMap::new();
    let origin = |alias: &HashMap<Local, Local>, mut l: Local| {
        while body.local(l).is_some_and(|d| d.kind == LocalKind::Temp) {
            match alias.get(&l) {
                Some(&next) => l = next,
                None => break,
            }
        }
        l
    };
    let bare = |op: &Operand| op.place().filter(|p| p.is_bare()).map(|p| p.local);
    for block in &body.blocks {
        for s in &block.stmts {
            match &s.kind {
                StmtKind::Ref(RefOp::Clone { dst, src }) if dst.is_bare() => {
                    if let Some(src) = bare(src) {
                        alias.insert(dst.local, src);
                    }
                }
                StmtKind::Ref(RefOp::Store { slot, value, .. }) => {
                    if let Some(v) = bare(value) {
                        held.entry(slot.local).or_default().push(origin(&alias, v));
                    }
                }
                StmtKind::Assign(dst, Rvalue::Use(src)) if dst.is_bare() => {
                    let Some(src) = bare(src) else { continue };
                    alias.insert(dst.local, src);
                    let Some(targets) = held.get(&src).cloned() else { continue };
                    let Some(owner) = body.local(dst.local) else { continue };
                    let Some(owner_name) = owner.name.clone() else {
                        held.entry(dst.local).or_default().extend(targets);
                        continue;
                    };
                    for t in &targets {
                        let Some(target) = body.local(*t) else { continue };
                        let Some(name) = target.name.clone() else { continue };
                        if matches!(target.kind, LocalKind::Val | LocalKind::Var) && target.region != owner.region && encloses(body, owner.region, target.region) {
                            found.push((s.span, format!("`{name}` is freed at the end of its block, but `{owner_name}` outlives it and stores a Ref to it; reading that Ref later gives none")));
                        }
                    }
                }
                _ => {}
            }
        }
    }
    found
}

fn encloses(body: &Body, ancestor: RegionId, region: RegionId) -> bool {
    let mut cur = body.regions[region.0 as usize].parent;
    while let Some(r) = cur {
        if r == ancestor {
            return true;
        }
        cur = body.regions[r.0 as usize].parent;
    }
    false
}
