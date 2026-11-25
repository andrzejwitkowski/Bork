//! Borrow conflicts on MIR (RFC 0006 M4c, #219): a new `Borrow` of a place conflicts with a
//! borrow of an overlapping place that is still live (read later) when either side is `&rw`
//! (RFC 0003 section 5, RFC 0004 borrow sets). Unlike `typeck::check_aliased_borrows` it follows
//! borrows through locals, so `val b = &a; f(&rw a, &b)` is rejected. Run from `check` (`frontend::object_lifetimes`)
//! over what `main` reaches. A view of a `Vec` element (`&v[i]`, #282) also locks the Vec until its last use.
// shortcut: only direct borrows and views copied through locals are tracked. Borrows stored in aggregates
// or returned by calls cannot occur yet (typeck rejects them, `ret_borrows` is always empty); extend this
// pass when RFC 0004 views / RFC 0005 returned borrows land (#348). Liveness-based (NLL), not RFC 0004 3.6 by-scope.
// Two-phase: an `&rw` into an unnamed call-argument temp is `reserved` until the call (`h(&rw a, id(&a))` is fine).

use std::collections::{HashMap, HashSet};

use super::body::*;
use crate::hir::VecOp;
use crate::span::Span;

#[derive(Clone, PartialEq)]
struct Loan {
    root: Place,
    rw: bool,
    /// A view of one `Vec` element (`&v[i]`): the Vec is also locked against mutation and moves while it is live.
    elem: bool,
    /// Locals this loan was reborrowed through (`val s = &r` where `r` holds the loan): while `s` is used they are
    /// suspended, so they do not conflict with it.
    via: Vec<Local>,
    /// Two-phase: an `&rw` into an unnamed call-argument temp, not active until the call.
    reserved: bool,
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
/// Per block: locals live after each statement, and locals live just before the terminator.
fn liveness(body: &Body) -> Vec<(Vec<HashSet<Local>>, HashSet<Local>)> {
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
            let before_term = live.clone();
            let mut after = Vec::new();
            for s in body.blocks[i].stmts.iter().rev() {
                after.push(live.clone());
                step_live(&mut live, uses(|f| s.kind.walk(f)));
            }
            after.reverse();
            (after, before_term)
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
    // (new holder, old holder) pairs already reported, so a reborrow of the new holder does not repeat them.
    let mut reported: HashSet<(Local, Local)> = HashSet::new();
    let mut state_in: Vec<Option<Loans>> = vec![None; body.blocks.len()];
    state_in[0] = Some(Loans::new());
    let mut work = vec![0usize];
    while let Some(i) = work.pop() {
        let mut st = state_in[i].clone().unwrap();
        let block = &body.blocks[i];
        for (k, s) in block.stmts.iter().enumerate() {
            let after = &live[i].0[k];
            lock_conflicts(body, &s.kind, s.span, &st, after, &names, &mut found);
            match &s.kind {
                StmtKind::VecOp { dst: Some(dst), op: op @ (VecOp::View | VecOp::ViewRw), receiver, .. } if dst.is_bare() => {
                    // Through an alias (`val a = &rw v; &a[0]`) the Vec is the alias's own root.
                    let rw = *op == VecOp::ViewRw;
                    let roots: Vec<Place> = match receiver.place().filter(|p| p.is_bare()) {
                        Some(p) => st.get(&p.local).map_or_else(|| vec![p.clone()], |ls| ls.iter().map(|l| l.root.clone()).collect()),
                        None => Vec::new(),
                    };
                    if roots.is_empty() {
                        st.remove(&dst.local);
                    } else {
                        st.insert(dst.local, roots.into_iter().map(|root| Loan { root, rw, elem: true, via: Vec::new(), reserved: false }).collect());
                    }
                }
                StmtKind::Borrow { dst, src, rw } if dst.is_bare() => {
                    let inherited = src.is_bare().then(|| st.get(&src.local).cloned()).flatten();
                    let temp = *rw && is_arg_temp(body, &names, dst.local);
                    // A reborrow of a borrow holds the same roots, shared when the reborrow is shared.
                    let loans: Vec<Loan> = match inherited {
                        Some(l) => l.into_iter().map(|o| { let mut via = o.via.clone(); via.push(src.local); Loan { rw: o.rw && *rw, via, reserved: temp && o.rw, ..o } }).collect(),
                        None => vec![Loan { root: src.clone(), rw: *rw, elem: false, via: Vec::new(), reserved: temp }],
                    };
                    for new in loans.iter().filter(|l| !l.elem) {
                        for (holder, held) in &st {
                            if *holder == dst.local || new.via.contains(holder) || !after.contains(holder) || reported.contains(&(*holder, src.local)) {
                                continue;
                            }
                            // Element views are reported by `lock_conflicts`.
                            if let Some(old) = held.iter().find(|o| !o.elem && overlap(&o.root, &new.root) && (new.rw || o.rw)) {
                                if !new.rw && old.reserved && ends_in_nested_call(block, *holder, dst.local) {
                                    continue;
                                }
                                let msg = call_message(body, block, *holder, dst.local, old, new, &names)
                                    .unwrap_or_else(|| general_message(body, *holder, old, new, &names));
                                reported.insert((dst.local, *holder));
                                if !found.contains(&(s.span, msg.clone())) {
                                    found.push((s.span, msg));
                                }
                            }
                        }
                    }
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
        if let Some(Terminator { kind, span, .. }) = &block.term {
            if let TermKind::Call { args, .. } | TermKind::CallClosure { args, .. } = kind {
                // A move in a call argument is not a statement: check it against loans live up to the call.
                let before = &live[i].1;
                for (holder, loan) in st.iter().filter(|(h, _)| before.contains(h)).flat_map(|(h, ls)| ls.iter().map(move |l| (*h, l))) {
                    let root = loan.root.local;
                    if !loan.elem && holder != root && args.iter().any(|a| matches!(a, Operand::Move(p) if p.is_bare() && p.local == root)) {
                        let msg = format!("cannot move `{}` while `{}`, a borrow of it, is still in use", name(body, root, &names), name(body, holder, &names));
                        if !found.contains(&(*span, msg.clone())) {
                            found.push((*span, msg));
                        }
                    }
                }
            }
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

/// Mutating `Vec` operations, moves and overwrites of a `Vec` while a live view of one of its elements exists
/// (`push` may reallocate the buffer the view points into). Reads are fine under a shared view; an `&rw` view
/// is exclusive.
/// Whether `local` is `root` or holds a loan on it (`val alias = &rw v` reaches `v`; `v.push(..)` passes `&v` in a temporary).
fn reaches(st: &Loans, local: Local, root: Local) -> bool {
    local == root || st.get(&local).is_some_and(|ls| ls.iter().any(|l| l.root.local == root))
}

fn lock_conflicts(body: &Body, kind: &StmtKind, span: Span, st: &Loans, after: &HashSet<Local>, names: &HashMap<Local, String>, found: &mut Vec<(Span, String)>) {
    let mut loans = st.iter().filter(|(h, _)| after.contains(h)).flat_map(|(h, ls)| ls.iter().map(move |l| (*h, l))).peekable();
    if loans.peek().is_none() {
        return;
    }
    let (_, kill) = uses(|f| kind.walk(f));
    let mut moved = Vec::new();
    let mut written = Vec::new();
    kind.walk(&mut |u| match u {
        Use::Op(Operand::Move(p)) | Use::Consume(p) => moved.push(p.local),
        Use::Def(p) => written.push(p.clone()),
        _ => {}
    });
    // A write through a holder (`r[0] = v` with `r = &rw a`) conflicts with another live holder of the same owner,
    // such as a reborrow `s = &r`; the holder's own ancestors are suspended, so they do not count.
    for w in written.iter().filter(|w| !w.is_bare() && matches!(kind, StmtKind::Assign(..))) {
        for wl in st.get(&w.local).into_iter().flatten().filter(|l| !l.elem) {
            for (h, ls) in st.iter().filter(|(h, _)| **h != w.local && after.contains(*h) && !wl.via.contains(*h)) {
                if ls.iter().any(|l| !l.elem && overlap(&l.root, &wl.root)) {
                    let msg = format!("cannot assign through `{}` while `{}`, a borrow of `{}`, is still in use", name(body, w.local, names), name(body, *h, names), name(body, wl.root.local, names));
                    if !found.contains(&(span, msg.clone())) {
                        found.push((span, msg));
                    }
                }
            }
        }
    }
    for (holder, loan) in loans {
        let root = loan.root.local;
        let what = match kind {
            // A mutating `Vec` operation on a plainly borrowed `Vec` writes it (not through the holder or a reborrow of it).
            StmtKind::VecOp { op, receiver, .. } if !loan.elem && op.mutates() && receiver.place().is_some_and(|p| p.local != holder && reaches(st, p.local, root) && !st.get(&p.local).is_some_and(|ls| ls.iter().any(|l| l.via.contains(&holder)))) => {
                format!("call `{}` on", op.name())
            }
            StmtKind::VecOp { op, receiver, .. }
                if loan.elem
                    && (op.mutates() || *op == VecOp::ViewRw || loan.rw)
                    && receiver.place().is_some_and(|p| reaches(st, p.local, root)) =>
            {
                if matches!(op, VecOp::View | VecOp::ViewRw) { "borrow an element of".to_string() } else { format!("call `{}` on", op.name()) }
            }
            // Taking `&rw v` under a view, or any borrow of `v` under an `&rw` view.
            StmtKind::Borrow { src, rw, .. } if loan.elem && reaches(st, src.local, root) && (*rw || loan.rw) => "borrow".into(),
            _ if kill.contains(&root) || (!loan.elem && written.iter().any(|w| overlap(w, &loan.root))) => "assign to".into(),
            _ if moved.contains(&root) => "move".into(),
            _ => continue,
        };
        let held = if loan.elem { "a view of one of its elements" } else { "a borrow of it" };
        let msg = format!("cannot {what} `{}` while `{}`, {held}, is still in use", name(body, root, names), name(body, holder, names));
        if !found.contains(&(span, msg.clone())) {
            found.push((span, msg));
        }
    }
}

/// Whether `l` is an unnamed temp passed as an argument of some call (the reserved phase of a two-phase borrow).
fn is_arg_temp(body: &Body, names: &HashMap<Local, String>, l: Local) -> bool {
    !names.contains_key(&l)
        && body.local(l).is_some_and(|d| d.name.is_none())
        && body.blocks.iter().filter_map(|b| b.term.as_ref()).any(|t| {
            matches!(&t.kind, TermKind::Call { args, .. } | TermKind::CallClosure { args, .. } if args.iter().any(|a| a.place().is_some_and(|p| p.is_bare() && p.local == l)))
        })
}

/// Two-phase borrow: a reserved `&rw a` (`holder`, not yet passed) does not conflict with a shared borrow
/// `dst` that a nested call consumes without returning it, as in `h(&rw a, id(&a))`.
fn ends_in_nested_call(block: &Block, holder: Local, dst: Local) -> bool {
    let (args, returns_borrows) = match block.term.as_ref().map(|t| &t.kind) {
        Some(TermKind::Call { args, ret_borrows, .. }) => (args, !ret_borrows.is_empty()),
        Some(TermKind::CallClosure { args, .. }) => (args, false),
        _ => return false,
    };
    let uses_local = |l: Local| args.iter().any(|a| a.place().is_some_and(|p| p.is_bare() && p.local == l));
    !returns_borrows && uses_local(dst) && !uses_local(holder)
}

fn shown(root: &str, rw: bool) -> String {
    if rw { format!("`&rw {root}`") } else { format!("`&{root}`") }
}

fn general_message(body: &Body, holder: Local, old: &Loan, new: &Loan, names: &HashMap<Local, String>) -> String {
    let root = name(body, new.root.local, names);
    format!(
        "{} borrows `{root}` while {} (held by `{}`) is still in use; an `&rw` borrow must be exclusive",
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
        "argument {} ({}) and argument {} ({}) of `{callee}` borrow `{root}`; an `&rw` borrow must be exclusive",
        a + 1, shown(&root, old.rw), b + 1, shown(&root, new.rw),
    ))
}
