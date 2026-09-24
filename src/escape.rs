//! Rejects `String` values whose bytes would outlive the arena that holds them.
//!
//! `place` is the same rule as codegen `alloc_sink`: `move`, `promote`, and `concat` land in
//! the consumer's sink when one is set, otherwise in the current region. A plain load keeps
//! the depth already stored on the binding. Region depths mirror `RegionEmitter`, including
//! block peeling.

use std::collections::HashMap;

use crate::builtins::{self, Builtin};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, UseKind};
use crate::span::Span;

pub fn check_function(function: &HirFunction, diagnostics: &mut Vec<Diagnostic>) {
    let params = function
        .params
        .iter()
        .map(|param| {
            let local = Local {
                decl_depth: 0,
                value_depth: 0,
            };
            (param.name.as_str(), local)
        })
        .collect();
    let mut escape = Escape {
        depth: 0,
        scopes: vec![params],
        diagnostics,
    };
    escape.region(&function.body, false, None);
}

fn peel_blocks(block: &HirBlock) -> &HirBlock {
    let mut current = block;
    while let [HirStmt::Block(inner)] = current.stmts.as_slice() {
        current = inner;
    }
    current
}

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
}

struct Escape<'h, 'd> {
    depth: usize,
    scopes: Vec<HashMap<&'h str, Local>>,
    diagnostics: &'d mut Vec<Diagnostic>,
}

impl<'h> Escape<'h, '_> {
    fn region(&mut self, body: &'h HirBlock, yields: bool, sink: Option<usize>) -> usize {
        let body = peel_blocks(body);
        self.depth += 1;
        self.scopes.push(HashMap::new());
        let mut value = 0;
        if let Some((last, prefix)) = body.stmts.split_last() {
            for stmt in prefix {
                self.stmt(stmt);
            }
            match last {
                HirStmt::Expr(expr) if yields => {
                    value = self.place(expr, sink);
                    if sink.is_none() && value == self.depth && expr.ty.is_string() {
                        reject(
                            self.diagnostics,
                            "a `String` moved inside an `if` branch cannot be its value: \
                             the branch's arena is freed when the branch ends",
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
                if !value.ty.is_string() {
                    return;
                }
                if self.place(value, Some(0)) > 0 {
                    reject(
                        self.diagnostics,
                        "returning a `String` whose bytes live in an inner region is not \
                         supported: that arena is freed before the value is returned",
                        value.span,
                    );
                }
                self.check_return_string_form(value);
            }
            HirStmt::Return { value: None } => {}
            HirStmt::Block(body) | HirStmt::MoveBlock { body, .. } => {
                self.region(body, false, None);
            }
            HirStmt::VarDecl { name, value, .. } => {
                let value_depth = self.place(value, None);
                let local = Local {
                    decl_depth: self.depth,
                    value_depth,
                };
                self.scopes
                    .last_mut()
                    .expect("regions open a scope")
                    .insert(name, local);
            }
            HirStmt::Assign { name, value } => {
                let target = self
                    .lookup(name)
                    .map(|local| local.decl_depth)
                    .unwrap_or(0);
                let value_depth = self.place(value, Some(target));
                let Some(local) = self.lookup_mut(name) else {
                    return;
                };
                let outlives = value_depth > target;
                local.value_depth = value_depth.min(target);
                if outlives && value.ty.is_string() {
                    reject(
                        self.diagnostics,
                        &format!(
                            "assigning a `String` moved in an inner region to `{name}` is not \
                             supported: its arena is freed before `{name}` goes out of scope"
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
                self.region(body, false, None);
            }
        }
    }

    fn check_return_string_form(&mut self, expr: &'h HirExpr) {
        if expr.ty.is_string() && Self::is_concat_call(expr) {
            reject(
                self.diagnostics,
                "returning the result of `concat` is not supported yet: returned \
                 `String` bytes must outlive the callee arena",
                expr.span,
            );
            return;
        }
        if Self::is_moved_or_promoted_string(expr) {
            reject(
                self.diagnostics,
                "returning a moved or promoted `String` is not supported yet: use \
                 `return s` for parameters and locals, or return a string literal",
                expr.span,
            );
            return;
        }
        if let HirExprKind::If {
            then_block,
            else_block,
            ..
        } = &expr.kind
        {
            self.check_return_string_in_block(then_block);
            if let Some(else_block) = else_block {
                self.check_return_string_in_block(else_block);
            }
        }
    }

    fn check_return_string_in_block(&mut self, block: &'h HirBlock) {
        let block = peel_blocks(block);
        if let Some(HirStmt::Expr(expr)) = block.stmts.last() {
            self.check_return_string_form(expr);
        }
    }

    fn is_concat_call(expr: &HirExpr) -> bool {
        matches!(
            &expr.kind,
            HirExprKind::Call { callee, .. } if matches!(
                &callee.kind,
                HirExprKind::Ident { name, .. }
                    if builtins::resolve(name) == Some(Builtin::Concat)
            )
        )
    }

    fn is_moved_or_promoted_string(expr: &HirExpr) -> bool {
        matches!(
            &expr.kind,
            HirExprKind::Ident {
                use_kind: UseKind::Move | UseKind::Promote,
                ..
            } if expr.ty.is_string()
        )
    }

    /// Depth of the arena that holds `expr`'s string bytes.
    /// `sink` is the consumer arena (`0` on return, the target's `decl_depth` on assign).
    fn place(&mut self, expr: &'h HirExpr, sink: Option<usize>) -> usize {
        match &expr.kind {
            HirExprKind::Int { .. } | HirExprKind::Str { .. } | HirExprKind::None => 0,
            HirExprKind::Ident { name, use_kind } => {
                if expr.ty.is_string() && matches!(*use_kind, UseKind::Move | UseKind::Promote) {
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
            HirExprKind::Call { callee, args, .. } => {
                let concat = matches!(
                    &callee.kind,
                    HirExprKind::Ident { name, .. } if builtins::resolve(name) == Some(Builtin::Concat)
                );
                self.place(callee, None);
                if expr.ty.is_string() && concat {
                    for arg in args {
                        self.place(arg, None);
                    }
                    sink.unwrap_or(self.depth)
                } else {
                    let deepest = args.iter().map(|arg| self.place(arg, None)).fold(0, usize::max);
                    if expr.ty.is_string() {
                        deepest
                    } else {
                        0
                    }
                }
            }
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
            | HirExprKind::Unary { expr: inner, .. }
            | HirExprKind::Field {
                receiver: inner, ..
            } => self.place(inner, None),
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
        let mut hir = result.hir.unwrap();
        let stmts = &mut hir.functions[0].body.stmts;
        let assign = stmts.remove(3);
        let decl = stmts.remove(2);
        let HirStmt::Assign { name, mut value } = assign else {
            panic!("expected assign");
        };
        if let HirExprKind::Ident { use_kind, .. } = &mut value.kind {
            *use_kind = crate::hir::UseKind::Local;
        }
        stmts.insert(
            2,
            HirStmt::Block(HirBlock {
                stmts: vec![decl, HirStmt::Assign { name, value }],
            }),
        );

        let mut diagnostics = Vec::new();
        for function in &hir.functions {
            crate::escape::check_function(function, &mut diagnostics);
        }

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.phase == Phase::Ownership
                    && diagnostic
                        .message
                        .contains("moved in an inner region to `s`")
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
}
