//! MIR verifier v0 (RFC 0006 3.6). A failure is an internal compiler error.
//!
//! Checked here: CFG shape, references to locals/regions/hidden ABI, region nesting on every path, and
//! that each `Ref` temporary is defined and consumed exactly once per path. Init/move dataflow and
//! arena-lifetime enclosure need real bodies and arrive with `build.rs` / M3.

use std::collections::{BTreeSet, HashMap};
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
        let mut stack = vec![(0usize, State { regions: Vec::new(), live: BTreeSet::new() })];
        while let Some((bi, entry)) = stack.pop() {
            match seen.get(&bi) {
                Some(prev) if *prev == entry => continue,
                Some(_) => {
                    self.err(Some(BlockId(bi as u32)), None, "paths reach this block with different open regions or live Ref temporaries");
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
                s.kind.walk(&mut |u| events.push(self.temp_event(u)));
                self.apply_temps(&mut st, events, b, Some(si));
            }
            let Some(term) = &block.term else { continue };
            let mut events = Vec::new();
            term.kind.walk(&mut |u| events.push(self.temp_event(u)));
            self.apply_temps(&mut st, events, b, None);
            match &term.kind {
                TermKind::Return => {
                    if !st.regions.is_empty() {
                        self.err(Some(b), None, "return leaves regions open");
                    }
                    if !st.live.is_empty() {
                        self.err(Some(b), None, format!("return with unconsumed Ref temporaries {:?}", st.live));
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
