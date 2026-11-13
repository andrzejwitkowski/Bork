//! Lowers monomorphized HIR to MIR (RFC 0006 section 3), one [`Body`] per function.
//!
//! Region structure comes from the sema `ArenaReport`: its children are matched to HIR regions purely
//! by the order the walk consumes them (`region_walk`), so every lowering below takes children in the
//! same order as `RegionVisitor` does (condition before its regions, `&&`/`||`/`?:` right-hand sides
//! never, arms in order).

mod expr;
mod own;
mod place;
mod refs;
mod stmt;

use std::collections::HashMap;

use super::body::*;
use super::capability::{Capability, Unsupported};
use crate::hir::{peel_blocks, HirBlock, HirFunction, NodeId, Prim};
use crate::mono::Mono;
use crate::region_walk::RegionSite;
use crate::sema::ArenaNode;
use crate::span::Span;
use crate::ty::{TyCtx, TyId};

#[derive(Debug, Clone, PartialEq)]
pub enum BuildError {
    Unsupported(Unsupported),
    /// The report and the HIR disagree: a compiler bug, not a user error.
    Internal(String),
    /// A construct the language accepts that the builder cannot express and RFC 3.10 has no capability for.
    Unlowered { what: String, span: Span },
}

pub type Res<T> = Result<T, BuildError>;

impl BuildError {
    pub(super) fn unsupported(capability: Capability, span: Span) -> BuildError {
        BuildError::Unsupported(Unsupported { capability, span, message: None })
    }
    pub(super) fn unsupported_with(capability: Capability, span: Span, message: &'static str) -> BuildError {
        BuildError::Unsupported(Unsupported { capability, span, message: Some(message) })
    }
    pub(super) fn unlowered(what: impl Into<String>, span: Span) -> BuildError {
        BuildError::Unlowered { what: what.into(), span }
    }
}

/// Hidden parameters of `function` (what `codegen/llvm/call_abi.rs` decides today).
pub fn call_abi(function: &HirFunction, tcx: &TyCtx) -> CallAbi {
    if function.name == "main" {
        CallAbi::default()
    } else if tcx.owns_result(function.return_ty) || tcx.uses_arena_storage(function.return_ty) {
        CallAbi { caller_parent: true, result_sink: true }
    } else {
        CallAbi { caller_parent: true, result_sink: false }
    }
}

pub fn build_program(mono: &Mono, tcx: &TyCtx) -> Res<Vec<Body>> {
    build_program_with(mono, tcx, false)
}

/// MIR that ignores `mono.memory_plan` and the `codegen_push` flags: every allocation goes to the sink and every
/// region is entered. Input of the allocation analyses in `plan` (M4b), which derive both from it.
pub fn build_program_neutral(mono: &Mono, tcx: &TyCtx) -> Res<Vec<Body>> {
    build_program_with(mono, tcx, true)
}

/// One result per function, in `mono.hir.functions` order, so the MIR backend can fall back per function.
pub fn build_function_results(mono: &Mono, tcx: &TyCtx) -> Res<Vec<Res<Body>>> {
    build_each(mono, tcx, false)
}

fn build_program_with(mono: &Mono, tcx: &TyCtx, neutral: bool) -> Res<Vec<Body>> {
    build_each(mono, tcx, neutral)?.into_iter().collect()
}

/// Whole-program shapes LLVM has no type for: `unit` as a parameter, a class field or a `Vec`/array element, and a
/// `main` that is not `fun main(): i32|unit`. Reported here so the gate sees them (declaration and layout would
/// otherwise fail after it).
fn check_signatures(mono: &Mono, tcx: &TyCtx) -> Res<()> {
    let (unit, i32_ty) = (tcx.prim_id(Prim::Unit), tcx.prim_id(Prim::I32));
    // A bare `unit` is reported as `own`; a `Vec`/array of `unit` inside the type is always `UnitElem`.
    let shape = |ty: TyId, own: Capability, span: Span| {
        let cap = if ty == unit { own } else if has_unit_elem(tcx, ty) { Capability::UnitElem } else { return Ok(()) };
        Err(BuildError::unsupported(cap, span))
    };
    for class in &mono.hir.classes {
        for field in &class.fields {
            shape(field.ty, Capability::UnitField, field.span)?;
        }
    }
    // A `unit` enum payload is laid out as zero-sized, so only the element rule applies there.
    for field in mono.hir.enums.iter().flat_map(|e| &e.variants).flat_map(|v| &v.fields) {
        shape(if field.ty == unit { i32_ty } else { field.ty }, Capability::UnitElem, field.span)?;
    }
    for f in &mono.hir.functions {
        for p in &f.params {
            shape(p.ty, Capability::UnitParam, f.name_span)?;
        }
        shape(if f.return_ty == unit { i32_ty } else { f.return_ty }, Capability::UnitElem, f.name_span)?;
        if f.name == "main" && (!f.params.is_empty() || (f.return_ty != i32_ty && f.return_ty != unit)) {
            return Err(BuildError::unsupported(Capability::MainSignature, f.name_span));
        }
    }
    Ok(())
}

/// Whether `ty` holds a `Vec` or array of `unit` anywhere inside it.
fn has_unit_elem(tcx: &TyCtx, ty: TyId) -> bool {
    let unit = tcx.prim_id(Prim::Unit);
    tcx.vec_elem(ty).or_else(|| tcx.array_elem(ty)).is_some_and(|e| e == unit) || tcx.data(ty).children().into_iter().any(|c| has_unit_elem(tcx, c))
}

fn build_each(mono: &Mono, tcx: &TyCtx, neutral: bool) -> Res<Vec<Res<Body>>> {
    check_signatures(mono, tcx)?;
    let functions = &mono.hir.functions;
    let roots = &mono.report.roots;
    if functions.len() != roots.len() {
        return Err(BuildError::Internal(format!("{} functions but {} report roots", functions.len(), roots.len())));
    }
    let abis: HashMap<&str, CallAbi> = functions.iter().map(|f| (f.name.as_str(), call_abi(f, tcx))).collect();
    Ok(functions
        .iter()
        .zip(roots)
        .map(|(function, root)| {
            if root.label.strip_prefix("fun ") != Some(function.name.as_str()) {
                return Err(BuildError::Internal(format!("report root `{}` for function `{}`", root.label, function.name)));
            }
            FnBuilder::new(mono, tcx, &abis, function, root, neutral).build()
        })
        .collect())
}

struct Open {
    id: RegionId,
    pushed: bool,
}

/// A point in the open regions and live debug scopes that `exit_to` unwinds to. Captured together so the two
/// stacks cannot disagree.
#[derive(Clone, Copy, Default)]
pub(super) struct Mark {
    /// `open.len()`
    depth: usize,
    /// `dscopes.len()`
    scopes: usize,
}

struct LoopCtx {
    exit: BlockId,
    latch: BlockId,
    /// Taken once the loop's own region is open: `break`/`continue` leave everything above it, including the body scope.
    mark: Mark,
}

pub(super) struct FnBuilder<'a> {
    pub(super) mono: &'a Mono,
    pub(super) tcx: &'a TyCtx,
    abis: &'a HashMap<&'a str, CallAbi>,
    pub(super) function: &'a HirFunction,
    pub(super) locals: Vec<LocalDecl>,
    blocks: Vec<Block>,
    pub(super) cur: BlockId,
    regions: Vec<RegionInfo>,
    open: Vec<Open>,
    loops: Vec<LoopCtx>,
    /// Open debug scopes (`open.len()` before the region they belong to was pushed, scope local), outermost first.
    /// Live debug scopes, innermost last.
    dscopes: Vec<Local>,
    scopes: Vec<HashMap<String, Local>>,
    /// Names bound to a `unit` payload: no local exists, so reading one is the `unit-payload-bind` capability gap.
    pub(super) unit_binds: std::collections::HashSet<String>,
    cursors: Vec<(&'a ArenaNode, usize)>,
    pub(super) abi: CallAbi,
    ret: Local,
    pub(super) unit: TyId,
    pub(super) bool_ty: TyId,
    /// Arena that fresh payloads are allocated in (`alloc_sink` in the LLVM emitter).
    pub(super) sink: ArenaRef,
    /// See [`build_program_neutral`].
    pub(super) neutral: bool,
}

impl<'a> FnBuilder<'a> {
    fn new(
        mono: &'a Mono,
        tcx: &'a TyCtx,
        abis: &'a HashMap<&'a str, CallAbi>,
        function: &'a HirFunction,
        root: &'a ArenaNode,
        neutral: bool,
    ) -> Self {
        FnBuilder {
            mono,
            tcx,
            abis,
            function,
            locals: Vec::new(),
            blocks: vec![Block::default()],
            cur: BlockId(0),
            regions: Vec::new(),
            open: Vec::new(),
            loops: Vec::new(),
            dscopes: Vec::new(),
            scopes: vec![HashMap::new()],
            unit_binds: Default::default(),
            cursors: vec![(root, 0)],
            abi: call_abi(function, tcx),
            ret: Local(0),
            unit: tcx.prim_id(Prim::Unit),
            bool_ty: tcx.prim_id(Prim::Bool),
            sink: ArenaRef::Current,
            neutral,
        }
    }

    fn build(mut self) -> Res<Body> {
        let function = self.function;
        self.ret = self.new_local(function.return_ty, LocalKind::Return, None);
        for param in &function.params {
            let local = self.new_local(param.ty, LocalKind::Arg, Some(param.name.clone()));
            self.declare(&param.name, local);
        }
        let span = function.name_span;
        self.regions.push(RegionInfo { parent: None, site: RegionSite::Block, arena: self.cursors[0].0.id });
        self.scope_enter(span, None);
        self.open.push(Open { id: RegionId(0), pushed: true });
        self.stmt(StmtKind::RegionEnter(RegionId(0)), span, None);
        for (local, param) in (1u32..).map(Local).zip(&function.params) {
            self.register_ref_local(local, param.ty, span);
        }
        let (body, _) = peel_blocks(&function.body);
        self.stmts(body)?;
        if !self.terminated() {
            if function.return_ty == self.unit {
                self.exit_to(Mark::default(), span);
                self.terminate(TermKind::Return, span, None);
            } else {
                // Typeck guarantees a non-unit function returns on every path.
                self.terminate(TermKind::Unreachable, span, None);
            }
        }
        if self.cursors.last().is_some_and(|(n, next)| *next != n.children.len()) {
            return Err(BuildError::Internal(format!("function `{}` left report regions unconsumed", function.name)));
        }
        let mut blocks = self.blocks;
        // A `Vec`/array of `unit` is allocated in the body that writes `Vec<()>(...)` or `[(), ()]`.
        for stmt in blocks.iter().flat_map(|b| &b.stmts) {
            if let StmtKind::Alloc { dst, kind: AllocKind::Vec { .. } | AllocKind::Array, .. } = &stmt.kind {
                if self.locals.get(dst.local.0 as usize).is_some_and(|l| has_unit_elem(self.tcx, l.ty)) {
                    return Err(BuildError::unsupported(Capability::UnitElem, stmt.span));
                }
            }
        }
        for block in &mut blocks {
            if block.term.is_none() {
                block.term = Some(Terminator { span, node: None, kind: TermKind::Unreachable, region: RegionId(0) });
            }
        }
        Ok(Body {
            name: function.name.clone(),
            locals: self.locals,
            blocks,
            regions: self.regions,
            abi: self.abi,
            ret: self.ret,
        })
    }

    // --- locals, scopes ---

    pub(super) fn new_local(&mut self, ty: TyId, kind: LocalKind, name: Option<String>) -> Local {
        let region = self.cur_region();
        self.locals.push(LocalDecl { ty, kind, name, region });
        Local(self.locals.len() as u32 - 1)
    }

    pub(super) fn temp(&mut self, ty: TyId) -> Local {
        self.new_local(ty, LocalKind::Temp, None)
    }

    pub(super) fn declare(&mut self, name: &str, local: Local) {
        self.scopes.last_mut().expect("a scope is always open").insert(name.to_string(), local);
    }

    /// A name the HIR uses must be in scope; anything else is a builder bug.
    pub(super) fn lookup(&self, name: &str, span: Span) -> Res<Local> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied()).ok_or_else(|| {
            if self.unit_binds.contains(name) {
                BuildError::unsupported(Capability::UnitPayloadBind, span)
            } else {
                BuildError::Internal(format!("name `{name}` is not in scope in `{}`", self.function.name))
            }
        })
    }

    pub(super) fn is_unit(&self, ty: TyId) -> bool {
        ty == self.unit
    }

    // --- blocks ---

    pub(super) fn new_block(&mut self) -> BlockId {
        self.blocks.push(Block::default());
        BlockId(self.blocks.len() as u32 - 1)
    }

    pub(super) fn terminated(&self) -> bool {
        self.blocks[self.cur.0 as usize].term.is_some()
    }

    /// Code after `return`/`break` still has to be walked so report regions are consumed; it lands in
    /// a block nothing jumps to.
    fn ensure_open(&mut self) {
        if self.terminated() {
            self.cur = self.new_block();
        }
    }

    fn cur_region(&self) -> RegionId {
        self.open.last().map_or(RegionId(0), |o| o.id)
    }

    pub(super) fn stmt(&mut self, kind: StmtKind, span: Span, node: Option<NodeId>) {
        self.stmt_with(kind, span, node, None);
    }

    pub(super) fn stmt_with(&mut self, kind: StmtKind, span: Span, node: Option<NodeId>, debug: Option<DebugLabel>) {
        self.ensure_open();
        let region = self.cur_region();
        self.blocks[self.cur.0 as usize].stmts.push(Statement { span, node, kind, debug, region });
    }

    pub(super) fn terminate(&mut self, kind: TermKind, span: Span, node: Option<NodeId>) {
        self.ensure_open();
        let region = self.cur_region();
        self.blocks[self.cur.0 as usize].term = Some(Terminator { span, node, kind, region });
    }

    pub(super) fn goto(&mut self, target: BlockId, span: Span) {
        if !self.terminated() {
            self.terminate(TermKind::Goto(target), span, None);
        }
    }

    // --- regions ---

    fn take_child(&mut self, site: RegionSite) -> Res<&'a ArenaNode> {
        let (node, next) = self.cursors.last_mut().expect("root cursor");
        let node: &'a ArenaNode = node;
        let child = node
            .children
            .get(*next)
            .ok_or_else(|| BuildError::Internal(format!("no report region left for {site:?} in `{}`", node.label)))?;
        if !site.matches(&child.label) {
            return Err(BuildError::Internal(format!("{site:?} does not match report region `{}`", child.label)));
        }
        *next += 1;
        Ok(child)
    }

    /// Opens the report region for `site`. `enter_in` is the block the `RegionEnter` goes to when it is
    /// not the current one (a `while` evaluates its condition before its region is known).
    pub(super) fn enter_region<'b>(
        &mut self,
        site: RegionSite,
        body: &'b HirBlock,
        force: bool,
        enter_in: Option<BlockId>,
        span: Span,
    ) -> Res<(&'b HirBlock, RegionId, bool)> {
        let child = self.take_child(site)?;
        let (peeled, count) = peel_blocks(body);
        if count != child.compacted_braces {
            return Err(BuildError::Internal(format!(
                "`{}` peels {count} blocks but the report says {}",
                child.label, child.compacted_braces
            )));
        }
        let pushed = self.neutral || child.codegen_push || force;
        let parent = self.open.last().map(|o| o.id);
        self.regions.push(RegionInfo { parent, site, arena: child.id });
        let id = RegionId(self.regions.len() as u32 - 1);
        if pushed {
            match enter_in {
                Some(b) => self.blocks[b.0 as usize].stmts.push(Statement { span, node: None, kind: StmtKind::RegionEnter(id), debug: None, region: id }),
                None => self.stmt(StmtKind::RegionEnter(id), span, None),
            }
        }
        self.open.push(Open { id, pushed });
        // The region is pushed, then its debug scope is entered.
        self.scope_enter(span, enter_in);
        self.cursors.push((child, 0));
        self.scopes.push(HashMap::new());
        Ok((peeled, id, pushed))
    }

    /// Closes the innermost region, emitting its exit at the current block when it is still reachable.
    pub(super) fn exit_region(&mut self, span: Span) -> Res<()> {
        let (node, next) = self.cursors.pop().expect("region cursor");
        if next != node.children.len() {
            return Err(BuildError::Internal(format!("region `{}` left {} children unconsumed", node.label, node.children.len() - next)));
        }
        self.scopes.pop();
        self.scope_exit(span);
        let open = self.open.pop().expect("open region");
        if open.pushed && !self.terminated() {
            self.stmt(StmtKind::RegionExit(open.id), span, None);
        }
        Ok(())
    }

    /// `StorageDead` for the debug scopes above `to`, then `RegionExit` for every pushed region above it, innermost
    /// first, without closing them: `return`, `break`, `continue` and a failed presence guard leave regions on the way
    /// out only. Scopes are marked by position, not region depth: a loop body's scope shares its depth with the
    /// regions nested in it.
    pub(super) fn exit_to(&mut self, to: Mark, span: Span) {
        let depth = to.depth;
        // Debug scopes are left before any region, innermost first.
        let dead: Vec<Local> = self.dscopes[to.scopes..].iter().rev().copied().collect();
        for l in dead {
            self.stmt(StmtKind::StorageDead(l), span, None);
        }
        let ids: Vec<RegionId> = self.open[depth..].iter().rev().filter(|o| o.pushed).map(|o| o.id).collect();
        for id in ids {
            self.stmt(StmtKind::RegionExit(id), span, None);
        }
    }

    fn new_scope_local(&mut self) -> Local {
        let l = self.new_local(self.unit, LocalKind::Hidden(HiddenKind::DebugScope), None);
        self.dscopes.push(l);
        l
    }

    /// Opens a debug scope; `in_block` is where a `while` enters its region (not the current block).
    pub(super) fn scope_enter(&mut self, span: Span, in_block: Option<BlockId>) {
        let l = self.new_scope_local();
        match in_block {
            Some(b) => {
                let region = self.cur_region();
                self.blocks[b.0 as usize].stmts.push(Statement { span, node: None, kind: StmtKind::StorageLive(l), debug: None, region });
            }
            None => self.stmt(StmtKind::StorageLive(l), span, None),
        }
    }

    /// Leaves the innermost debug scope; nothing is emitted when the block already ended.
    pub(super) fn scope_exit(&mut self, span: Span) {
        let l = self.dscopes.pop().expect("open debug scope");
        if !self.terminated() {
            self.stmt(StmtKind::StorageDead(l), span, None);
        }
    }

    pub(super) fn innermost_pushed(&self) -> Option<RegionId> {
        self.open.iter().rev().find(|o| o.pushed).map(|o| o.id)
    }

    pub(super) fn mark(&self) -> Mark {
        Mark { depth: self.open.len(), scopes: self.dscopes.len() }
    }


    pub(super) fn push_loop(&mut self, exit: BlockId, latch: BlockId) {
        self.loops.push(LoopCtx { exit, latch, mark: self.mark() });
    }

    pub(super) fn pop_loop(&mut self) {
        self.loops.pop();
    }

    pub(super) fn loop_ctx(&self) -> Option<(BlockId, BlockId, Mark)> {
        self.loops.last().map(|l| (l.exit, l.latch, l.mark))
    }

    pub(super) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(super) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    pub(super) fn with_sink<T>(&mut self, sink: ArenaRef, f: impl FnOnce(&mut Self) -> Res<T>) -> Res<T> {
        let saved = std::mem::replace(&mut self.sink, sink);
        let r = f(self);
        self.sink = saved;
        r
    }

    pub(super) fn abi_of(&self, name: &str) -> Option<CallAbi> {
        self.abis.get(name).copied()
    }
}
