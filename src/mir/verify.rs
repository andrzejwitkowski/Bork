//! MIR verifier v0 (RFC 0006 3.6). A failure is an internal compiler error.
//!
//! Checked here, in two kinds (RFC 0006 3.6):
//! - **Structural facts**, identical on every path into a block (the builder emits structured control flow and
//!   lowering keeps one state per block): CFG shape, references to locals/regions/hidden ABI, region nesting, live
//!   `Ref` temporaries, live debug scopes (`StorageLive`/`StorageDead` pair up and none is live at `return`), and that
//!   an arena named by an op exists at that point (`Region(r)` is open, `Current` has an open region or a caller
//!   parent, `Fresh(n)` is a dynamic-arena local).
//! - **Value facts**, joined as a fixpoint over the CFG: a bare local is not read after it was moved (maybe-moved,
//!   union at joins) or before it was initialised (definitely-initialised, intersection at joins).
//!
//! Not checked here: that the value stored into an arena does not outlive it (that is `mir::escape`; a second copy
//! here would be a duplicate), nor moves out of a projected place (`a.f`).

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::fmt;

use super::body::*;
use crate::ty::TyCtx;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyError {
    pub block: Option<BlockId>,
    pub stmt: Option<usize>,
    pub message: String,
}

impl fmt::Display for VerifyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(b) = self.block {
            write!(f, "bb{}", b.0)?;
            if let Some(s) = self.stmt {
                write!(f, "[{s}]")?;
            }
            f.write_str(": ")?;
        }
        f.write_str(&self.message)
    }
}

/// Value facts at a point: definitely-initialised and maybe-moved bare locals.
#[derive(Clone, PartialEq, Eq)]
struct ValueState {
    init: BTreeSet<Local>,
    moved: BTreeSet<Local>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Event {
    Read,
    Consume,
    Define,
}

/// Region stack and live `Ref` temporaries at a point of the CFG.
#[derive(Clone, PartialEq, Eq)]
struct State {
    regions: Vec<RegionId>,
    live: BTreeSet<Local>,
    /// Live `DebugScope` locals, innermost last.
    scopes: Vec<Local>,
}

pub fn verify(body: &Body, tcx: &TyCtx) -> Vec<VerifyError> {
    let mut v = Verifier { body, tcx, errors: Vec::new() };
    v.run();
    v.errors
}

pub fn verify_or_panic(body: &Body, tcx: &TyCtx) {
    let errors = verify(body, tcx);
    if !errors.is_empty() {
        let list: Vec<String> = errors.iter().map(|e| e.to_string()).collect();
        panic!("internal compiler error: invalid MIR for `{}`:\n  {}", body.name, list.join("\n  "));
    }
}

struct Verifier<'a> {
    body: &'a Body,
    tcx: &'a TyCtx,
    errors: Vec<VerifyError>,
}

impl Verifier<'_> {
    fn err(&mut self, block: Option<BlockId>, stmt: Option<usize>, message: impl Into<String>) {
        self.errors.push(VerifyError { block, stmt, message: message.into() });
    }

    fn run(&mut self) {
        let body = self.body;
        if body.blocks.is_empty() {
            self.err(None, None, "body has no blocks");
            return;
        }
        if body.local(body.ret).is_none_or(|l| l.kind != LocalKind::Return) {
            self.err(None, None, "`ret` is not a Return local");
        }
        for (i, r) in body.regions.iter().enumerate() {
            if r.parent.is_some_and(|p| p.0 as usize >= body.regions.len() || p.0 as usize >= i) {
                self.err(None, None, format!("region r{i} has an invalid parent"));
            }
        }
        // Structure first: the path walk below assumes every id resolves.
        for (bi, block) in body.blocks.iter().enumerate() {
            let b = BlockId(bi as u32);
            for (si, s) in block.stmts.iter().enumerate() {
                self.check_uses(b, Some(si), |f| s.kind.walk(f));
            }
            match &block.term {
                None => self.err(Some(b), None, "block has no terminator"),
                Some(t) => {
                    self.check_uses(b, None, |f| t.kind.walk(f));
                    for target in t.kind.successors() {
                        if target.0 as usize >= body.blocks.len() {
                            self.err(Some(b), None, format!("jump to missing bb{}", target.0));
                        }
                    }
                }
            }
        }
        if self.errors.is_empty() {
            self.walk_paths();
        }
        if self.errors.is_empty() {
            self.check_values();
        }
    }

    /// Locals whose value the init/move check follows: variables and temporaries, not the hidden ABI locals.
    fn is_value_local(&self, l: Local) -> bool {
        self.body.local(l).is_some_and(|d| !matches!(d.kind, LocalKind::Hidden(_)))
    }

    /// Value facts as a forward fixpoint from the entry block only (unreachable blocks are never analysed): `moved`
    /// grows by union at joins (maybe-moved), `init` shrinks by intersection (definitely-initialised). A conditional
    /// move with no later read therefore passes, a read after it does not.
    fn check_values(&mut self) {
        let body = self.body;
        let entry = ValueState { init: body.locals.iter().enumerate().filter(|(_, d)| d.kind == LocalKind::Arg).map(|(i, _)| Local(i as u32)).collect(), moved: BTreeSet::new() };
        let mut at: HashMap<usize, ValueState> = HashMap::from([(0, entry)]);
        let mut work = VecDeque::from([0usize]);
        while let Some(bi) = work.pop_front() {
            let mut st = at[&bi].clone();
            self.run_block(bi, &mut st, &mut |_, _| {});
            for next in body.blocks[bi].term.iter().flat_map(|t| t.kind.successors()) {
                let n = next.0 as usize;
                let merged = match at.get(&n) {
                    None => st.clone(),
                    Some(old) => ValueState { init: old.init.intersection(&st.init).copied().collect(), moved: old.moved.union(&st.moved).copied().collect() },
                };
                if at.get(&n) != Some(&merged) {
                    at.insert(n, merged);
                    work.push_back(n);
                }
            }
        }
        // Report from the settled entry states, once per block in order.
        let mut reached: Vec<usize> = at.keys().copied().collect();
        reached.sort_unstable();
        let mut found = Vec::new();
        for bi in reached {
            let mut st = at[&bi].clone();
            self.run_block(bi, &mut st, &mut |s, m| found.push((BlockId(bi as u32), s, m)));
        }
        for (b, s, m) in found {
            self.err(Some(b), s, m);
        }
    }

    /// Advances `st` over block `bi`; `on_err(statement, message)` gets every violation (`None` = the terminator).
    fn run_block(&self, bi: usize, st: &mut ValueState, on_err: &mut dyn FnMut(Option<usize>, String)) {
        let body = self.body;
        let block = &body.blocks[bi];
        for (si, s) in block.stmts.iter().enumerate() {
            self.apply_values(st, |f| s.kind.walk(f)).into_iter().for_each(|m| on_err(Some(si), m));
        }
        let Some(term) = &block.term else { return };
        self.apply_values(st, |f| term.kind.walk(f)).into_iter().for_each(|m| on_err(None, m));
        if matches!(term.kind, TermKind::Return) {
            let ret_unit = body.local(body.ret).is_some_and(|d| d.ty == self.tcx.prim_id(crate::hir::Prim::Unit));
            if !ret_unit && !st.init.contains(&body.ret) {
                on_err(None, "return of a value that was not assigned on every path".into());
            }
        }
    }

    /// One statement or terminator: reads are checked against `st` before its definitions apply (the order `walk`
    /// reports). Returns the violations and advances `st`.
    fn apply_values<'b>(&self, st: &mut ValueState, walk: impl FnOnce(&mut dyn FnMut(Use<'b>))) -> Vec<String> {
        let mut errors: Vec<String> = Vec::new();
        let read = |st: &ValueState, p: &Place, errors: &mut Vec<String>| {
            let mut check = |l: Local| {
                if self.is_value_local(l) {
                    if st.moved.contains(&l) {
                        errors.push(format!("_{} is read after it was moved", l.0));
                    } else if !st.init.contains(&l) {
                        errors.push(format!("_{} is read before it was assigned on every path", l.0));
                    }
                }
            };
            check(p.local);
            for pr in &p.proj {
                if let Proj::Index(idx) = pr {
                    if let Operand::Copy(i) | Operand::Move(i) | Operand::Reborrow(i) = &**idx {
                        check(i.local);
                    }
                }
            }
        };
        let mut moves: Vec<Local> = Vec::new();
        let mut defs: Vec<Local> = Vec::new();
        walk(&mut |u| match u {
            Use::Read(p) | Use::Op(Operand::Copy(p) | Operand::Reborrow(p)) => read(st, p, &mut errors),
            Use::Consume(p) | Use::Op(Operand::Move(p)) => {
                read(st, p, &mut errors);
                if p.is_bare() {
                    moves.push(p.local);
                }
            }
            Use::Def(p) if p.is_bare() => defs.push(p.local),
            Use::Local(l) => defs.push(l),
            _ => {}
        });
        st.moved.extend(moves);
        for l in defs {
            st.moved.remove(&l);
            st.init.insert(l);
        }
        errors
    }

    fn check_uses<'b>(&mut self, b: BlockId, s: Option<usize>, walk: impl FnOnce(&mut dyn FnMut(Use<'b>))) {
        let mut found: Vec<String> = Vec::new();
        let body = self.body;
        let local = |l: Local, found: &mut Vec<String>| {
            if body.local(l).is_none() {
                found.push(format!("use of missing local _{}", l.0));
            }
        };
        fn place_locals(p: &Place, local: &impl Fn(Local, &mut Vec<String>), found: &mut Vec<String>) {
            local(p.local, found);
            for pr in &p.proj {
                if let Proj::Index(o) = pr {
                    op_locals(o, local, found);
                }
            }
        }
        fn op_locals(o: &Operand, local: &impl Fn(Local, &mut Vec<String>), found: &mut Vec<String>) {
            if let Operand::Copy(p) | Operand::Move(p) | Operand::Reborrow(p) = o {
                place_locals(p, local, found);
            }
        }
        let abi = body.abi;
        let nregions = body.regions.len();
        walk(&mut |u| match u {
            Use::Def(p) | Use::Read(p) | Use::Consume(p) => place_locals(p, &local, &mut found),
            Use::Op(o) => op_locals(o, &local, &mut found),
            Use::Local(l) => local(l, &mut found),
            Use::Region(r) if r.0 as usize >= nregions => found.push(format!("missing region r{}", r.0)),
            Use::Region(_) => {}
            Use::Arena(a) => match a {
                ArenaRef::Region(r) if r.0 as usize >= nregions => found.push(format!("missing region r{}", r.0)),
                ArenaRef::Home(l) | ArenaRef::ObjectArena(l) => local(*l, &mut found),
                ArenaRef::ResultSink if !abi.result_sink => {
                    found.push("`result_sink` used but the CallAbi has no result sink".into())
                }
                ArenaRef::CallerParent if !abi.caller_parent => {
                    found.push("`caller_parent` used but the CallAbi has no caller parent".into())
                }
                _ => {}
            },
        });
        for m in found {
            self.err(Some(b), s, m);
        }
    }

    fn is_ref_temp(&self, l: Local) -> bool {
        self.body
            .local(l)
            .is_some_and(|d| d.kind == LocalKind::Temp && self.tcx.is_managed_ref(d.ty))
    }

    /// Depth-first over the CFG; every block must be reached with one agreed state.
    fn walk_paths(&mut self) {
        let mut seen: HashMap<usize, State> = HashMap::new();
        let mut stack = vec![(0usize, State { regions: Vec::new(), live: BTreeSet::new(), scopes: Vec::new() })];
        while let Some((bi, entry)) = stack.pop() {
            match seen.get(&bi) {
                Some(prev) if *prev == entry => continue,
                Some(prev) => {
                    let message = if prev.scopes != entry.scopes && prev.regions == entry.regions && prev.live == entry.live {
                        "paths reach this block with different live debug scopes"
                    } else {
                        "paths reach this block with different open regions or live Ref temporaries"
                    };
                    self.err(Some(BlockId(bi as u32)), None, message);
                    continue;
                }
                None => {
                    seen.insert(bi, entry.clone());
                }
            }
            let b = BlockId(bi as u32);
            let block = &self.body.blocks[bi];
            let mut st = entry;
            for (si, s) in block.stmts.iter().enumerate() {
                match &s.kind {
                    StmtKind::RegionEnter(r) => st.regions.push(*r),
                    StmtKind::RegionExit(r) => {
                        if st.regions.pop() != Some(*r) {
                            self.err(Some(b), Some(si), format!("region_exit r{} does not close the innermost open region", r.0));
                        }
                    }
                    StmtKind::StorageLive(l) => {
                        if self.body.local(*l).is_none_or(|d| d.kind != LocalKind::Hidden(HiddenKind::DebugScope)) {
                            self.err(Some(b), Some(si), "storage_live of a local that is not a DebugScope");
                        } else if st.scopes.contains(l) {
                            self.err(Some(b), Some(si), format!("storage_live _{} while its scope is already live", l.0));
                        } else {
                            st.scopes.push(*l);
                        }
                    }
                    StmtKind::StorageDead(l) => {
                        if st.scopes.pop() != Some(*l) {
                            self.err(Some(b), Some(si), format!("storage_dead _{} does not close the innermost live debug scope", l.0));
                        }
                    }
                    StmtKind::RegionReset { region, dst } => {
                        if st.regions.last() != Some(region) {
                            self.err(Some(b), Some(si), format!("region_reset r{} outside its region", region.0));
                        }
                        if self.body.local(*dst).is_none_or(|d| d.kind != LocalKind::Hidden(HiddenKind::RegionArena)) {
                            self.err(Some(b), Some(si), "region_reset target is not a RegionArena local");
                        }
                    }
                    _ => {}
                }
                let mut events = Vec::new();
                let mut arenas = Vec::new();
                s.kind.walk(&mut |u| {
                    if let Use::Arena(a) = u {
                        arenas.push(a);
                    } else {
                        events.push(self.temp_event(u));
                    }
                });
                for a in arenas {
                    self.check_arena(a, &st, b, Some(si));
                }
                self.apply_temps(&mut st, events, b, Some(si));
            }
            let Some(term) = &block.term else { continue };
            let mut events = Vec::new();
            let mut arenas = Vec::new();
            term.kind.walk(&mut |u| {
                if let Use::Arena(a) = u {
                    arenas.push(a);
                } else {
                    events.push(self.temp_event(u));
                }
            });
            for a in arenas {
                self.check_arena(a, &st, b, None);
            }
            self.apply_temps(&mut st, events, b, None);
            match &term.kind {
                TermKind::Return => {
                    if !st.regions.is_empty() {
                        self.err(Some(b), None, "return leaves regions open");
                    }
                    if !st.live.is_empty() {
                        self.err(Some(b), None, format!("return with unconsumed Ref temporaries {:?}", st.live));
                    }
                    if !st.scopes.is_empty() {
                        self.err(Some(b), None, format!("return leaves debug scopes live {:?}", st.scopes));
                    }
                }
                other => {
                    for next in other.successors() {
                        stack.push((next.0 as usize, st.clone()));
                    }
                }
            }
        }
    }

    /// The arena `a` exists at this point of the path.
    fn check_arena(&mut self, a: &ArenaRef, st: &State, b: BlockId, s: Option<usize>) {
        match a {
            ArenaRef::Region(r) if !st.regions.contains(r) => self.err(Some(b), s, format!("arena r{} is not open here", r.0)),
            ArenaRef::Current if st.regions.is_empty() && !self.body.abi.caller_parent => {
                self.err(Some(b), s, "`current` arena with no open region and no caller parent")
            }
            ArenaRef::Fresh(n) if self.body.local(Local(*n)).is_none_or(|d| d.kind != LocalKind::Hidden(HiddenKind::DynamicArena)) => {
                self.err(Some(b), s, format!("fresh arena _{n} is not a DynamicArena local"))
            }
            _ => {}
        }
    }

    /// The uses that matter to the Ref-temporary check.
    fn temp_event(&self, u: Use<'_>) -> Option<(Local, Event)> {
        let bare_ref = |p: &Place| (p.is_bare() && self.is_ref_temp(p.local)).then_some(p.local);
        match u {
            Use::Def(p) => bare_ref(p).map(|l| (l, Event::Define)),
            Use::Consume(p) => bare_ref(p).map(|l| (l, Event::Consume)),
            Use::Read(p) => bare_ref(p).map(|l| (l, Event::Read)),
            Use::Op(Operand::Move(p)) => bare_ref(p).map(|l| (l, Event::Consume)),
            Use::Op(Operand::Copy(p) | Operand::Reborrow(p)) => bare_ref(p).map(|l| (l, Event::Read)),
            _ => None,
        }
    }

    fn apply_temps(&mut self, st: &mut State, events: Vec<Option<(Local, Event)>>, b: BlockId, s: Option<usize>) {
        // Uses come before the definition within one statement, so reads see the old state.
        for (l, kind) in events.into_iter().flatten() {
            match kind {
                Event::Read | Event::Consume if !st.live.contains(&l) => {
                    self.err(Some(b), s, format!("Ref temporary _{} used while not live (never defined or already consumed)", l.0));
                }
                Event::Consume => {
                    st.live.remove(&l);
                }
                Event::Define => {
                    if !st.live.insert(l) {
                        self.err(Some(b), s, format!("Ref temporary _{} overwritten before being consumed", l.0));
                    }
                }
                Event::Read => {}
            }
        }
    }
}
