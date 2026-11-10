//! Borrow conflicts on MIR (RFC 0006 M4c, #219): a new `Borrow` of a place conflicts with a
//! borrow of an overlapping place that is still live (read later) when either side is `&rw`
//! (RFC 0003 section 5, RFC 0004 borrow sets). Unlike `typeck::check_aliased_borrows` it follows
//! borrows through locals, so `val b = &a; f(&rw a, &b)` is rejected. Not wired into `check`
//! yet (needs MIR for every function, #243); tests compare it with the old pass.
// shortcut: only direct borrows and views copied through locals are tracked (not borrows stored
// in aggregates or returned from calls beyond `ret_borrows`); writes/moves of a borrowed place and
// Vec reallocation under a live view are RFC 0004 section 3.2 rules, left to a follow-up.

use std::collections::{HashMap, HashSet};

use super::body::*;
use crate::span::Span;

#[derive(Clone, PartialEq)]
struct Loan {
    root: Place,
    rw: bool,
}

type Loans = HashMap<Local, Vec<Loan>>;

fn overlap(a: &Place, b: &Place) -> bool {
    a.local == b.local
        && a.proj.iter().zip(&b.proj).all(|(x, y)| !matches!((x, y), (Proj::Field(i), Proj::Field(j)) if i != j))
}

/// Locals read by `walk` of a statement/terminator, and bare locals it overwrites.
fn uses(walk: impl FnOnce(&mut dyn FnMut(Use<'_>))) -> (Vec<Local>, Vec<Local>) {
    let (mut read, mut kill) = (Vec::new(), Vec::new());
    walk(&mut |u| match u {
        Use::Op(o) => read.extend(o.place().map(|p| p.local)),
        Use::Read(p) | Use::Consume(p) => read.push(p.local),
        Use::Local(l) => read.push(l),
        Use::Def(p) if p.is_bare() => kill.push(p.local),
        Use::Def(p) => read.push(p.local),
        Use::Arena(_) | Use::Region(_) => {}
    });
    (read, kill)
}

fn step_live(live: &mut HashSet<Local>, (read, kill): (Vec<Local>, Vec<Local>)) {
    for k in kill {
        live.remove(&k);
    }
    live.extend(read);
}

fn term_uses(b: &Block) -> (Vec<Local>, Vec<Local>) {
    b.term.as_ref().map_or_else(Default::default, |t| uses(|f| t.kind.walk(f)))
}

/// Locals live after each statement of each block.
fn liveness(body: &Body) -> Vec<Vec<HashSet<Local>>> {
    let n = body.blocks.len();
    let mut live_in = vec![HashSet::<Local>::new(); n];
    let live_out = |i: usize, live_in: &[HashSet<Local>]| -> HashSet<Local> {
        let succ = body.blocks[i].term.as_ref().map_or_else(Vec::new, |t| t.kind.successors());
        succ.iter().flat_map(|s| live_in[s.0 as usize].iter().copied()).collect()
    };
    let mut changed = true;
    while changed {
        changed = false;
        for i in (0..n).rev() {
            let mut live = live_out(i, &live_in);
            step_live(&mut live, term_uses(&body.blocks[i]));
            for s in body.blocks[i].stmts.iter().rev() {
                step_live(&mut live, uses(|f| s.kind.walk(f)));
            }
            if live != live_in[i] {
                live_in[i] = live;
                changed = true;
            }
        }
    }
    (0..n)
        .map(|i| {
            let mut live = live_out(i, &live_in);
            step_live(&mut live, term_uses(&body.blocks[i]));
            let mut after = Vec::new();
            for s in body.blocks[i].stmts.iter().rev() {
                after.push(live.clone());
                step_live(&mut live, uses(|f| s.kind.walk(f)));
            }
            after.reverse();
            after
        })
        .collect()
}

/// Source name of a local: the debug label of the statement that assigned it (`_2 = copy _1  // label "a"`),
/// else its declared name, else `_N`.
fn name(body: &Body, l: Local, names: &HashMap<Local, String>) -> String {
    names.get(&l).cloned().or_else(|| body.local(l)?.name.clone()).unwrap_or_else(|| format!("_{}", l.0))
}

/// `(span of the later borrow, message)`; wording of `check_aliased_borrows` when both borrows
/// are arguments of one call, otherwise a general one naming the variable that holds the old borrow.
pub fn check(body: &Body) -> Vec<(Span, String)> {
    let live = liveness(body);
    let mut names: HashMap<Local, String> = HashMap::new();
    for b in &body.blocks {
        for s in &b.stmts {
            if let (Some(l), StmtKind::Assign(d, _)) = (&s.debug, &s.kind) {
                names.entry(d.local).or_insert_with(|| l.name.clone());
            }
        }
    }
    let mut found: Vec<(Span, String)> = Vec::new();
    let mut state_in: Vec<Option<Loans>> = vec![None; body.blocks.len()];
    state_in[0] = Some(Loans::new());
    let mut work = vec![0usize];
    while let Some(i) = work.pop() {
        let mut st = state_in[i].clone().unwrap();
        let block = &body.blocks[i];
        for (k, s) in block.stmts.iter().enumerate() {
            let after = &live[i][k];
            match &s.kind {
                StmtKind::Borrow { dst, src, rw } if dst.is_bare() => {
                    let inherited = src.is_bare().then(|| st.get(&src.local).cloned()).flatten();
                    let loans = match inherited {
                        Some(l) => l,
                        None => {
                            let new = Loan { root: src.clone(), rw: *rw };
                            for (holder, held) in &st {
                                if *holder == dst.local || !after.contains(holder) {
                                    continue;
                                }
                                if let Some(old) = held.iter().find(|o| overlap(&o.root, &new.root) && (new.rw || o.rw)) {
                                    let msg = call_message(body, block, *holder, dst.local, old, &new, &names)
                                        .unwrap_or_else(|| general_message(body, *holder, old, &new, &names));
                                    if !found.contains(&(s.span, msg.clone())) {
                                        found.push((s.span, msg));
                                    }
                                }
                            }
                            vec![new]
                        }
                    };
                    st.insert(dst.local, loans);
                }
                StmtKind::Assign(dst, Rvalue::Use(Operand::Copy(src) | Operand::Move(src))) if dst.is_bare() && src.is_bare() => {
                    match st.get(&src.local).cloned() {
                        Some(l) => st.insert(dst.local, l),
                        None => st.remove(&dst.local),
                    };
                }
                kind => {
                    let (_, kill) = uses(|f| kind.walk(f));
                    for k in kill {
                        st.remove(&k);
                    }
                }
            }
        }
        if let Some(Terminator { kind, .. }) = &block.term {
            if let TermKind::Call { args, dest, ret_borrows, .. } = kind {
                let carried: Vec<Loan> = ret_borrows
                    .iter()
                    .filter_map(|&a| args.get(a as usize)?.place().and_then(|p| st.get(&p.local)))
                    .flatten()
                    .cloned()
                    .collect();
                if dest.is_bare() {
                    if carried.is_empty() { st.remove(&dest.local); } else { st.insert(dest.local, carried); }
                }
            }
            for next in kind.successors() {
                let n = next.0 as usize;
                let mut merged = state_in[n].clone().unwrap_or_default();
                for (l, v) in &st {
                    let e = merged.entry(*l).or_default();
                    for loan in v {
                        if !e.contains(loan) {
                            e.push(loan.clone());
                        }
                    }
                }
                if state_in[n].as_ref() != Some(&merged) {
                    state_in[n] = Some(merged);
                    work.push(n);
                }
            }
        }
    }
    found
}

fn shown(root: &str, rw: bool) -> String {
    if rw { format!("`&rw {root}`") } else { format!("`&{root}`") }
}

fn general_message(body: &Body, holder: Local, old: &Loan, new: &Loan, names: &HashMap<Local, String>) -> String {
    let root = name(body, new.root.local, names);
    format!(
        "{} borrows `{root}` while {} (held by `{}`) is still in use; `&rw` needs it to itself",
        shown(&root, new.rw),
        shown(&root, old.rw),
        name(body, holder, names),
    )
}

/// Old wording when the block ends in a call taking both borrows as arguments.
fn call_message(body: &Body, block: &Block, old_holder: Local, new_holder: Local, old: &Loan, new: &Loan, names: &HashMap<Local, String>) -> Option<String> {
    let TermKind::Call { callee, args, .. } = &block.term.as_ref()?.kind else { return None };
    let pos = |l: Local| args.iter().position(|a| a.place().is_some_and(|p| p.is_bare() && p.local == l));
    let (a, b) = (pos(old_holder)?, pos(new_holder)?);
    let root = name(body, new.root.local, names);
    Some(format!(
        "argument {} ({}) and argument {} ({}) of `{callee}` borrow `{root}`; `&rw` needs it to itself",
        a + 1, shown(&root, old.rw), b + 1, shown(&root, new.rw),
    ))
}
