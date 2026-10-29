//! Assigns an [`AstId`] to every AST node that carries a typeck fact, in a deterministic
//! pre-order walk, and returns the `(Span, AstId)` index the LSP resolves positions with.

use crate::ast::{AssignTarget, Block, Closure, ConditionalBindings, Expr, Pattern, Program, Stmt};
use crate::span::{AstId, Span, SpannedName};

/// Numbers `program` in place. The index lists every numbered node with its source span.
pub fn number_nodes(program: &mut Program) -> Vec<(Span, AstId)> {
    number_nodes_from(program, 0)
}

/// Like [`number_nodes`], with ids starting at `first`. The prelude is numbered this way, after the
/// user's program, and its index is dropped.
pub fn number_nodes_from(program: &mut Program, first: u32) -> Vec<(Span, AstId)> {
    let mut n = Numberer { next: first, ..Numberer::default() };
    program.constants.iter_mut().for_each(|c| c.number(&mut n));
    program.classes.iter_mut().for_each(|c| c.number(&mut n));
    program.enums.iter_mut().for_each(|e| e.number(&mut n));
    program.functions.iter_mut().for_each(|f| f.number(&mut n));
    program.traits.iter_mut().for_each(|t| t.number(&mut n));
    program.impls.iter_mut().for_each(|i| i.number(&mut n));
    n.index
}

/// A declaration that numbers its own names; the walk above is then one line per item kind.
trait Numbered {
    fn number(&mut self, n: &mut Numberer);
}

impl Numbered for crate::ast::ConstDecl {
    fn number(&mut self, n: &mut Numberer) {
        n.name(&mut self.name);
        n.expr(&mut self.value);
    }
}

impl Numbered for crate::ast::Class {
    fn number(&mut self, n: &mut Numberer) {
        n.generics(&mut self.generics);
        self.fields.iter_mut().for_each(|f| n.name(&mut f.name));
        self.methods.iter_mut().for_each(|m| m.number(n));
    }
}

impl Numbered for crate::ast::Enum {
    fn number(&mut self, n: &mut Numberer) {
        n.name(&mut self.name);
        n.generics(&mut self.generics);
        for variant in &mut self.variants {
            n.name(&mut variant.name);
            variant.fields.iter_mut().for_each(|f| n.name(&mut f.name));
        }
    }
}

impl Numbered for crate::ast::Function {
    fn number(&mut self, n: &mut Numberer) {
        self.id = n.fresh(self.name_span);
        n.generics(&mut self.generics);
        n.params(&mut self.params);
        n.block(&mut self.body);
    }
}

impl Numbered for crate::ast::Trait {
    fn number(&mut self, n: &mut Numberer) {
        n.name(&mut self.name);
        n.generics(&mut self.generics);
        for m in &mut self.methods {
            n.name(&mut m.name);
            n.generics(&mut m.generics);
            n.params(&mut m.params);
        }
    }
}

impl Numbered for crate::ast::Impl {
    fn number(&mut self, n: &mut Numberer) {
        n.generics(&mut self.generics);
        n.name(&mut self.trait_name);
        self.methods.iter_mut().for_each(|m| m.number(n));
    }
}

#[derive(Default)]
struct Numberer {
    next: u32,
    index: Vec<(Span, AstId)>,
}

impl Numberer {
    fn fresh(&mut self, span: Span) -> AstId {
        let id = AstId(self.next);
        self.next += 1;
        self.index.push((span, id));
        id
    }

    fn name(&mut self, name: &mut SpannedName) {
        name.id = self.fresh(name.span);
    }

    fn names(&mut self, names: &mut [SpannedName]) {
        names.iter_mut().for_each(|n| self.name(n));
    }

    fn generics(&mut self, generics: &mut [crate::ast::TypeParam]) {
        generics.iter_mut().for_each(|g| self.name(&mut g.name));
    }

    fn params(&mut self, params: &mut [crate::ast::Param]) {
        params.iter_mut().for_each(|p| self.name(&mut p.name));
    }

    fn block(&mut self, block: &mut Block) {
        block.stmts.iter_mut().for_each(|s| self.stmt(s));
    }

    fn stmt(&mut self, stmt: &mut Stmt) {
        match stmt {
            Stmt::Block(b) => self.block(b),
            Stmt::VarDecl { name_span, id, value, .. } => {
                *id = self.fresh(*name_span);
                self.expr(value);
            }
            Stmt::Assign { target, value } => {
                match target {
                    AssignTarget::Name { name_span, id, .. } => *id = self.fresh(*name_span),
                    AssignTarget::Index { name_span, id, index, .. } => {
                        *id = self.fresh(*name_span);
                        self.expr(index);
                    }
                    AssignTarget::Field { receiver, span, id, .. } => {
                        *id = self.fresh(*span);
                        self.expr(receiver);
                    }
                }
                self.expr(value);
            }
            Stmt::For { name, iter, body } => {
                self.name(name);
                self.expr(iter);
                self.block(body);
            }
            Stmt::While { cond, body } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::MoveBlock { captures, body } => {
                if let Some(caps) = captures {
                    self.names(caps);
                }
                self.block(body);
            }
            Stmt::Return(value) => value.iter_mut().for_each(|v| self.expr(v)),
            Stmt::Expr(e) => self.expr(e),
            Stmt::Break { .. } | Stmt::Continue { .. } => {}
        }
    }

    fn bindings(&mut self, bindings: &mut ConditionalBindings) {
        self.name(&mut bindings.head.name);
        self.expr(&mut bindings.head.value);
        for b in &mut bindings.tail {
            self.name(&mut b.name);
            self.expr(&mut b.value);
        }
    }

    fn pattern(&mut self, pat: &mut Pattern) {
        match pat {
            Pattern::Binding(name) => self.name(name),
            Pattern::Variant { enum_name, name, args, .. } => {
                enum_name.iter_mut().for_each(|e| self.name(e));
                self.name(name);
                args.iter_mut().for_each(|a| self.pattern(a));
            }
            Pattern::Some { inner, .. } => self.pattern(inner),
            Pattern::Wildcard { .. } | Pattern::Literal { .. } | Pattern::None { .. } => {}
        }
    }

    fn closure(&mut self, closure: &mut Closure) {
        self.names(&mut closure.params);
        if let Some(caps) = &mut closure.captures {
            self.names(caps);
        }
        self.block(&mut closure.body);
    }

    fn expr(&mut self, expr: &mut Expr) {
        match expr {
            Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Str(_) => {}
            Expr::ArrayLit { elements, .. } => elements.iter_mut().for_each(|e| self.expr(e)),
            Expr::Index { receiver, index, .. } => {
                self.expr(receiver);
                self.expr(index);
            }
            Expr::Slice { receiver, lo, hi, .. } => {
                self.expr(receiver);
                self.expr(lo);
                self.expr(hi);
            }
            Expr::VecNew { cap, .. } => cap.iter_mut().for_each(|c| self.expr(c)),
            Expr::Ident { span, id, .. } | Expr::Move { span, id, .. } | Expr::Promote { span, id, .. } => {
                *id = self.fresh(*span);
            }
            Expr::Some { expr, span, id } => {
                *id = self.fresh(*span);
                self.expr(expr);
            }
            Expr::None { span, id } => *id = self.fresh(*span),
            Expr::Unary { expr, .. } => self.expr(expr),
            Expr::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            Expr::Field { receiver, span, id, .. } => {
                *id = self.fresh(*span);
                self.expr(receiver);
            }
            Expr::Call { callee, args, trailing, .. } => {
                self.expr(callee);
                args.iter_mut().for_each(|a| self.expr(a));
                trailing.iter_mut().for_each(|c| self.closure(c));
            }
            Expr::If { cond, then_block, else_block } => {
                self.expr(cond);
                self.block(then_block);
                else_block.iter_mut().for_each(|b| self.block(b));
            }
            Expr::IfVal { bindings, then_block, else_block } => {
                self.bindings(bindings);
                self.block(then_block);
                else_block.iter_mut().for_each(|b| self.block(b));
            }
            Expr::Match { scrutinee, arms, .. } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.pattern(&mut arm.pat);
                    self.block(&mut arm.body);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::parse_indexed;
    use std::collections::HashSet;

    #[test]
    fn ids_are_unique_dense_and_indexed_over_the_corpus() {
        fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    collect(&path, out);
                } else if path.extension().is_some_and(|e| e == "bork") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        collect(&std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("programs"), &mut files);
        let mut total = 0;
        for file in files {
            let Ok((program, index)) = parse_indexed(&std::fs::read_to_string(&file).unwrap()) else {
                continue;
            };
            // Every `id: AstId(n)` in the tree, read from Debug so this does not share the walker.
            let text = format!("{program:?}");
            let mut ids: Vec<u32> = text
                .split("id: AstId(")
                .skip(1)
                .map(|rest| rest.split(')').next().unwrap().parse().unwrap())
                .collect();
            assert!(ids.iter().all(|id| *id != u32::MAX), "{}: unnumbered node", file.display());
            ids.sort_unstable();
            assert_eq!(ids, (0..ids.len() as u32).collect::<Vec<_>>(), "{}: ids not dense/unique", file.display());
            assert_eq!(index.len(), ids.len(), "{}: index misses nodes", file.display());
            assert_eq!(index.iter().map(|(_, id)| id).collect::<HashSet<_>>().len(), ids.len());
            total += ids.len();
        }
        assert!(total > 1000, "only {total} nodes checked");
    }
}
