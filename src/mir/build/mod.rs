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
        BuildError::Unsupported(Unsupported { capability, span })
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
    let functions = &mono.hir.functions;
    let roots = &mono.report.roots;
    if functions.len() != roots.len() {
        return Err(BuildError::Internal(format!("{} functions but {} report roots", functions.len(), roots.len())));
    }
    let abis: HashMap<&str, CallAbi> = functions.iter().map(|f| (f.name.as_str(), call_abi(f, tcx))).collect();
    functions
        .iter()
        .zip(roots)
        .map(|(function, root)| {
            if root.label != format!("fun {}", function.name) {
                return Err(BuildError::Internal(format!("report root `{}` for function `{}`", root.label, function.name)));
            }
            FnBuilder::new(mono, tcx, &abis, function, root).build()
        })
        .collect()
}

struct Open {
    id: RegionId,
    pushed: bool,
}

struct LoopCtx {
    exit: BlockId,
    latch: BlockId,
    /// `open.len()` once the loop's own region is open: `break`/`continue` leave everything above it.
    depth: usize,
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
}

impl<'a> FnBuilder<'a> {
    fn new(
        mono: &'a Mono,
        tcx: &'a TyCtx,
        abis: &'a HashMap<&'a str, CallAbi>,
        function: &'a HirFunction,
        root: &'a ArenaNode,
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
            scopes: vec![HashMap::new()],
            unit_binds: Default::default(),
            cursors: vec![(root, 0)],
            abi: call_abi(function, tcx),
            ret: Local(0),
            unit: tcx.prim_id(Prim::Unit),
            bool_ty: tcx.prim_id(Prim::Bool),
            sink: ArenaRef::Current,
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
        self.regions.push(RegionInfo { parent: None, site: RegionSite::Block });
        self.open.push(Open { id: RegionId(0), pushed: true });
        self.stmt(StmtKind::RegionEnter(RegionId(0)), span, None);
        for (local, param) in (1u32..).map(Local).zip(&function.params) {
            self.register_ref_local(local, param.ty, span);
        }
        let (body, _) = peel_blocks(&function.body);
        self.stmts(body)?;
        if !self.terminated() {
            if function.return_ty == self.unit {
                self.exit_to(0, span);
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
        for block in &mut blocks {
            if block.term.is_none() {
                block.term = Some(Terminator { span, node: None, kind: TermKind::Unreachable });
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
        self.locals.push(LocalDecl { ty, kind, name });
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

    pub(super) fn stmt(&mut self, kind: StmtKind, span: Span, node: Option<NodeId>) {
        self.stmt_with(kind, span, node, None);
    }

    pub(super) fn stmt_with(&mut self, kind: StmtKind, span: Span, node: Option<NodeId>, debug: Option<DebugLabel>) {
        self.ensure_open();
        self.blocks[self.cur.0 as usize].stmts.push(Statement { span, node, kind, debug });
    }

    pub(super) fn terminate(&mut self, kind: TermKind, span: Span, node: Option<NodeId>) {
        self.ensure_open();
        self.blocks[self.cur.0 as usize].term = Some(Terminator { span, node, kind });
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
        let pushed = child.codegen_push || force;
        let parent = self.open.last().map(|o| o.id);
        self.regions.push(RegionInfo { parent, site });
        let id = RegionId(self.regions.len() as u32 - 1);
        if pushed {
            match enter_in {
                Some(b) => self.blocks[b.0 as usize].stmts.push(Statement { span, node: None, kind: StmtKind::RegionEnter(id), debug: None }),
                None => self.stmt(StmtKind::RegionEnter(id), span, None),
            }
        }
        self.open.push(Open { id, pushed });
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
        let open = self.open.pop().expect("open region");
        if open.pushed && !self.terminated() {
            self.stmt(StmtKind::RegionExit(open.id), span, None);
        }
        Ok(())
    }

    /// `RegionExit` for every pushed region above `depth`, innermost first, without closing them:
    /// `return`, `break` and `continue` leave regions on the way out only.
    pub(super) fn exit_to(&mut self, depth: usize, span: Span) {
        let ids: Vec<RegionId> = self.open[depth..].iter().rev().filter(|o| o.pushed).map(|o| o.id).collect();
        for id in ids {
            self.stmt(StmtKind::RegionExit(id), span, None);
        }
    }

    pub(super) fn innermost_pushed(&self) -> Option<RegionId> {
        self.open.iter().rev().find(|o| o.pushed).map(|o| o.id)
    }

    pub(super) fn open_depth(&self) -> usize {
        self.open.len()
    }

    pub(super) fn push_loop(&mut self, exit: BlockId, latch: BlockId) {
        self.loops.push(LoopCtx { exit, latch, depth: self.open.len() });
    }

    pub(super) fn pop_loop(&mut self) {
        self.loops.pop();
    }

    pub(super) fn loop_ctx(&self) -> Option<(BlockId, BlockId, usize)> {
        self.loops.last().map(|l| (l.exit, l.latch, l.depth))
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
