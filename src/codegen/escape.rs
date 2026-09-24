//! Rejects `String` values whose bytes would outlive the arena that holds them.
//!
//! `move` of a String copies its bytes into the innermost open arena, and every arena pops
//! when its region ends. A String therefore carries the depth of the region it was copied in
//! (`0` for `.rodata` literals and caller-owned parameters); it must not reach a consumer
//! outside that region. Region depths mirror `RegionEmitter`, including its block peeling.

use std::collections::HashMap;

use crate::diag::Diagnostic;
use crate::hir::{HirBlock, HirExpr, HirExprKind, HirFunction, HirStmt, UseKind};

use super::gate::reject;
use super::regions::peel_blocks;

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
    escape.region(&function.body, false);
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
    fn region(&mut self, body: &'h HirBlock, yields: bool) -> usize {
        let (body, _) = peel_blocks(body);
        self.depth += 1;
        self.scopes.push(HashMap::new());
        let mut value = 0;
        if let Some((last, prefix)) = body.stmts.split_last() {
            for stmt in prefix {
                self.stmt(stmt);
            }
            match last {
                HirStmt::Expr(expr) if yields => {
                    value = self.expr(expr);
                    if value == self.depth && expr.ty.is_string() {
                        reject(
                            self.diagnostics,
                            "a `String` moved inside an `if` branch cannot be its value in \
                             codegen: the branch's arena is freed when the branch ends",
                            expr.span,
                        );
                    }
                }
                _ => self.stmt(last),
            }
        }
        self.scopes.pop();
        self.depth -= 1;
        value.min(self.depth)
    }

    fn stmt(&mut self, stmt: &'h HirStmt) {
        match stmt {
            HirStmt::Return { value: Some(value) } => {
                if self.expr(value) > 0 && value.ty.is_string() {
                    reject(
                        self.diagnostics,
                        "returning a moved `String` is not supported by codegen: its bytes live \
                         in this function's arena, which is freed on return",
                        value.span,
                    );
                }
            }
            HirStmt::Return { value: None } => {}
            HirStmt::Block(body) | HirStmt::MoveBlock { body, .. } => {
                self.region(body, false);
            }
            HirStmt::VarDecl { name, value, .. } => {
                let value_depth = self.expr(value);
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
                let value_depth = self.expr(value);
                let Some(local) = self.lookup_mut(name) else {
                    return;
                };
                let outlives = value_depth > local.decl_depth;
                local.value_depth = value_depth.min(local.decl_depth);
                if outlives && value.ty.is_string() {
                    reject(
                        self.diagnostics,
                        &format!(
                            "assigning a `String` moved in an inner region to `{name}` is not \
                             supported by codegen: its arena is freed before `{name}` goes out \
                             of scope"
                        ),
                        value.span,
                    );
                }
            }
            HirStmt::Expr(expr) => {
                self.expr(expr);
            }
            HirStmt::For { iter, body, .. } => {
                self.expr(iter);
                self.region(body, false);
            }
        }
    }

    fn expr(&mut self, expr: &'h HirExpr) -> usize {
        match &expr.kind {
            HirExprKind::Int { .. } | HirExprKind::Str { .. } | HirExprKind::None => 0,
            HirExprKind::Ident { name, use_kind } => {
                if *use_kind == UseKind::Move && expr.ty.is_string() {
                    self.depth
                } else {
                    self.lookup(name).map_or(0, |local| local.value_depth)
                }
            }
            HirExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
                0
            }
            // A callee can only return literals or its own parameters, so a String result
            // lives no deeper than the String arguments it was given.
            HirExprKind::Call { callee, args, .. } => {
                self.expr(callee);
                let deepest = args.iter().map(|arg| self.expr(arg)).fold(0, usize::max);
                if expr.ty.is_string() {
                    deepest
                } else {
                    0
                }
            }
            HirExprKind::If {
                cond,
                then_block,
                else_block,
            } => {
                self.expr(cond);
                let then_depth = self.region(then_block, true);
                let else_depth = else_block
                    .as_ref()
                    .map_or(0, |block| self.region(block, true));
                then_depth.max(else_depth)
            }
            HirExprKind::Some(inner)
            | HirExprKind::Unary { expr: inner, .. }
            | HirExprKind::Field {
                receiver: inner, ..
            } => self.expr(inner),
        }
    }



    fn expr_sink(&mut self, expr: &'h HirExpr, target: usize) -> usize {
        match &expr.kind {
            HirExprKind::Str { .. } => target,
            HirExprKind::Call {
                callee,
                args,
                ..
            } if expr.ty.is_string() => {
                if matches!(
                    &callee.kind,
                    HirExprKind::Ident { name, .. } if crate::builtins::is_concat(name)
                ) {
                    for arg in args {
                        self.expr(arg);
                    }
                    return target;
                }
                self.expr(expr)
            }
            HirExprKind::Ident { use_kind, .. } if expr.ty.is_string() => {
                if matches!(*use_kind, UseKind::Move | UseKind::Promote) {
                    target
                } else {
                    self.lookup(
                        match &expr.kind {
                            HirExprKind::Ident { name, .. } => name,
                            _ => return target,
                        },
                    )
                        .map_or(0, |local| local.value_depth)
                }
            }
            _ => self.expr(expr),
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
    use crate::diag::{Diagnostic, Phase};
    use crate::hir::{HirBlock, HirStmt};

    fn gate_of(source: &str) -> Vec<Diagnostic> {
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        crate::codegen::gate(result.hir.as_ref().unwrap())
    }

    fn assert_rejects(source: &str, needle: &str) {
        let diagnostics = gate_of(source);
        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.phase == Phase::Codegen
                    && diagnostic.message.contains(needle)
                    && diagnostic.span.is_some()
            }),
            "missing `{needle}` diagnostic: {diagnostics:?}"
        );
    }

    #[test]
    fn rejects_returning_moved_string() {
        assert_rejects(
            "fun mk(): String {\n    var s = \"esc\"\n    return move s\n}\nfun main() { println(mk()) }\n",
            "returning a moved `String`",
        );
    }

    #[test]
    fn rejects_returning_local_that_holds_moved_string() {
        assert_rejects(
            "fun mk(): String {\n    var s = \"esc\"\n    val t = move s\n    return t\n}\nfun main() { println(mk()) }\n",
            "returning a moved `String`",
        );
    }

    /// Sema rejects this shape in source, so the assignment is wrapped in a block by hand.
    #[test]
    fn rejects_assigning_inner_move_to_outer_local() {
        let source = "fun main() {\n    var s = \"a\"\n    var x = \"b\"\n    s = move x\n    println(s)\n}\n";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let mut hir = result.hir.unwrap();
        let stmts = &mut hir.functions[0].body.stmts;
        let assign = stmts.remove(2);
        assert!(matches!(assign, HirStmt::Assign { .. }));
        stmts.insert(
            2,
            HirStmt::Block(HirBlock {
                stmts: vec![assign],
            }),
        );

        let diagnostics = crate::codegen::gate(&hir);

        assert!(
            diagnostics.iter().any(|diagnostic| {
                diagnostic.phase == Phase::Codegen
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
    fn allows_literals_params_and_same_region_moves() {
        let source = r#"
fun lit(): String { return "x" }
fun id(s: String): String { return s }
fun main() {
    var s = "ab"
    val t = move s
    val u = id(t)
    val c = 1
    val v = if (c > 0) { u } else { lit() }
    println(v)
}
"#;
        assert_eq!(gate_of(source), vec![]);
    }
}
