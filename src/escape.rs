//! Rejects `String` values whose bytes would outlive the arena that holds them.
//!
//! `place` is the same rule as codegen `alloc_sink`: `move`, `promote`, and `concat` land in
//! the consumer's sink when one is set, otherwise in the current region. A plain load keeps
//! the depth already stored on the binding.
//!
//! This stays a standalone HIR pass instead of a `region_walk` visitor: it needs a bottom-up
//! value depth per expression plus a top-down sink, while `RegionVisitor` callbacks return
//! `()`. It shares `peel_to_body` and `Ty::owns_result` with codegen, so block peeling and the
//! sink rule have one definition.

use std::collections::HashMap;

use crate::ast::BinOp;
use crate::builtins;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{
    peel_to_body, HirBlock, HirConditionalBinding, HirConditionalBindings, HirExpr, HirExprKind,
    HirFunction, HirStmt, UseKind,
};
use crate::span::Span;
use crate::ty::{TyCtx, TyId};

pub fn check_function(function: &HirFunction, tcx: &TyCtx, diagnostics: &mut Vec<Diagnostic>) {
    let params = function
        .params
        .iter()
        .map(|param| {
            let local = Local {
                decl_depth: 0,
                value_depth: 0,
                short_lived_object: false,
                in_loop: false,
            };
            (param.name.as_str(), local)
        })
        .collect();
    let mut escape = Escape {
        depth: 0,
        loop_depth: 0,
        scopes: vec![params],
        tcx,
        diagnostics,
    };
    escape.region(&function.body, false, None);
}

const LOOP_HINT: &str = "declare it before the loop and return it from inside, or build the returned value outside the loop";

fn reject(diagnostics: &mut Vec<Diagnostic>, message: &str, span: Option<Span>) {
    diagnostics.push(Diagnostic {
        phase: Phase::Ownership,
        severity: Severity::Error,
        message: message.to_owned(),
        span,
    });
}

#[derive(Clone, Copy)]
struct Local {
    decl_depth: usize,
    value_depth: usize,
    /// A `val`/`var` of this function holding a class or enum handle that is not hoisted into
    /// the result sink: it dies with its region.
    short_lived_object: bool,
    /// Declared inside a `for` / `while` body.
    in_loop: bool,
}

struct Escape<'h, 'd> {
    depth: usize,
    loop_depth: usize,
    scopes: Vec<HashMap<&'h str, Local>>,
    tcx: &'d TyCtx,
    diagnostics: &'d mut Vec<Diagnostic>,
}

impl<'h> Escape<'h, '_> {
    fn region(&mut self, body: &'h HirBlock, yields: bool, sink: Option<usize>) -> usize {
        self.open_scope();
        self.region_body(body, yields, sink)
    }

    /// `if val` / `when` region: binds the guard names first. Later sources are evaluated
    /// inside the region, as in codegen.
    fn presence_region(
        &mut self,
        body: &'h HirBlock,
        sink: Option<usize>,
        bindings: &'h HirConditionalBindings,
    ) -> usize {
        self.open_scope();
        self.bind_guard(&bindings.head);
        for binding in &bindings.tail {
            self.place(&binding.value, None);
            self.bind_guard(binding);
        }
        self.region_body(body, true, sink)
    }

    /// A local object stored in a returned construct would be freed with its region while the
    /// result still points at it. `hoist` allocates the simple cases in the result sink.
    fn check_returned_objects(&mut self, value: &'h HirExpr) {
        self.check_stored_objects(value, None);
    }

    /// With `owner_depth`, only locals declared in a deeper region than the owner dangle (an
    /// assignment into an outer `var`); without it, every short-lived local does (a `return`).
    fn check_stored_objects(&mut self, value: &'h HirExpr, owner_depth: Option<usize>) {
        let (HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. }) = &value.kind
        else {
            return;
        };
        let mut names = Vec::new();
        crate::hoist::collect_idents(fields, self.tcx, &mut names);
        for name in names {
            let Some(local) = self.lookup(&name).copied().filter(|local| {
                local.short_lived_object && owner_depth.is_none_or(|depth| local.decl_depth > depth)
            }) else {
                continue;
            };
            let message = if local.in_loop {
                format!("`{name}` is declared inside a loop, so it is freed on every iteration; {LOOP_HINT}")
            } else {
                format!(
                    "`{name}` is freed before the value that stores it, but that value would still point at it; \
                     build it directly in the constructor, or declare it at the top level of the function and return the value as its final statement"
                )
            };
            reject(self.diagnostics, &message, value.span);
        }
    }

    /// `return a` where `a` is an object declared in a loop body.
    fn check_returned_loop_local(&mut self, value: &'h HirExpr) -> bool {
        let HirExprKind::Ident { name, .. } = &value.kind else { return false };
        let Some(local) = self.lookup(name).copied() else { return false };
        if !(local.short_lived_object && local.in_loop) {
            return false;
        }
        reject(
            self.diagnostics,
            &format!("`{name}` is declared inside a loop, so it is freed on every iteration; {LOOP_HINT}"),
            value.span,
        );
        true
    }

    fn open_scope(&mut self) {
        self.depth += 1;
        self.scopes.push(HashMap::new());
    }

    /// Walks a region body whose scope is already open, then closes it.
    fn region_body(&mut self, body: &'h HirBlock, yields: bool, sink: Option<usize>) -> usize {
        let body = peel_to_body(body);
        let mut value = 0;
        if let Some((last, prefix)) = body.stmts.split_last() {
            for stmt in prefix {
                self.stmt(stmt);
            }
            match last {
                HirStmt::Expr(expr) if yields => {
                    value = self.place(expr, sink);
                    if sink.is_none()
                        && value == self.depth
                        && self.maybe_arena(expr.ty)
                        && !self.tcx.owns_result(expr.ty)
                    {
                        let note = self.param_note(expr.ty);
                        reject(
                            self.diagnostics,
                            &format!(
                                "a `String` moved inside an `if` branch cannot be its value: \
                                 the branch's arena is freed when the branch ends{note}"
                            ),
                            expr.span,
                        );
                    }
                }
                _ => self.stmt(last),
            }
        }
        self.scopes.pop();
        self.depth -= 1;
        if sink.is_some() {
            value
        } else {
            value.min(self.depth)
        }
    }

    fn stmt(&mut self, stmt: &'h HirStmt) {
        match stmt {
            HirStmt::Return { value: Some(value) } => {
                self.check_returned_objects(value);
                if self.check_returned_loop_local(value) {
                    return;
                }
                if self.tcx.is_ref(value.ty) {
                    reject(
                        self.diagnostics,
                        "returning a reference is not supported",
                        value.span,
                    );
                    return;
                }
                if !self.maybe_arena(value.ty) {
                    return;
                }
                if self.place(value, Some(0)) > 0 {
                    let note = self.param_note(value.ty);
                    reject(
                        self.diagnostics,
                        &format!(
                            "returning a `{value_ty}` whose bytes live in an inner region is not \
                             supported: that arena is freed before the value is returned{note}",
                            value_ty = if self.tcx.has_param(value.ty) {
                                "generic value".to_string()
                            } else {
                                self.tcx.display(value.ty).to_string()
                            }
                        ),
                        value.span,
                    );
                }
                self.check_return_string_form(value);
            }
            HirStmt::Return { value: None } => {}
            HirStmt::Block(body) | HirStmt::MoveBlock { body, .. } => {
                self.region(body, false, None);
            }
            HirStmt::VarDecl { name, value, ty, alloc_in_binding, .. } => {
                let hoisted = alloc_in_binding.as_deref() == Some(crate::hoist::RESULT_SINK);
                // A hoisted object lives in the caller's arena (depth 0), not in this region.
                let value_depth = if hoisted { 0 } else { self.place(value, None) };
                let local = Local {
                    decl_depth: self.depth,
                    value_depth,
                    short_lived_object: !hoisted && crate::hoist::holds_object(self.tcx, *ty),
                    in_loop: self.loop_depth > 0,
                };
                self.scopes
                    .last_mut()
                    .expect("regions open a scope")
                    .insert(name, local);
            }
            HirStmt::Assign { target, value } => {
                if let Some(receiver) = target.receiver() {
                    self.place(receiver, None);
                }
                let Some(name) = target.binding_name() else {
                    self.place(value, None);
                    return;
                };
                if let Some(index) = target.index() {
                    self.place(index, None);
                }
                let depth = self
                    .lookup(name)
                    .map(|local| local.decl_depth)
                    .unwrap_or(0);
                self.check_stored_objects(value, Some(depth));
                let value_depth = self.place(value, Some(depth));
                if target.index().is_none() {
                    if let Some(local) = self.lookup_mut(name) {
                        local.value_depth = value_depth.min(depth);
                    }
                }
                let outlives = value_depth > depth;
                if outlives && self.maybe_arena(value.ty) {
                    let note = self.param_note(value.ty);
                    reject(
                        self.diagnostics,
                        &format!(
                            "assigning a value whose bytes live in an inner region to `{name}` \
                             is not supported: that arena is freed before `{name}` goes out of scope{note}"
                        ),
                        value.span,
                    );
                }
            }
            HirStmt::Expr(expr) => {
                self.place(expr, None);
            }
            HirStmt::For { iter, body, .. } => {
                self.place(iter, None);
                self.loop_depth += 1;
                self.region(body, false, None);
                self.loop_depth -= 1;
            }
            HirStmt::While { cond, body } => {
                self.place(cond, None);
                self.loop_depth += 1;
                self.region(body, false, None);
                self.loop_depth -= 1;
            }
            HirStmt::Break { .. } | HirStmt::Continue { .. } => {}
        }
    }

    fn check_return_string_form(&mut self, expr: &'h HirExpr) {
        if self.tcx.is_string(expr.ty) && self.is_fresh_string(expr) {
            reject(
                self.diagnostics,
                "returning the result of `concat` or `stringFromUtf8` is not supported yet: \
                 returned `String` bytes must outlive the callee arena",
                expr.span,
            );
            return;
        }
        if self.is_moved_or_promoted_string(expr) {
            let note = self.param_note(expr.ty);
            reject(
                self.diagnostics,
                &format!(
                    "returning a moved or promoted `String` is not supported yet: use \
                     `return s` for parameters and locals, or return a string literal{note}"
                ),
                expr.span,
            );
            return;
        }
        match &expr.kind {
            HirExprKind::If {
                then_block,
                else_block,
                ..
            }
            | HirExprKind::PresenceMatch {
                some_block: then_block,
                none_block: else_block,
                ..
            } => {
                self.check_return_string_in_block(then_block);
                if let Some(else_block) = else_block {
                    self.check_return_string_in_block(else_block);
                }
            }
            _ => {}
        }
    }

    fn check_return_string_in_block(&mut self, block: &'h HirBlock) {
        let block = peel_to_body(block);
        if let Some(HirStmt::Expr(expr)) = block.stmts.last() {
            self.check_return_string_form(expr);
        }
    }

    /// A value whose bytes are allocated by an allocating builtin, possibly wrapped in
    /// `Some`, `!!`, `?:`, `if`, or `when` whose branch ends in one.
    fn is_fresh_string(&self, expr: &HirExpr) -> bool {
        match &expr.kind {
            HirExprKind::Call { callee, .. } => matches!(
                &callee.kind,
                HirExprKind::Ident { name, .. } if builtins::is_string_alloc(name)
            ),
            HirExprKind::Some(inner)
            | HirExprKind::Unary { expr: inner, .. } => self.is_fresh_string(inner),
            HirExprKind::Binary { op: BinOp::Elvis, lhs, rhs } => {
                self.is_fresh_string(lhs) || self.is_fresh_string(rhs)
            }
            HirExprKind::If { then_block, else_block, .. } => {
                self.block_ends_fresh(then_block)
                    || else_block.as_ref().is_some_and(|block| self.block_ends_fresh(block))
            }
            HirExprKind::PresenceMatch {
                some_block,
                none_block,
                ..
            } => {
                self.block_ends_fresh(some_block)
                    || none_block.as_ref().is_some_and(|block| self.block_ends_fresh(block))
            }
            _ => false,
        }
    }

    fn block_ends_fresh(&self, block: &HirBlock) -> bool {
        matches!(peel_to_body(block).stmts.last(), Some(HirStmt::Expr(expr)) if self.is_fresh_string(expr))
    }

    fn is_moved_or_promoted_string(&self, expr: &HirExpr) -> bool {
        matches!(
            &expr.kind,
            HirExprKind::Ident {
                use_kind: UseKind::Move | UseKind::Promote,
                ..
            } if self.tcx.is_string(expr.ty) || self.tcx.is_param(expr.ty)
        )
    }

    /// Appended to a `String`-worded rejection when `ty` involves an opaque type parameter.
    fn param_note(&self, ty: TyId) -> &'static str {
        if self.tcx.has_param(ty) {
            " (a type parameter is judged as its strictest instance, `String`)"
        } else {
            ""
        }
    }

    /// Whether a value of `ty` may live in an arena. An opaque `T` is judged as its most
    /// restrictive instantiation (`String`), so a generic body accepted here stays accepted for
    /// every instance (RFC 0002 section 4).
    fn maybe_arena(&self, ty: TyId) -> bool {
        self.tcx.uses_arena_storage(ty) || self.tcx.has_param(ty)
    }

    /// Depth of the arena that holds `expr`'s string bytes.
    /// `sink` is the consumer arena (`0` on return, the target's `decl_depth` on assign).
    fn place(&mut self, expr: &'h HirExpr, sink: Option<usize>) -> usize {
        match &expr.kind {
            HirExprKind::Int { .. }
            | HirExprKind::Float { .. }
            | HirExprKind::Bool { .. }
            | HirExprKind::Str { .. }
            | HirExprKind::None => 0,
            HirExprKind::ArrayLit { elements } => {
                let deepest = elements
                    .iter()
                    .map(|element| self.place(element, sink))
                    .max()
                    .unwrap_or(0);
                self.buffer_depth(sink).max(deepest)
            }
            HirExprKind::VecNew { cap, .. } => {
                if let Some(cap) = cap {
                    self.place(cap, None);
                }
                self.buffer_depth(sink)
            }
            HirExprKind::VecOp { receiver, args, .. } => {
                for arg in args {
                    self.place(arg, None);
                }
                if !self.maybe_arena(expr.ty) {
                    self.place(receiver, None);
                    return 0;
                }
                self.array_value_depth(receiver)
            }
            HirExprKind::Index { receiver, index, .. } => {
                self.place(index, None);
                if !self.maybe_arena(expr.ty) {
                    self.place(receiver, None);
                    return 0;
                }
                self.array_value_depth(receiver)
            }
            HirExprKind::Slice { receiver, lo, hi } => {
                let depth = self.place(receiver, None);
                self.place(lo, None);
                self.place(hi, None);
                depth
            }
            HirExprKind::Ident { name, use_kind } => {
                if self.maybe_arena(expr.ty)
                    && matches!(*use_kind, UseKind::Move | UseKind::Promote)
                {
                    match sink {
                        Some(0) => self.lookup(name).map_or(0, |local| local.value_depth),
                        Some(depth) => depth,
                        None => self.depth,
                    }
                } else {
                    self.lookup(name).map_or(0, |local| local.value_depth)
                }
            }
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.place(lhs, None);
                self.place(rhs, None);
                0
            }
            HirExprKind::Call { callee, args, .. } => self.place_call(expr, callee, args, sink),
            HirExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.place(cond, None);
                let then_depth = self.region(then_block, true, sink);
                let else_depth = else_block
                    .as_ref()
                    .map_or(0, |block| self.region(block, true, sink));
                then_depth.max(else_depth)
            }
            HirExprKind::Some(inner)
            | HirExprKind::RefCreate(inner)
            | HirExprKind::Unary { expr: inner, .. }
            | HirExprKind::Field {
                receiver: inner, ..
            } => self.place(inner, None),
            // A `T?` is `Option<T>`, but for escape it stays transparent as `Some(e)` always was:
            // codegen copies a returned value into the caller's sink arena at the return.
            HirExprKind::EnumConstruct { fields, .. } if self.tcx.is_nullable(expr.ty) => {
                fields.iter().map(|field| self.place(field, None)).max().unwrap_or(0)
            }
            HirExprKind::ObjectConstruct { fields, .. } | HirExprKind::EnumConstruct { fields, .. } => {
                for field in fields {
                    self.place(field, sink);
                }
                sink.unwrap_or(self.depth)
            }
            HirExprKind::ObjectField { receiver, .. } => self.place(receiver, None),
            HirExprKind::PresenceMatch {
                bindings,
                some_block,
                none_block,
                ..
            } => self.place_presence(expr, bindings, some_block, none_block.as_ref(), sink),
            // Not lowered yet (#53); the gate rejects it before codegen.
            HirExprKind::Match { .. } => sink.unwrap_or(self.depth),
        }
    }

    /// Where a fresh buffer (array literal, `Vec`) is allocated: the consumer's arena, or here.
    fn buffer_depth(&self, sink: Option<usize>) -> usize {
        match sink {
            Some(0) | None => self.depth,
            Some(depth) => depth,
        }
    }

    fn place_call(
        &mut self,
        expr: &'h HirExpr,
        callee: &'h HirExpr,
        args: &'h [HirExpr],
        sink: Option<usize>,
    ) -> usize {
        let concat = matches!(
            &callee.kind,
            HirExprKind::Ident { name, .. } if builtins::is_string_alloc(name)
        );
        self.place(callee, None);
        if self.tcx.is_string(expr.ty) && concat {
            for arg in args {
                self.place(arg, None);
            }
            sink.unwrap_or(self.depth)
        } else {
            let deepest = args.iter().map(|arg| self.place(arg, None)).fold(0, usize::max);
            if self.maybe_arena(expr.ty) {
                deepest
            } else {
                0
            }
        }
    }

    fn place_presence(
        &mut self,
        expr: &'h HirExpr,
        bindings: &'h HirConditionalBindings,
        some_block: &'h HirBlock,
        none_block: Option<&'h HirBlock>,
        sink: Option<usize>,
    ) -> usize {
        self.place(&bindings.head.value, None);
        if self.tcx.is_ref(expr.ty) {
            reject(
                self.diagnostics,
                "a presence guard reference cannot escape its scope",
                expr.span,
            );
        }
        // Both presence arms materialize owned results before releasing observations.
        let result_depth = sink.unwrap_or(self.depth);
        let some_depth = self.presence_region(some_block, Some(result_depth), bindings);
        let none_depth = none_block.map_or(0, |block| self.region(block, true, Some(result_depth)));
        if self.maybe_arena(expr.ty) {
            result_depth
        } else {
            some_depth.max(none_depth)
        }
    }

    fn bind_guard(&mut self, binding: &'h HirConditionalBinding) {
        self.scopes.last_mut().unwrap().insert(
            &binding.name.name,
            Local {
                decl_depth: self.depth,
                value_depth: self.depth,
                short_lived_object: false,
                in_loop: false,
            },
        );
    }

    fn array_value_depth(&mut self, receiver: &'h HirExpr) -> usize {
        if let HirExprKind::Ident { name, .. } = &receiver.kind {
            self.lookup(name)
                .map(|local| local.value_depth)
                .unwrap_or(0)
        } else {
            self.place(receiver, None)
        }
    }

    fn lookup(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|scope| scope.get(name))
    }

    fn lookup_mut(&mut self, name: &str) -> Option<&mut Local> {
        self.scopes
            .iter_mut()
            .rev()
            .find_map(|scope| scope.get_mut(name))
    }
}

#[cfg(test)]
mod tests {
    use crate::diag::Phase;
    use crate::hir::{HirBlock, HirExprKind, HirStmt};

    fn assert_ok(source: &str) {
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
    }

    fn assert_rejects(source: &str, needle: &str) {
        let result = crate::frontend::check(source);
        assert!(
            result.diagnostics.iter().any(|diagnostic| {
                diagnostic.phase == Phase::Ownership
                    && diagnostic.message.contains(needle)
                    && diagnostic.span.is_some()
            }),
            "missing `{needle}` diagnostic: {:?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_returning_string_from_inner_region() {
        assert_rejects(
            "fun mk(): String {\n    var s = \"esc\"\n    {\n        val x = move s\n        return x\n    }\n}\nfun main() { println(mk()) }\n",
            "inner region",
        );
    }

    #[test]
    fn rejects_returning_function_local_without_move() {
        assert_rejects(
            "fun mk(): String {\n    var s = \"esc\"\n    val t = move s\n    return t\n}\nfun main() { println(mk()) }\n",
            "inner region",
        );
    }

    /// `move` into an assign sink is legal from an inner block. A plain load of a string
    /// allocated in that block is not, so the RHS use-kind is cleared by hand.
    #[test]
    fn rejects_assigning_inner_move_to_outer_local() {
        let source = "fun main() {\n    var s = \"a\"\n    var src = \"b\"\n    var x = move src\n    s = move x\n    println(s)\n}\n";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let mut hir = result.hir.clone().unwrap();
        let stmts = &mut hir.functions[0].body.stmts;
        let assign = stmts.remove(3);
        let decl = stmts.remove(2);
        let HirStmt::Assign {
            target,
            mut value,
        } = assign
        else {
            panic!("expected assign");
        };
        if let HirExprKind::Ident { use_kind, .. } = &mut value.kind {
            *use_kind = crate::hir::UseKind::Local;
        }
        stmts.insert(
            2,
            HirStmt::Block(HirBlock {
                stmts: vec![decl, HirStmt::Assign { target, value }],
                ..HirBlock::default()
            }),
        );

        let mut diagnostics = Vec::new();
        for function in &hir.functions {
            crate::escape::check_function(function, &result.tcx, &mut diagnostics);
        }

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.phase == Phase::Ownership
                    && diagnostic
                        .message
                        .contains("that arena is freed before `s` goes out of scope")
                    && diagnostic.span.is_some()
            }),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn rejects_if_branch_yielding_moved_string() {
        assert_rejects(
            "fun main() {\n    var s = \"a\"\n    val c = 1\n    val t = if (c > 0) { move s } else { \"b\" }\n    println(t)\n}\n",
            "moved inside an `if` branch",
        );
    }

    #[test]
    fn rejects_return_if_branch_yielding_concat() {
        let result = crate::frontend::check(
            "fun f(c: i32): String {\n    return if (c > 0) { concat(\"a\", \"b\") } else { \"x\" }\n}\nfun main() {}\n",
        );
        assert!(
            result.diagnostics.iter().any(|d| {
                d.phase == Phase::Ownership
                    && d.message.contains("concat")
                    && d.message.contains("not supported")
            }),
            "{:?}",
            result.diagnostics
        );
    }

    #[test]
    fn rejects_return_when_branch_yielding_string_from_utf8() {
        assert_rejects(
            "class Box { value: i32 }\n\
             fun make(): String {\n\
                 val bytes: [u8; 1] = [65]\n\
                 val reference: Ref<Box> = Box(1)\n\
                 return when reference {\n\
                     Some(value) => { stringFromUtf8(&bytes, 0, 1) ?: \"x\" }\n\
                     None => { \"x\" }\n\
                 }\n\
             }\n\
             fun main() {}\n",
            "stringFromUtf8",
        );
    }

    #[test]
    fn allows_return_nullable_none_and_elvis() {
        assert_ok(
            r#"
fun guest(name: String?): String {
    return name ?: "Guest"
}
fun empty(): String? {
    return None
}
fun main() {
    println(guest(None))
    val s: String? = None
    println(s ?: "x")
}
"#,
        );
    }

    #[test]
    fn allows_return_some_and_not_null_assert() {
        assert_ok(
            r#"
fun wrapped(): String {
    val s: String? = Some("ok")
    return s!!
}
fun main() {
    {
        val inner: String? = Some("block")
        println(inner!!)
    }
    println(wrapped())
}
"#,
        );
    }

    #[test]
    fn allows_return_move_concat_and_promote() {
        assert_ok(
            r#"
fun lit(): String { return "x" }
fun id(s: String): String { return s }
fun mk(): String {
    var s = "ab"
    return s
}
fun main() {
    var outer = "a"
    {
        var inner = "b"
        outer = move inner
    }
    var left = "L"
    var right = "R"
    outer = concat(left, right)
    {
        var held = "p"
        outer = promote held
    }
    val t = move outer
    val u = id(t)
    val c = 1
    val v = if (c > 0) { u } else { lit() }
    println(v)
}
"#,
        );
    }

    #[test]
    fn rejects_class_handles_that_outlive_guarded_objects() {
        for guard in ["if val (a = r, b = r)", "if val a = r"] {
            for body in [
                format!("val out = {guard} {{ a.child }} else {{ &fallback }}"),
                format!("val out = Wrap({guard} {{ a.child }} else {{ fallback }})"),
                format!("val out = {guard} {{ Wrap(a.child) }} else {{ Wrap(fallback) }}"),
                format!("val out = Wrap(fallback)\n {guard} {{ out.child = a.child }}"),
                format!("{guard} {{ a.child = a.child }}"),
                format!("{guard} {{ consume(a.child) }}"),
                format!("{guard} {{ val child = a.child\n val owned = move child }}"),
                format!("val out = Wrap(fallback)\n return {guard} {{ out.child = a.child\n 1 }} else {{ 0 }}"),
            ] {
                let source = format!("class Leaf {{ value: i32 }}\nclass Node {{ child: Leaf }}\nclass Wrap {{ child: Leaf }}\nfun consume(node: Leaf) {{}}\nfun use(r: Ref<Node>, fallback: Leaf): i32 {{\n {body}\n return 0\n}}");
                let checked = crate::frontend::check(&source);
                assert!(
                    checked.diagnostics.iter().any(|d| {
                        d.message.contains("presence guard reference")
                            || d.message.contains("has type &Leaf")
                            || d.message.contains("it is a borrow")
                            || d.message.contains("is borrow &Leaf")
                    }),
                    "{source}\n{:?}",
                    checked.diagnostics
                );
            }
        }
    }

    #[test]
    fn allows_class_fields_within_guard_and_fresh_owned_results() {
        let checked = crate::frontend::check("class Leaf { value: i32 }\nclass Node { child: Leaf }\nfun use(r: Ref<Node>) {\n val out = if val (a = r, b = r) {\n val child: &Leaf = a.child\n println(child.value)\n Leaf(child.value)\n } else { Leaf(0) }\n println(out.value)\n}");
        assert!(checked.is_ok(), "{:?}", checked.diagnostics);
    }
}
