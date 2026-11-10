//! Allocation planning on MIR (RFC 0006 section 3.3, M4b #218): which object / enum allocations need a dynamic
//! arena and which of those live for the whole program. Port of the provenance rules of `memory.rs`, as
//! flow-insensitive dataflow over `Alloc` results, local assignments and `RefOp::Store`.
//! Call conservatism is kept (no escape summaries), as in the old pass.

use std::collections::{HashMap, HashSet};

use super::body::*;
use crate::hir::{NodeId, VecOp};
use crate::memory::{ArenaClass, ArenaPlan, LifetimeDomain, MemoryPlan};
use crate::sema::ArenaNode;
use crate::ty::TyCtx;

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
fn analyse(body: &Body, id: usize, tcx: &TyCtx, st: &mut State, sites: &mut HashMap<NodeId, ArenaPlan>) {
    let ty_of = |l: Local| body.locals[l.0 as usize].ty;
    let o = |st: &State, op: &Operand| -> Origins {
        op.place().and_then(|p| st.org.get(&(id, p.local)).cloned()).unwrap_or_default()
    };
    let add = |st: &mut State, l: Local, more: &Origins| {
        if !more.is_empty() {
            st.org.entry((id, l)).or_default().extend(more.iter().copied());
        }
    };
    for block in &body.blocks {
        for s in &block.stmts {
            match &s.kind {
                StmtKind::Alloc { dst, kind: AllocKind::Object { class: name } | AllocKind::EnumSlot { enum_name: name, .. }, .. } => {
                    if let (Some(node), true) = (s.node, dst.is_bare()) {
                        sites.entry(node).or_insert_with(|| ArenaPlan {
                            node,
                            allocation_site: s.span,
                            class_name: name.clone(),
                            class: ArenaClass::Lexical,
                            lifetime_domain: LifetimeDomain::LexicalScope(body.regions[s.region.0 as usize].arena),
                        });
                        add(st, dst.local, &Origins::from([node]));
                    }
                }
                StmtKind::Assign(dst, rv) => {
                    let value = match rv {
                        Rvalue::Use(op) => o(st, op),
                        Rvalue::Binary(_, a, b) => {
                            let mut v = o(st, a);
                            v.extend(o(st, b));
                            v
                        }
                        Rvalue::Unary(_, a) | Rvalue::Cast(a, _) | Rvalue::Slice { base: a, .. } => o(st, a),
                        Rvalue::Aggregate(_, ops) => ops.iter().flat_map(|op| o(st, op)).collect(),
                        _ => Origins::new(),
                    };
                    let src_ty = match rv {
                        Rvalue::Use(op) => op.place().map(|p| ty_of(p.local)),
                        _ => None,
                    };
                    let decl = &body.locals[dst.local.0 as usize];
                    match dst.proj.last() {
                        None => {
                            add(st, dst.local, &value);
                            let outer = encloses(body, decl.region, s.region);
                            if dst.local == body.ret {
                                if tcx.is_managed_ref(decl.ty) {
                                    st.mark_program(&value);
                                } else {
                                    st.returned.extend(value.iter().copied());
                                }
                            } else if outer && (decl.kind == LocalKind::Var || (decl.kind == LocalKind::Temp && tcx.owns_result(decl.ty))) {
                                let all = st.org.get(&(id, dst.local)).cloned().unwrap_or_default();
                                st.mark_program(&all);
                            }
                        }
                        Some(Proj::Field(_)) => {
                            // nested object stored in an object, or a Ref array stored in a field.
                            if src_ty.is_some_and(|t| tcx.array_elem(t).is_some_and(|e| tcx.is_managed_ref(e))) {
                                st.mark_program(&value);
                            } else if src_ty.is_some_and(|t| crate::hoist::holds_object(tcx, t)) {
                                let owners = st.org.get(&(id, dst.local)).cloned().unwrap_or_default();
                                // A node-less slot is the `Some(..)` wrapper: it passes the payload's origins on.
                                if owners.is_empty() {
                                    add(st, dst.local, &value);
                                }
                                for owner in owners {
                                    for inner in &value {
                                        st.nested.insert((owner, *inner));
                                    }
                                }
                            }
                        }
                        Some(Proj::Index(_)) => {
                            // An array literal carries the origins of its elements; a `Ref` element makes them program-lived.
                            add(st, dst.local, &value);
                            if src_ty.is_some_and(|t| tcx.is_managed_ref(t)) {
                                let all = st.org.get(&(id, dst.local)).cloned().unwrap_or_default();
                                st.mark_program(&all);
                            }
                        }
                        Some(Proj::Some) => add(st, dst.local, &value),
                        _ => {}
                    }
                }
                StmtKind::Ref(RefOp::Create { dst, object, .. }) => {
                    let v = o(st, object);
                    st.mark_dynamic(&v);
                    add(st, dst.local, &v);
                }
                StmtKind::Ref(RefOp::Clone { dst, src }) => {
                    let mut v = o(st, src);
                    if let Some(p) = src.place() {
                        if let Some(Proj::Field(i)) = p.proj.last() {
                            let mut f = Origins::new();
                            for source in st.org.get(&(id, p.local)).cloned().unwrap_or_default() {
                                if let Some(x) = st.field.get(&(source, *i)) {
                                    f.extend(x.iter().copied());
                                }
                            }
                            v = f;
                        }
                    }
                    add(st, dst.local, &v);
                }
                StmtKind::Ref(RefOp::MoveOut { dst, src }) => {
                    let v = st.org.get(&(id, src.local)).cloned().unwrap_or_default();
                    add(st, dst.local, &v);
                }
                StmtKind::Ref(RefOp::Rehome { dst, value, .. }) => {
                    let v = o(st, value);
                    add(st, dst.local, &v);
                }
                StmtKind::Ref(RefOp::Store { slot, value, .. }) => {
                    let targets = o(st, value);
                    let sources = st.org.get(&(id, slot.local)).cloned().unwrap_or_default();
                    if let Some(Proj::Index(_)) = slot.proj.last() {
                        add(st, slot.local, &targets);
                        let all = st.org.get(&(id, slot.local)).cloned().unwrap_or_default();
                        st.mark_program(&all);
                    } else {
                        st.add_ref_edges(&sources, &targets);
                        let idx = slot.proj.iter().rev().find_map(|p| if let Proj::Field(i) = p { Some(*i) } else { None });
                        if let Some(i) = idx {
                            for source in sources {
                                st.field.entry((source, i)).or_default().extend(targets.iter().copied());
                            }
                        }
                    }
                }
                // Builtins that are calls in HIR (`strongCount`, `weakCount`, intrinsics): arguments become program-lived.
                StmtKind::Ref(RefOp::Count { value, .. }) => {
                    let v = o(st, value);
                    st.mark_program(&v);
                }
                StmtKind::Intrinsic { args, .. } => {
                    for a in args {
                        if a.place().is_some_and(|p| p.is_bare() && tcx.owns_result(ty_of(p.local))) {
                            let v = o(st, a);
                            st.mark_program(&v);
                        }
                    }
                }
                StmtKind::Observe { object, value, .. } => {
                    let v = o(st, value);
                    add(st, *object, &v);
                }
                StmtKind::VecOp { op: VecOp::Push | VecOp::Set, args, .. } => {
                    if args.last().and_then(Operand::place).is_some_and(|p| tcx.is_managed_ref(ty_of(p.local))) {
                        let mut all = Origins::new();
                        for a in args {
                            all.extend(o(st, a));
                        }
                        st.mark_program(&all);
                    }
                }
                StmtKind::Promote { dst, src, .. } | StmtKind::Move { dst, src } | StmtKind::Borrow { dst, src, .. } | StmtKind::CopyInto { dst, src, .. } => {
                    let v = st.org.get(&(id, src.local)).cloned().unwrap_or_default();
                    add(st, dst.local, &v);
                }
                _ => {}
            }
        }
        if let Some(Terminator { kind: TermKind::Call { args, .. }, .. }) = &block.term {
            for a in args {
                if a.place().is_some_and(|p| p.is_bare() && tcx.owns_result(ty_of(p.local))) {
                    let v = o(st, a);
                    st.mark_program(&v);
                }
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
