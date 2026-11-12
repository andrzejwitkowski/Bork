//! Allocation planning on MIR (RFC 0006 section 3.3, M4b #218): which object / enum allocations need a dynamic
//! arena and which of those live for the whole program. Port of the provenance rules of `memory.rs`, as
//! flow-insensitive dataflow over `Alloc` results, local assignments and `RefOp::Store`.
//! Call conservatism is kept (no escape summaries), as in the old pass.

use std::collections::{HashMap, HashSet};

use super::body::*;
use crate::hir::{NodeId, VecOp};
use crate::sema::ArenaNode;
use crate::span::Span;
use crate::ty::{TyCtx, TyId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArenaClass {
    Lexical,
    Dynamic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifetimeDomain {
    Program,
    LexicalScope(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaPlan {
    /// The allocating expression; the key every lookup uses.
    pub node: NodeId,
    /// Where it is in the source; for display and ordering only.
    pub allocation_site: Span,
    pub class_name: String,
    pub class: ArenaClass,
    pub lifetime_domain: LifetimeDomain,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemoryPlan {
    pub allocations: Vec<ArenaPlan>,
}

impl MemoryPlan {
    pub fn allocation_at(&self, node: NodeId) -> Option<&ArenaPlan> {
        self.allocations.iter().find(|allocation| allocation.node == node)
    }
}


type Origins = HashSet<NodeId>;

#[derive(Default)]
struct State {
    org: HashMap<(usize, Local), Origins>,
    field: HashMap<(NodeId, u32), Origins>,
    dynamic: HashSet<NodeId>,
    program: HashSet<NodeId>,
    edges: HashSet<(NodeId, NodeId)>,
    nested: HashSet<(NodeId, NodeId)>,
    returned: HashSet<NodeId>,
}

impl State {
    fn size(&self) -> usize {
        self.org.values().map(HashSet::len).sum::<usize>()
            + self.field.values().map(HashSet::len).sum::<usize>()
            + self.dynamic.len()
            + self.program.len()
            + self.edges.len()
            + self.nested.len()
            + self.returned.len()
    }

    fn mark_dynamic(&mut self, o: &Origins) {
        self.dynamic.extend(o.iter().copied());
    }

    fn mark_program(&mut self, o: &Origins) {
        self.mark_dynamic(o);
        self.program.extend(o.iter().copied());
    }

    fn add_ref_edges(&mut self, sources: &Origins, targets: &Origins) {
        self.mark_dynamic(targets);
        if sources.is_empty() {
            self.mark_program(targets);
            return;
        }
        for s in sources {
            for t in targets {
                self.edges.insert((*s, *t));
            }
        }
    }
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

/// Accumulates the origins and lifetime facts of one body into `st` (`id` keeps bodies' locals apart).
struct Pass<'a> {
    body: &'a Body,
    id: usize,
    tcx: &'a TyCtx,
    st: &'a mut State,
    sites: &'a mut HashMap<NodeId, ArenaPlan>,
}

fn analyse(body: &Body, id: usize, tcx: &TyCtx, st: &mut State, sites: &mut HashMap<NodeId, ArenaPlan>) {
    let mut pass = Pass { body, id, tcx, st, sites };
    for block in &body.blocks {
        for s in &block.stmts {
            pass.stmt(s);
        }
        if let Some(Terminator { kind: TermKind::Call { args, .. }, .. }) = &block.term {
            pass.escape_args(args);
        }
    }
}

impl Pass<'_> {
    fn local_origins(&self, l: Local) -> Origins {
        self.st.org.get(&(self.id, l)).cloned().unwrap_or_default()
    }

    fn origins(&self, op: &Operand) -> Origins {
        op.place().map(|p| self.local_origins(p.local)).unwrap_or_default()
    }

    fn add(&mut self, l: Local, more: &Origins) {
        if !more.is_empty() {
            self.st.org.entry((self.id, l)).or_default().extend(more.iter().copied());
        }
    }

    /// Everything `l` may hold lives for the whole program.
    fn mark_local_program(&mut self, l: Local) {
        let all = self.local_origins(l);
        self.st.mark_program(&all);
    }

    fn ty_of(&self, l: Local) -> TyId {
        self.body.locals[l.0 as usize].ty
    }

    /// The type of a bare local operand, if it is one.
    fn operand_ty(&self, op: &Operand) -> Option<TyId> {
        op.place().map(|p| self.ty_of(p.local))
    }

    /// Calls have no escape summaries: a record passed to one becomes program-lived.
    fn escape_args(&mut self, args: &[Operand]) {
        for a in args {
            if a.place().is_some_and(|p| p.is_bare() && self.tcx.owns_result(self.ty_of(p.local))) {
                let v = self.origins(a);
                self.st.mark_program(&v);
            }
        }
    }

    fn stmt(&mut self, s: &Statement) {
        match &s.kind {
            StmtKind::Alloc { dst, kind: AllocKind::Object { class: name } | AllocKind::EnumSlot { enum_name: name, .. }, .. } => {
                self.alloc(s, dst, name)
            }
            StmtKind::Assign(dst, rv) => self.assign(s, dst, rv),
            StmtKind::Ref(op) => self.ref_op(op),
            // Builtins that are calls in HIR (`strongCount`, `weakCount`, intrinsics): arguments become program-lived.
            StmtKind::Intrinsic { args, .. } => self.escape_args(args),
            StmtKind::Observe { object, value, .. } => {
                let v = self.origins(value);
                self.add(*object, &v);
            }
            StmtKind::VecOp { op: VecOp::Push | VecOp::Set, args, .. } => self.vec_store(args),
            StmtKind::Promote { dst, src, .. } | StmtKind::Move { dst, src } | StmtKind::Borrow { dst, src, .. } | StmtKind::CopyInto { dst, src, .. } => {
                let v = self.local_origins(src.local);
                self.add(dst.local, &v);
            }
            _ => {}
        }
    }

    fn alloc(&mut self, s: &Statement, dst: &Place, name: &str) {
        let (Some(node), true) = (s.node, dst.is_bare()) else { return };
        let scope = self.body.regions[s.region.0 as usize].arena;
        self.sites.entry(node).or_insert_with(|| ArenaPlan {
            node,
            allocation_site: s.span,
            class_name: name.to_owned(),
            class: ArenaClass::Lexical,
            lifetime_domain: LifetimeDomain::LexicalScope(scope),
        });
        self.add(dst.local, &Origins::from([node]));
    }

    fn assign(&mut self, s: &Statement, dst: &Place, rv: &Rvalue) {
        let value = match rv {
            Rvalue::Use(op) => self.origins(op),
            Rvalue::Binary(_, a, b) => {
                let mut v = self.origins(a);
                v.extend(self.origins(b));
                v
            }
            Rvalue::Unary(_, a) | Rvalue::Cast(a, _) | Rvalue::Slice { base: a, .. } => self.origins(a),
            Rvalue::Aggregate(_, ops) => ops.iter().flat_map(|op| self.origins(op)).collect(),
            _ => Origins::new(),
        };
        let src_ty = match rv {
            Rvalue::Use(op) => self.operand_ty(op),
            _ => None,
        };
        match dst.proj.last() {
            None => self.assign_local(s, dst.local, &value),
            Some(Proj::Field(_)) => self.assign_field(dst.local, &value, src_ty),
            Some(Proj::Index(_)) => self.assign_index(dst.local, &value, src_ty),
            Some(Proj::Some) => self.add(dst.local, &value),
            _ => {}
        }
    }

    /// `x = value` on a whole local: the return slot, or a variable / result temp assigned from an inner region.
    fn assign_local(&mut self, s: &Statement, local: Local, value: &Origins) {
        let decl = &self.body.locals[local.0 as usize];
        self.add(local, value);
        if local == self.body.ret {
            if self.tcx.is_managed_ref(decl.ty) {
                self.st.mark_program(value);
            } else {
                self.st.returned.extend(value.iter().copied());
            }
        } else if encloses(self.body, decl.region, s.region)
            && (decl.kind == LocalKind::Var || (decl.kind == LocalKind::Temp && self.tcx.owns_result(decl.ty)))
        {
            self.mark_local_program(local);
        }
    }

    /// A nested object stored in an object, or a `Ref` array stored in a field.
    fn assign_field(&mut self, local: Local, value: &Origins, src_ty: Option<TyId>) {
        let tcx = self.tcx;
        if src_ty.is_some_and(|t| tcx.array_elem(t).is_some_and(|e| tcx.is_managed_ref(e))) {
            self.st.mark_program(value);
        } else if src_ty.is_some_and(|t| crate::hoist::holds_object(tcx, t)) {
            let owners = self.local_origins(local);
            // A node-less slot is the `Some(..)` wrapper: it passes the payload's origins on.
            if owners.is_empty() {
                self.add(local, value);
            }
            for owner in owners {
                for inner in value {
                    self.st.nested.insert((owner, *inner));
                }
            }
        }
    }

    /// An array literal carries the origins of its elements; a `Ref` element makes them program-lived.
    fn assign_index(&mut self, local: Local, value: &Origins, src_ty: Option<TyId>) {
        self.add(local, value);
        if src_ty.is_some_and(|t| self.tcx.is_managed_ref(t)) {
            self.mark_local_program(local);
        }
    }

    fn vec_store(&mut self, args: &[Operand]) {
        let pushes_ref = args.last().and_then(|a| self.operand_ty(a)).is_some_and(|t| self.tcx.is_managed_ref(t));
        if pushes_ref {
            let all = args.iter().flat_map(|a| self.origins(a)).collect();
            self.st.mark_program(&all);
        }
    }

    fn ref_op(&mut self, op: &RefOp) {
        match op {
            RefOp::Create { dst, object, .. } => {
                let v = self.origins(object);
                self.st.mark_dynamic(&v);
                self.add(dst.local, &v);
            }
            RefOp::Clone { dst, src } => {
                let v = self.clone_origins(src);
                self.add(dst.local, &v);
            }
            RefOp::MoveOut { dst, src } => {
                let v = self.local_origins(src.local);
                self.add(dst.local, &v);
            }
            RefOp::Rehome { dst, value, .. } => {
                let v = self.origins(value);
                self.add(dst.local, &v);
            }
            RefOp::Store { slot, value, .. } => self.ref_store(slot, value),
            RefOp::Count { value, .. } => {
                let v = self.origins(value);
                self.st.mark_program(&v);
            }
            _ => {}
        }
    }

    /// A `Ref` read out of a field holds what was stored into that field; any other copy keeps its source's origins.
    fn clone_origins(&self, src: &Operand) -> Origins {
        let Some(p) = src.place() else { return Origins::new() };
        let Some(Proj::Field(i)) = p.proj.last() else { return self.local_origins(p.local) };
        let mut out = Origins::new();
        for source in self.local_origins(p.local) {
            if let Some(stored) = self.st.field.get(&(source, *i)) {
                out.extend(stored.iter().copied());
            }
        }
        out
    }

    fn ref_store(&mut self, slot: &Place, value: &Operand) {
        let targets = self.origins(value);
        if let Some(Proj::Index(_)) = slot.proj.last() {
            self.add(slot.local, &targets);
            self.mark_local_program(slot.local);
            return;
        }
        let sources = self.local_origins(slot.local);
        self.st.add_ref_edges(&sources, &targets);
        let field = slot.proj.iter().rev().find_map(|p| if let Proj::Field(i) = p { Some(*i) } else { None });
        if let Some(i) = field {
            for source in sources {
                self.st.field.entry((source, i)).or_default().extend(targets.iter().copied());
            }
        }
    }
}

/// Plan of every object / enum allocation in `bodies`, sorted by source position like `memory::plan`.
pub fn derive_memory_plan(bodies: &[Body], tcx: &TyCtx) -> MemoryPlan {
    let mut st = State::default();
    let mut sites = HashMap::new();
    loop {
        let before = st.size();
        for (id, body) in bodies.iter().enumerate() {
            analyse(body, id, tcx, &mut st, &mut sites);
        }
        // Lifetimes spread along edges and nesting until nothing changes.
        loop {
            let mut grew = false;
            let nested: Vec<_> = st.nested.iter().copied().collect();
            for (owner, inner) in nested {
                if st.returned.contains(&owner) {
                    grew |= st.returned.insert(inner);
                }
            }
            let edges: Vec<_> = st.edges.iter().copied().collect();
            for (source, target) in edges {
                if (st.program.contains(&source) || st.returned.contains(&source)) && !st.program.contains(&target) {
                    st.mark_program(&Origins::from([target]));
                    grew = true;
                }
            }
            if !grew {
                break;
            }
        }
        if st.size() == before {
            break;
        }
    }
    let mut allocations: Vec<ArenaPlan> = sites.into_values().collect();
    for a in &mut allocations {
        if st.dynamic.contains(&a.node) {
            a.class = ArenaClass::Dynamic;
        }
        if st.program.contains(&a.node) {
            a.lifetime_domain = LifetimeDomain::Program;
        }
    }
    allocations.sort_by_key(|a| a.allocation_site.start);
    MemoryPlan { allocations }
}

/// `ArenaNode::id`s of the regions of `body` that need a real `arena_push` (RFC 0006 3.3): the function root, and
/// every region that, itself or through a nested region, allocates, registers a drop, creates a dynamic arena or
/// observes a presence guard. `body` must come from `build_program_neutral` so that every region is visible.
pub fn derive_push(body: &Body) -> HashSet<usize> {
    let mut out = HashSet::from([body.regions[0].arena]);
    for s in body.blocks.iter().flat_map(|b| &b.stmts) {
        // A presence guard pushes its own region only (its ancestors have to allocate on their own).
        if matches!(s.kind, StmtKind::ObservationRegister { .. }) {
            out.insert(body.regions[s.region.0 as usize].arena);
        }
        let needs = match &s.kind {
            StmtKind::Alloc { arena, .. }
            | StmtKind::RegisterDrop { arena, .. }
            | StmtKind::CopyInto { arena, .. }
            | StmtKind::Promote { arena, .. } => lands_here(arena),
            StmtKind::ArenaCreateDynamic { .. } => true,
            StmtKind::Intrinsic { arena, .. } => arena.as_ref().is_some_and(lands_here),
            _ => false,
        };
        if needs || has_str(&s.kind) {
            mark(body, &mut out, s.region);
        }
    }
    for t in body.blocks.iter().filter_map(|b| b.term.as_ref()) {
        if matches!(&t.kind, TermKind::Call { args, .. } if args.iter().any(is_str)) {
            mark(body, &mut out, t.region);
        }
    }
    out
}

/// Only what lands in a region's own arena (or a fresh one made there) needs the push; a result-sink, root or
/// caller arena does not.
fn lands_here(a: &ArenaRef) -> bool {
    matches!(a, ArenaRef::Current | ArenaRef::Region(_) | ArenaRef::Fresh(_) | ArenaRef::Home(_))
}

fn is_str(op: &Operand) -> bool {
    matches!(op, Operand::Const(Const::Str(_)))
}

/// A string literal is an allocation in the old rule (`HirExprKind::Str`).
fn has_str(kind: &StmtKind) -> bool {
    match kind {
        StmtKind::Assign(_, Rvalue::Use(op)) => is_str(op),
        StmtKind::Assign(_, Rvalue::Binary(_, a, b)) => is_str(a) || is_str(b),
        StmtKind::Intrinsic { args, .. } | StmtKind::VecOp { args, .. } => args.iter().any(is_str),
        _ => false,
    }
}

fn mark(body: &Body, out: &mut HashSet<usize>, region: RegionId) {
    let mut cur = Some(region);
    while let Some(r) = cur {
        out.insert(body.regions[r.0 as usize].arena);
        cur = body.regions[r.0 as usize].parent;
    }
}

/// Writes the push decision into the report: `codegen_push` is true exactly for the regions in `pushed`.
pub fn stamp_push(node: &mut ArenaNode, pushed: &HashSet<usize>) {
    node.codegen_push = pushed.contains(&node.id);
    node.children.iter_mut().for_each(|c| stamp_push(c, pushed));
}
