//! Where a `val`/`var` initialiser must be allocated so that it outlives its region (RFC 0006 3.3): the arena of the
//! outer binding it is moved into on the next statement, or the caller's result arena when it is returned. Computed
//! on the monomorphic HIR, one decision per declaration keyed by its initialiser's node id.

use std::collections::{HashMap, HashSet};

use crate::hir::{HirAssignTarget, HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, NodeId, UseKind};
use crate::ty::TyCtx;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Sink {
    /// The caller's result arena.
    Result,
    /// The arena of this outer binding.
    Outer(String),
    /// The current region: a callable call result bound in a loop and only ever called, so it cannot outlive the
    /// iteration (#327).
    Local,
}

pub(crate) fn decl_sinks(function: &HirFunction, tcx: &TyCtx) -> HashMap<NodeId, Sink> {
    let mut sinks = HashMap::new();
    let visible: HashSet<String> = function.params.iter().map(|p| p.name.clone()).collect();
    outer_hoists(&function.body, &visible, &mut sinks);
    result_hoists(&function.body, tcx, &mut sinks);
    local_loop_calls(&function.body, tcx, &callee_only_decls(&function.body), false, &mut sinks);
    sinks
}

/// `val f = call()` in a loop body whose result is callable or moves a handle stays in the iteration's region when every
/// mention of `f` in the function is the callee of a call or a `when`/`if val` scrutinee: nothing can carry the value
/// out of the iteration (`return f` would hand out the loop slot, which `materialize` does not copy).
/// A `val q = f` alias counts when `q` is itself callee/scrutinee-only (`callee_only_decls`, which resolves uses by scope).
fn local_loop_calls(block: &HirBlock, tcx: &TyCtx, safe: &HashSet<NodeId>, in_loop: bool, out: &mut HashMap<NodeId, Sink>) {
    for stmt in &block.stmts {
        if let HirStmt::VarDecl { value, .. } = stmt {
            let called = matches!(value.kind, HirExprKind::Call { .. }) && (tcx.is_callable(value.ty) || moves_handle(tcx, value.ty));
            if in_loop && called && !out.contains_key(&value.id) && safe.contains(&value.id) {
                out.insert(value.id, Sink::Local);
            }
        }
        // `when (call())` in a loop: the arms bind clones, so the scrutinee itself never leaves the iteration.
        if let (true, HirStmt::Expr(e)) = (in_loop, stmt) {
            if let HirExprKind::Match { scrutinee, .. } = &e.kind {
                let call = matches!(scrutinee.kind, HirExprKind::Call { .. });
                if call && (tcx.is_callable(scrutinee.ty) || moves_handle(tcx, scrutinee.ty)) {
                    out.entry(scrutinee.id).or_insert(Sink::Local);
                }
            }
        }
        let inner_loop = in_loop || matches!(stmt, HirStmt::For { .. } | HirStmt::While { .. });
        for inner in nested_blocks(stmt) {
            local_loop_calls(inner, tcx, safe, inner_loop, out);
        }
        // `val x = if (c) { val w = call() ... }`: the initializer's blocks are not statements of this block.
        if let HirStmt::VarDecl { value, .. } = stmt {
            for inner in expr_blocks(value) {
                local_loop_calls(inner, tcx, safe, inner_loop, out);
            }
        }
    }
}

/// Mirrors `Builder::moves_handle`.
fn moves_handle(t: &TyCtx, ty: crate::ty::TyId) -> bool {
    let payload = if t.is_nullable(ty) { t.enum_args(ty).and_then(|(_, a)| a.first().copied()).unwrap_or(ty) } else { ty };
    t.enum_args(payload).is_some() || (t.record_def(ty).is_some() && !t.is_managed_ref(ty))
}

/// Initialiser ids of the declarations in `block` whose every use is a callee, a match scrutinee, or a `val` alias that is
/// itself such. Uses resolve to the innermost visible binding, so shadowed and reused names are told apart exactly.
fn callee_only_decls(block: &HirBlock) -> HashSet<NodeId> {
    use crate::hir::{ChildMut, HirPat, HirPatKind};
    /// A binding we do not track (pattern, `for`, closure parameter): it only shadows.
    const UNTRACKED: usize = usize::MAX;
    struct Decl {
        id: NodeId,
        depth: usize,
        escapes: bool,
        aliases: Vec<usize>,
    }
    struct Walk {
        scopes: Vec<HashMap<String, usize>>,
        decls: Vec<Decl>,
        /// Closure / `move` nesting: a use from deeper than the declaration is captured, so it escapes.
        depth: usize,
    }
    impl Walk {
        fn lookup(&self, name: &str) -> Option<usize> {
            self.scopes.iter().rev().find_map(|s| s.get(name)).copied().filter(|&d| d != UNTRACKED)
        }
        fn escape(&mut self, name: &str) {
            if let Some(d) = self.lookup(name) {
                self.decls[d].escapes = true;
            }
        }
        /// A callee or scrutinee use.
        fn plain_use(&mut self, name: &str) {
            if let Some(d) = self.lookup(name) {
                self.decls[d].escapes |= self.decls[d].depth != self.depth;
            }
        }
        fn bind(&mut self, name: &str, d: usize) {
            self.scopes.last_mut().unwrap().insert(name.to_string(), d);
        }
        fn pat(&mut self, p: &HirPat) {
            match &p.kind {
                HirPatKind::Bind { name, .. } => self.bind(&name.name, UNTRACKED),
                HirPatKind::Variant { fields, .. } => fields.iter().for_each(|f| self.pat(f)),
                HirPatKind::Some(inner) => self.pat(inner),
                _ => {}
            }
        }
        fn stmts(&mut self, b: &mut HirBlock) {
            b.stmts.iter_mut().for_each(|s| self.stmt(s));
        }
        fn block(&mut self, b: &mut HirBlock) {
            self.scopes.push(HashMap::new());
            self.stmts(b);
            self.scopes.pop();
        }
        fn child(&mut self, c: ChildMut<'_>) {
            match c {
                ChildMut::Block(b) => self.block(b),
                ChildMut::Expr(e) => self.expr(e),
            }
        }
        /// The name of `e` when it is a bare identifier.
        fn ident(e: &HirExpr) -> Option<String> {
            if let HirExprKind::Ident { name, .. } = &e.kind { Some(name.clone()) } else { None }
        }
        fn stmt(&mut self, s: &mut HirStmt) {
            match s {
                HirStmt::VarDecl { kind, name, value, .. } => {
                    // `val q = w` is an alias: `w` stays safe only if `q` does. Resolved before `q` is in scope.
                    let alias = Self::ident(value)
                        .filter(|_| *kind == crate::ast::BindingKind::Val)
                        .filter(|_| !matches!(value.kind, HirExprKind::Ident { use_kind: UseKind::Move, .. }))
                        .and_then(|n| self.lookup(&n));
                    match alias {
                        Some(t) => self.decls[t].escapes |= self.decls[t].depth != self.depth,
                        None => self.expr(value),
                    }
                    let d = self.decls.len();
                    self.decls.push(Decl { id: value.id, depth: self.depth, escapes: false, aliases: Vec::new() });
                    if let Some(t) = alias {
                        self.decls[t].aliases.push(d);
                    }
                    self.bind(name, d);
                }
                HirStmt::For { name, iter, body } => {
                    self.expr(iter);
                    self.scopes.push(HashMap::from([(name.clone(), UNTRACKED)]));
                    self.stmts(body);
                    self.scopes.pop();
                }
                HirStmt::MoveBlock { captures, body } => {
                    captures.iter().flatten().for_each(|n| self.escape(n));
                    self.depth += 1;
                    self.block(body);
                    self.depth -= 1;
                }
                HirStmt::Assign { target, .. } => {
                    // `children_mut` omits a `Name` target: the write must be seen, the binding now holds a value this analysis did not place.
                    if let Some(name) = target.binding_name() {
                        self.escape(name);
                    }
                    s.children_mut().into_iter().for_each(|c| self.child(c));
                }
                _ => s.children_mut().into_iter().for_each(|c| self.child(c)),
            }
        }
        fn expr(&mut self, e: &mut HirExpr) {
            match &mut e.kind {
                HirExprKind::Ident { name, .. } => self.escape(name),
                HirExprKind::Call { callee, args } => {
                    match Self::ident(callee) {
                        Some(n) => self.plain_use(&n),
                        None => self.expr(callee),
                    }
                    args.iter_mut().for_each(|a| self.expr(a));
                }
                HirExprKind::Match { scrutinee, arms } => {
                    match Self::ident(scrutinee) {
                        Some(n) => self.plain_use(&n),
                        None => self.expr(scrutinee),
                    }
                    for arm in arms {
                        self.scopes.push(HashMap::new());
                        self.pat(&arm.pat);
                        self.stmts(&mut arm.body);
                        self.scopes.pop();
                    }
                }
                HirExprKind::PresenceMatch { bindings, some_block, none_block } => {
                    // shortcut: only a single-binding `if val` counts its bare-ident initializer as a scrutinee use (kept from the
                    // name-based analysis); a chained one escapes, which is safe, just not optimal. Relax with a test if it matters.
                    let single = bindings.tail.is_empty();
                    let mut bound = 0;
                    for b in std::iter::once(&mut *bindings.head).chain(&mut bindings.tail) {
                        match Self::ident(&b.value) {
                            Some(n) if single => self.plain_use(&n),
                            _ => self.expr(&mut b.value),
                        }
                        // A later initializer can use the names bound before it.
                        self.scopes.push(HashMap::from([(b.name.name.clone(), UNTRACKED)]));
                        bound += 1;
                    }
                    self.block(some_block);
                    (0..bound).for_each(|_| drop(self.scopes.pop()));
                    none_block.iter_mut().for_each(|b| self.block(b));
                }
                HirExprKind::Closure { params, body, captures, .. } => {
                    captures.iter().for_each(|c| self.escape(&c.name));
                    self.depth += 1;
                    self.scopes.push(params.iter().map(|p| (p.name.clone(), UNTRACKED)).collect());
                    self.stmts(body);
                    self.scopes.pop();
                    self.depth -= 1;
                }
                _ => e.children_mut().into_iter().for_each(|c| self.child(c)),
            }
        }
    }
    let mut walk = Walk { scopes: vec![HashMap::new()], decls: Vec::new(), depth: 0 };
    walk.stmts(&mut block.clone());
    // An alias is declared after its target, so a reverse pass sees every alias's verdict first.
    let mut safe = vec![false; walk.decls.len()];
    for d in (0..safe.len()).rev() {
        safe[d] = !walk.decls[d].escapes && walk.decls[d].aliases.iter().all(|&a| safe[a]);
    }
    (0..safe.len()).filter(|&d| safe[d]).map(|d| walk.decls[d].id).collect()
}

fn outer_hoists(block: &HirBlock, outer: &HashSet<String>, out: &mut HashMap<NodeId, Sink>) {
    let mut local = HashSet::new();
    for (index, stmt) in block.stmts.iter().enumerate() {
        if let (Some(name), HirStmt::VarDecl { value, .. }) = (hoist_target(stmt, block.stmts.get(index + 1), outer, &local), stmt) {
            out.insert(value.id, Sink::Outer(name));
        }
        match stmt {
            HirStmt::Block(inner) | HirStmt::MoveBlock { body: inner, .. } | HirStmt::For { body: inner, .. } | HirStmt::While { body: inner, .. } => {
                let mut nested = outer.clone();
                nested.extend(local.iter().cloned());
                outer_hoists(inner, &nested, out);
            }
            HirStmt::VarDecl { name, .. } => {
                local.insert(name.clone());
            }
            _ => {}
        }
    }
}

/// `val inner = <init>` immediately followed by `outer = move inner`, with `outer` already in scope.
fn hoist_target(decl: &HirStmt, next: Option<&HirStmt>, outer: &HashSet<String>, local: &HashSet<String>) -> Option<String> {
    let HirStmt::VarDecl { name: inner, value, .. } = decl else { return None };
    let HirStmt::Assign { target: HirAssignTarget::Name { name: outer_name }, value: rhs } = next? else { return None };
    if !value_may_hoist(value) || !assign_moves_ident(rhs, inner) {
        return None;
    }
    (outer.contains(outer_name) || local.contains(outer_name)).then(|| outer_name.clone())
}

fn value_may_hoist(value: &HirExpr) -> bool {
    matches!(
        value.kind,
        HirExprKind::Str { .. } | HirExprKind::Call { .. } | HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. }
    )
}

fn assign_moves_ident(value: &HirExpr, name: &str) -> bool {
    matches!(&value.kind, HirExprKind::Ident { name: rhs, use_kind: UseKind::Move } if rhs == name)
}

/// A `val` stored in (or returned as) any `return` of the function, at any depth, is allocated in the result sink, so
/// it outlives the function with its owner. Declarations inside loop bodies are skipped.
fn result_hoists(body: &HirBlock, tcx: &TyCtx, out: &mut HashMap<NodeId, Sink>) {
    let mut returns = Vec::new();
    collect_returns(body, &mut returns);
    let mut used = Vec::new();
    let mut tails: Vec<&HirExpr> = Vec::new();
    for ret in &returns {
        value_tails(ret, &mut tails);
    }
    for ret in &tails {
        if let HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } = &ret.kind {
            collect_idents(fields, tcx, &mut used);
        }
    }
    let mut decls = Vec::new();
    collect_decls(body, &mut decls);
    for ret in &tails {
        // Only a construct: a call's result may own bytes (a `String?`) that this does not move.
        if let HirExprKind::Ident { name, .. } = &ret.kind {
            if decls.iter().any(|(n, v)| n == name && (matches!(v.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. } | HirExprKind::MakeClosure { .. }) || is_vec_factory_call(v))) {
                used.push(name.clone());
            }
        }
    }
    loop {
        let before = used.len();
        for (name, value) in &decls {
            if used.contains(name) {
                if let HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } = &value.kind {
                    collect_idents(fields, tcx, &mut used);
                }
            }
        }
        used.sort();
        used.dedup();
        if used.len() == before {
            break;
        }
    }
    mark_decls(body, &used, false, out);
}

/// A declaration inside a loop body is left alone: hoisting would allocate in the caller's arena on every iteration and
/// never free it.
fn mark_decls(block: &HirBlock, used: &[String], in_loop: bool, out: &mut HashMap<NodeId, Sink>) {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::VarDecl { name, value, .. } => {
                let constructs = matches!(value.kind, HirExprKind::ObjectConstruct { .. } | HirExprKind::EnumConstruct { .. } | HirExprKind::Call { .. } | HirExprKind::MakeClosure { .. });
                if constructs && !in_loop && used.contains(name) {
                    out.entry(value.id).or_insert(Sink::Result);
                }
            }
            HirStmt::Block(b) | HirStmt::MoveBlock { body: b, .. } => mark_decls(b, used, in_loop, out),
            HirStmt::For { body: b, .. } | HirStmt::While { body: b, .. } => mark_decls(b, used, true, out),
            HirStmt::Expr(e) => expr_blocks(e).into_iter().for_each(|b| mark_decls(b, used, in_loop, out)),
            _ => {}
        }
    }
}

/// `vecNew()` / `vecWithCapacity(n)`: a construct for `result_hoists` (the vector is allocated where it is declared).
fn is_vec_factory_call(e: &HirExpr) -> bool {
    matches!(&e.kind, HirExprKind::Call { callee, .. } if matches!(&callee.kind, HirExprKind::Ident { name, .. } if super::expr::is_vec_factory(name)))
}

pub(crate) fn nested_blocks(stmt: &HirStmt) -> Vec<&HirBlock> {
    match stmt {
        HirStmt::Block(b) | HirStmt::MoveBlock { body: b, .. } | HirStmt::For { body: b, .. } | HirStmt::While { body: b, .. } => vec![b],
        HirStmt::Expr(e) => expr_blocks(e),
        _ => vec![],
    }
}

pub(crate) fn expr_blocks(e: &crate::hir::HirExpr) -> Vec<&HirBlock> {
    match &e.kind {
        HirExprKind::If { then_block, else_block, .. } => std::iter::once(then_block).chain(else_block.iter()).collect(),
        HirExprKind::Match { arms, .. } => arms.iter().map(|arm| &arm.body).collect(),
        HirExprKind::PresenceMatch { some_block, none_block, .. } => std::iter::once(some_block).chain(none_block.iter()).collect(),
        _ => vec![],
    }
}

pub(crate) fn collect_returns<'a>(block: &'a HirBlock, out: &mut Vec<&'a crate::hir::HirExpr>) {
    for stmt in &block.stmts {
        if let HirStmt::Return { value: Some(v) } = stmt {
            out.push(v);
        }
        for inner in nested_blocks(stmt) {
            collect_returns(inner, out);
        }
    }
}

pub(crate) fn collect_decls<'a>(block: &'a HirBlock, out: &mut Vec<(String, &'a crate::hir::HirExpr)>) {
    for stmt in &block.stmts {
        if let HirStmt::VarDecl { name, value, .. } = stmt {
            out.push((name.clone(), value));
        }
        for inner in nested_blocks(stmt) {
            collect_decls(inner, out);
        }
    }
}

/// The expressions a returned value can actually be: itself, or for an `if`/`else` the tail
/// expression of each branch, recursively. `return if (f) { A(p) } else { A(q) }` stores `p` or `q`.
pub(crate) fn value_tails<'a>(expr: &'a crate::hir::HirExpr, out: &mut Vec<&'a crate::hir::HirExpr>) {
    match &expr.kind {
        HirExprKind::If { then_block, else_block, .. } => {
            for block in std::iter::once(then_block).chain(else_block.iter()) {
                if let Some(HirStmt::Expr(tail)) = block.stmts.last() {
                    value_tails(tail, out);
                }
            }
        }
        _ => out.push(expr),
    }
}

/// Names used directly as fields of a construct, looking through nested constructs.
pub(crate) fn collect_idents(fields: &[crate::hir::HirExpr], tcx: &TyCtx, out: &mut Vec<String>) {
    for field in fields {
        match &field.kind {
            HirExprKind::Ident { name, .. } => out.push(name.clone()),
            HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => {
                collect_idents(fields, tcx, out)
            }
            // `local.n` stored in the result still points into `local`'s arena.
            // Only when the field is itself an object handle: a scalar read copies the value.
            HirExprKind::ObjectField { receiver, .. } | HirExprKind::Field { receiver, .. }
                if tcx.holds_object(field.ty) =>
            {
                collect_idents(std::slice::from_ref(receiver.as_ref()), tcx, out)
            }
            _ => {}
        }
    }
}

