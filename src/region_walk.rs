//! Single HIR walk that visits sema arena children in source order.
//!
//! Shared by codegen scheduling (`codegen::regions`), push-flag stamping, and LLVM emission.
//! Sema builds the tree from the AST (`sema::peel_blocks`); HIR bodies use
//! `hir::peel_blocks` with the same rule — keep those peel implementations aligned.

use crate::ast::BinOp;
use crate::hir::{peel_blocks, HirBlock, HirExpr, HirExprKind, HirFunction, HirProgram, HirStmt, Ty};
use crate::sema::ArenaNode;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionSite {
    Block,
    MoveBlock,
    For,
    While,
    IfThen,
    IfElse,
    Closure,
}

impl RegionSite {
    pub fn matches(self, label: &str) -> bool {
        match self {
            RegionSite::Block => label == "Block",
            RegionSite::MoveBlock => label == "MoveBlock (move)",
            RegionSite::For => label.starts_with("ForLoop ("),
            RegionSite::While => label == "WhileLoop",
            RegionSite::IfThen => label == "IfThen",
            RegionSite::IfElse => label == "IfElse",
            RegionSite::Closure => label.starts_with("Closure"),
        }
    }
}

/// Whether codegen should `arena_push` for a report child with this label and peeled HIR body.
pub fn codegen_push_for_region(label: &str, peeled_body: &HirBlock) -> bool {
    if label.starts_with("Closure") {
        return false;
    }
    block_may_allocate_sink(peeled_body)
}

fn block_may_allocate_sink(block: &HirBlock) -> bool {
    block.stmts.iter().any(stmt_may_allocate_sink)
}

fn stmt_may_allocate_sink(stmt: &HirStmt) -> bool {
    match stmt {
        HirStmt::VarDecl {
            ty,
            value,
            alloc_in_binding,
            ..
        } => alloc_in_binding.is_none() && ty.uses_arena_storage() && expr_may_allocate_sink(value),
        HirStmt::Assign { value, .. } => expr_may_allocate_sink(value),
        HirStmt::Return { value } => value.as_ref().is_some_and(expr_may_allocate_sink),
        HirStmt::Expr(value) => expr_may_allocate_sink(value),
        HirStmt::Block(body) | HirStmt::MoveBlock { body, .. } => block_may_allocate_sink(body),
        HirStmt::For { body, .. } | HirStmt::While { body, .. } => block_may_allocate_sink(body),
        HirStmt::Break { .. } | HirStmt::Continue { .. } => false,
    }
}

fn expr_may_allocate_sink(expr: &HirExpr) -> bool {
    match &expr.kind {
        HirExprKind::Bool { .. }
        | HirExprKind::Int { .. }
        | HirExprKind::Float { .. } => false,
        HirExprKind::Str { .. } | HirExprKind::ArrayLit { .. } => true,
        HirExprKind::Ident { use_kind, .. } => {
            matches!(*use_kind, crate::hir::UseKind::Move | crate::hir::UseKind::Promote)
                && expr.ty.uses_arena_storage()
        }
        HirExprKind::Call { callee, args, .. } => {
            let HirExprKind::Ident { name, .. } = &callee.kind else {
                return args.iter().any(expr_may_allocate_sink);
            };
            crate::builtins::is_concat(name) || args.iter().any(expr_may_allocate_sink)
        }
        HirExprKind::Binary { op, rhs, .. } if *op == crate::ast::BinOp::Elvis => {
            expr_may_allocate_sink(rhs)
        }
        HirExprKind::If {
            then_block,
            else_block,
            ..
        } => {
            block_may_allocate_sink(then_block)
                || else_block.as_ref().is_some_and(block_may_allocate_sink)
        }
        HirExprKind::Unary { expr, .. } => expr_may_allocate_sink(expr),
        HirExprKind::Binary { lhs, rhs, .. } => {
            expr_may_allocate_sink(lhs) || expr_may_allocate_sink(rhs)
        }
        HirExprKind::Index { receiver, .. } | HirExprKind::Slice { receiver, .. } => {
            expr_may_allocate_sink(receiver)
        }
        HirExprKind::Field { receiver, .. } => expr_may_allocate_sink(receiver),
        HirExprKind::Some(inner) => expr_may_allocate_sink(inner),
        HirExprKind::None => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkError {
    message: String,
    diagnostic: Option<crate::diag::Diagnostic>,
}

impl WalkError {
    pub fn message(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            diagnostic: None,
        }
    }

    pub fn from_diagnostic(diag: crate::diag::Diagnostic) -> Self {
        Self {
            message: diag.message.clone(),
            diagnostic: Some(diag),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.message
    }

    pub fn diagnostic(&self) -> Option<&crate::diag::Diagnostic> {
        self.diagnostic.as_ref()
    }

    pub fn into_diagnostic(self) -> Option<crate::diag::Diagnostic> {
        self.diagnostic
    }
}

/// Cursor into `roots[root_index]` plus a path of child indices.
pub(crate) trait ArenaCursor {
    fn parent(&self) -> &ArenaNode;
    fn take_child(&mut self, site: RegionSite) -> Result<(), WalkError>;
    fn last_child(&self) -> &ArenaNode;
    fn last_child_mut(&mut self) -> &mut ArenaNode;
    fn descend(&mut self);
    fn ascend(&mut self);
    fn assert_children_done(&self) -> Result<(), WalkError>;
}

struct RegionWalk<'a> {
    roots: &'a mut [ArenaNode],
    root_index: usize,
    path: Vec<usize>,
    next_child: usize,
}

pub(crate) struct RegionWalkRef<'a> {
    roots: &'a [ArenaNode],
    root_index: usize,
    path: Vec<usize>,
    next_child: usize,
}

macro_rules! arena_cursor_shared {
    () => {
        fn parent(&self) -> &ArenaNode {
            let mut node = &self.roots[self.root_index];
            for &index in &self.path {
                node = &node.children[index];
            }
            node
        }

        fn take_child(&mut self, site: RegionSite) -> Result<(), WalkError> {
            let index = self.next_child;
            let parent_label = self.parent().label.clone();
            let child_label = self
                .parent()
                .children
                .get(index)
                .map(|c| c.label.as_str())
                .ok_or_else(|| {
                    WalkError::message(format!(
                        "{site:?} has no child #{} on arena `{parent_label}`",
                        index
                    ))
                })?;
            if !site.matches(child_label) {
                return Err(WalkError::message(format!(
                    "expected {site:?}, got `{child_label}`"
                )));
            }
            self.next_child += 1;
            Ok(())
        }

        fn last_child(&self) -> &ArenaNode {
            let index = self.next_child - 1;
            &self.parent().children[index]
        }

        fn assert_children_done(&self) -> Result<(), WalkError> {
            let parent = self.parent();
            if self.next_child != parent.children.len() {
                return Err(WalkError::message(format!(
                    "arena `{}` has {} children, HIR visited {}",
                    parent.label,
                    parent.children.len(),
                    self.next_child
                )));
            }
            Ok(())
        }

        fn descend(&mut self) {
            self.path.push(self.next_child - 1);
            self.next_child = 0;
        }

        fn ascend(&mut self) {
            let finished = self.path.pop().expect("ascend without descend");
            self.next_child = finished + 1;
        }
    };
}

impl<'a> ArenaCursor for RegionWalk<'a> {
    arena_cursor_shared!();

    fn last_child_mut(&mut self) -> &mut ArenaNode {
        let index = self.next_child - 1;
        let mut node = &mut self.roots[self.root_index];
        for &child_index in &self.path {
            node = &mut node.children[child_index];
        }
        &mut node.children[index]
    }
}

impl<'a> ArenaCursor for RegionWalkRef<'a> {
    arena_cursor_shared!();

    fn last_child_mut(&mut self) -> &mut ArenaNode {
        panic!("readonly arena walk cannot mutate report nodes");
    }
}

/// Entry point for nested `walk_*` calls from a custom `RegionVisitor` hook.
pub struct WalkDriver<'c, C: ArenaCursor> {
    pub(crate) cursor: &'c mut C,
}

impl<'c, C: ArenaCursor> WalkDriver<'c, C> {
    pub fn walk_block<V: RegionVisitor>(
        &mut self,
        visitor: &mut V,
        block: &HirBlock,
        value_ty: Option<&Ty>,
    ) -> Result<(), WalkError> {
        visitor.walk_block(self, block, value_ty)
    }

    pub fn walk_expr<V: RegionVisitor>(&mut self, visitor: &mut V, expr: &HirExpr) -> Result<(), WalkError> {
        walk_expr(self, visitor, expr)
    }

    pub fn walk_region<V: RegionVisitor>(
        &mut self,
        visitor: &mut V,
        site: RegionSite,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        walk_region(self, visitor, site, body)
    }

    pub(crate) fn take_child(&mut self, site: RegionSite) -> Result<(), WalkError> {
        self.cursor.take_child(site)
    }

    pub(crate) fn last_child(&self) -> &ArenaNode {
        self.cursor.last_child()
    }

    pub(crate) fn descend(&mut self) {
        self.cursor.descend();
    }

    pub(crate) fn assert_children_done(&self) -> Result<(), WalkError> {
        self.cursor.assert_children_done()
    }

    pub(crate) fn ascend(&mut self) {
        self.cursor.ascend();
    }
}

pub trait RegionVisitor {
    fn touch_codegen_push(&self) -> bool {
        false
    }

    fn on_function_root(&mut self, _root: &ArenaNode) -> Result<(), WalkError> {
        Ok(())
    }

    fn on_function_root_mut(&mut self, root: &mut ArenaNode) -> Result<(), WalkError> {
        self.on_function_root(root)
    }

    fn begin_function_body(
        &mut self,
        _name: &str,
        _root: &ArenaNode,
        _body: &HirBlock,
    ) -> Result<(), WalkError> {
        Ok(())
    }

    fn end_function_body(&mut self) -> Result<(), WalkError> {
        Ok(())
    }

    fn enter_region(
        &mut self,
        site: RegionSite,
        child: &ArenaNode,
        body: &HirBlock,
    ) -> Result<(), WalkError>;
    fn loop_latch(&mut self, site: RegionSite) -> Result<(), WalkError>;
    fn exit_region(&mut self, site: RegionSite) -> Result<(), WalkError>;
    fn skip_closure(&mut self, child: &ArenaNode) -> Result<(), WalkError>;

    /// LLVM/codegen: emit `expr` after subexpressions were walked for region sync.
    fn after_expr<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        expr: &HirExpr,
    ) -> Result<(), WalkError> {
        let _ = (driver, expr);
        Ok(())
    }

    /// Walk a block's statements (and optional trailing value expression).
    fn walk_block<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        block: &HirBlock,
        value_ty: Option<&Ty>,
    ) -> Result<(), WalkError>
    where
        Self: Sized,
    {
        default_walk_block(self, driver, block, value_ty)
    }

    fn for_loop<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        _name: &str,
        iter: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), WalkError>
    where
        Self: Sized,
    {
        driver.walk_expr(self, iter)?;
        driver.walk_region(self, RegionSite::For, body)?;
        Ok(())
    }

    fn while_loop<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        cond: &HirExpr,
        body: &HirBlock,
    ) -> Result<(), WalkError>
    where
        Self: Sized,
    {
        driver.walk_expr(self, cond)?;
        driver.walk_region(self, RegionSite::While, body)?;
        Ok(())
    }

    fn if_expr<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        cond: &HirExpr,
        then_block: &HirBlock,
        else_block: Option<&HirBlock>,
        _result_ty: &Ty,
    ) -> Result<(), WalkError>
    where
        Self: Sized,
    {
        driver.walk_expr(self, cond)?;
        driver.walk_region(self, RegionSite::IfThen, then_block)?;
        if let Some(else_block) = else_block {
            driver.walk_region(self, RegionSite::IfElse, else_block)?;
        }
        Ok(())
    }

    /// Non-region statements (assign, var, return, break, …).
    fn on_stmt<C: ArenaCursor>(
        &mut self,
        driver: &mut WalkDriver<'_, C>,
        stmt: &HirStmt,
    ) -> Result<(), WalkError>
    where
        Self: Sized,
    {
        default_on_stmt(self, driver, stmt)
    }

    /// Trailing expression in a value-position block (`if` branch, etc.).
    fn on_trailing_value<C: ArenaCursor>(
        &mut self,
        _driver: &mut WalkDriver<'_, C>,
        _expr: &HirExpr,
        _ty: &Ty,
    ) -> Result<(), WalkError> {
        Ok(())
    }
}

fn default_walk_block<C: ArenaCursor, V: RegionVisitor>(
    visitor: &mut V,
    driver: &mut WalkDriver<'_, C>,
    block: &HirBlock,
    value_ty: Option<&Ty>,
) -> Result<(), WalkError> {
    if value_ty.is_none() {
        for stmt in &block.stmts {
            walk_stmt(driver, visitor, stmt)?;
        }
        return Ok(());
    }
    let Some((last, prefix)) = block.stmts.split_last() else {
        return Ok(());
    };
    for stmt in prefix {
        walk_stmt(driver, visitor, stmt)?;
    }
    if let HirStmt::Expr(expr) = last {
        visitor.on_trailing_value(driver, expr, value_ty.unwrap())?;
    } else {
        walk_stmt(driver, visitor, last)?;
    }
    Ok(())
}

fn default_on_stmt<C: ArenaCursor, V: RegionVisitor>(
    visitor: &mut V,
    driver: &mut WalkDriver<'_, C>,
    stmt: &HirStmt,
) -> Result<(), WalkError> {
    match stmt {
        HirStmt::VarDecl { value, .. }
        | HirStmt::Expr(value)
        | HirStmt::Return { value: Some(value) } => {
            driver.walk_expr(visitor, value)?;
            Ok(())
        }
        HirStmt::Assign { target, value } => {
            if let Some(index) = target.index() {
                driver.walk_expr(visitor, index)?;
            }
            driver.walk_expr(visitor, value)?;
            Ok(())
        }
        HirStmt::Return { value: None } | HirStmt::Break { .. } | HirStmt::Continue { .. } => Ok(()),
        HirStmt::Block(_) | HirStmt::MoveBlock { .. } | HirStmt::For { .. } | HirStmt::While { .. } => {
            unreachable!("region statements are dispatched in walk_stmt")
        }
    }
}

fn walk_stmt<C: ArenaCursor, V: RegionVisitor>(
    driver: &mut WalkDriver<'_, C>,
    visitor: &mut V,
    stmt: &HirStmt,
) -> Result<(), WalkError> {
    match stmt {
        HirStmt::Block(body) => driver.walk_region(visitor, RegionSite::Block, body),
        HirStmt::MoveBlock { body, .. } => driver.walk_region(visitor, RegionSite::MoveBlock, body),
        HirStmt::For { name, iter, body, .. } => visitor.for_loop(driver, name, iter, body),
        HirStmt::While { cond, body } => visitor.while_loop(driver, cond, body),
        other => visitor.on_stmt(driver, other),
    }
}

pub(crate) fn region_enter<C: ArenaCursor, V: RegionVisitor>(
    driver: &mut WalkDriver<'_, C>,
    visitor: &mut V,
    site: RegionSite,
    body: &HirBlock,
) -> Result<(), WalkError> {
    driver.cursor.take_child(site)?;
    let (peeled, _) = peel_blocks(body);
    if visitor.touch_codegen_push() {
        let child = driver.cursor.last_child_mut();
        child.codegen_push = codegen_push_for_region(&child.label, peeled);
    }
    let child_ref = driver.cursor.last_child();
    visitor.enter_region(site, child_ref, body)?;
    driver.cursor.descend();
    Ok(())
}

pub(crate) fn region_exit<C: ArenaCursor, V: RegionVisitor>(
    driver: &mut WalkDriver<'_, C>,
    visitor: &mut V,
    site: RegionSite,
) -> Result<(), WalkError> {
    driver.cursor.assert_children_done()?;
    driver.cursor.ascend();
    if matches!(site, RegionSite::For | RegionSite::While) {
        visitor.loop_latch(site)?;
    }
    visitor.exit_region(site)?;
    Ok(())
}

fn walk_region<C: ArenaCursor, V: RegionVisitor>(
    driver: &mut WalkDriver<'_, C>,
    visitor: &mut V,
    site: RegionSite,
    body: &HirBlock,
) -> Result<(), WalkError> {
    region_enter(driver, visitor, site, body)?;
    let (peeled, _) = peel_blocks(body);
    visitor.walk_block(driver, peeled, None)?;
    region_exit(driver, visitor, site)?;
    Ok(())
}

fn walk_expr<C: ArenaCursor, V: RegionVisitor>(
    driver: &mut WalkDriver<'_, C>,
    visitor: &mut V,
    expr: &HirExpr,
) -> Result<(), WalkError> {
    match &expr.kind {
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => visitor.if_expr(
            driver,
            cond,
            then_block,
            else_block.as_ref(),
            &expr.ty,
        ),
        HirExprKind::ArrayLit { elements } => {
            for element in elements {
                walk_expr(driver, visitor, element)?;
            }
            visitor.after_expr(driver, expr)
        }
        HirExprKind::Index { receiver, index, .. } => {
            walk_expr(driver, visitor, receiver)?;
            walk_expr(driver, visitor, index)?;
            visitor.after_expr(driver, expr)
        }
        HirExprKind::Slice { receiver, lo, hi } => {
            walk_expr(driver, visitor, receiver)?;
            walk_expr(driver, visitor, lo)?;
            walk_expr(driver, visitor, hi)?;
            visitor.after_expr(driver, expr)
        }
        HirExprKind::Some(inner)
        | HirExprKind::Unary { expr: inner, .. }
        | HirExprKind::Field {
            receiver: inner, ..
        } => {
            walk_expr(driver, visitor, inner)?;
            visitor.after_expr(driver, expr)
        }
        HirExprKind::Binary { op, lhs, rhs, .. } => {
            walk_expr(driver, visitor, lhs)?;
            if !matches!(op, BinOp::And | BinOp::Or) {
                walk_expr(driver, visitor, rhs)?;
            }
            visitor.after_expr(driver, expr)
        }
        HirExprKind::Call {
            callee,
            args,
            has_trailing_closure,
        } => {
            if !matches!(callee.kind, HirExprKind::Ident { .. }) {
                walk_expr(driver, visitor, callee)?;
            }
            for arg in args {
                walk_expr(driver, visitor, arg)?;
            }
            if *has_trailing_closure {
                driver.cursor.take_child(RegionSite::Closure)?;
                if visitor.touch_codegen_push() {
                    driver.cursor.last_child_mut().codegen_push = false;
                }
                let child_ref = driver.cursor.last_child();
                visitor.skip_closure(child_ref)?;
            }
            visitor.after_expr(driver, expr)
        }
        HirExprKind::Int { .. }
        | HirExprKind::Float { .. }
        | HirExprKind::Bool { .. }
        | HirExprKind::Str { .. }
        | HirExprKind::Ident { .. }
        | HirExprKind::None => visitor.after_expr(driver, expr),
    }
}

fn walk_one_function<C, V>(
    function: &HirFunction,
    cursor: &mut C,
    visitor: &mut V,
) -> Result<(), WalkError>
where
    C: ArenaCursor,
    V: RegionVisitor,
{
    let root = cursor.parent();
    visitor.begin_function_body(&function.name, root, &function.body)?;
    let (body, _) = peel_blocks(&function.body);
    let mut driver = WalkDriver { cursor };
    visitor.walk_block(&mut driver, body, None)?;
    cursor.assert_children_done()?;
    visitor.end_function_body()?;
    Ok(())
}

pub fn walk_program<V: RegionVisitor>(
    program: &HirProgram,
    roots: &mut [ArenaNode],
    visitor: &mut V,
) -> Result<(), WalkError> {
    if program.functions.len() != roots.len() {
        return Err(WalkError::message(format!(
            "HIR has {} functions, report has {}",
            program.functions.len(),
            roots.len()
        )));
    }
    for (root_index, function) in program.functions.iter().enumerate() {
        visitor
            .on_function_root_mut(&mut roots[root_index])
            .map_err(|_| WalkError::message("visitor rejected function root"))?;
        let mut cursor = RegionWalk {
            roots,
            root_index,
            path: Vec::new(),
            next_child: 0,
        };
        walk_one_function(function, &mut cursor, visitor)?;
    }
    Ok(())
}

pub fn walk_program_readonly<V: RegionVisitor>(
    program: &HirProgram,
    roots: &[ArenaNode],
    visitor: &mut V,
) -> Result<(), WalkError> {
    if program.functions.len() != roots.len() {
        return Err(WalkError::message(format!(
            "HIR has {} functions, report has {}",
            program.functions.len(),
            roots.len()
        )));
    }
    for (root_index, function) in program.functions.iter().enumerate() {
        visitor
            .on_function_root(&roots[root_index])
            .map_err(|_| WalkError::message("visitor rejected function root"))?;
        let mut cursor = RegionWalkRef {
            roots,
            root_index,
            path: Vec::new(),
            next_child: 0,
        };
        walk_one_function(function, &mut cursor, visitor)?;
    }
    Ok(())
}

/// Walk a single function's body in lockstep with `roots[root_index]`.
pub fn walk_function_readonly<V: RegionVisitor>(
    function: &HirFunction,
    root_index: usize,
    roots: &[ArenaNode],
    visitor: &mut V,
) -> Result<(), WalkError> {
    visitor
        .on_function_root(&roots[root_index])
        .map_err(|_| WalkError::message("visitor rejected function root"))?;
    let mut cursor = RegionWalkRef {
        roots,
        root_index,
        path: Vec::new(),
        next_child: 0,
    };
    walk_one_function(function, &mut cursor, visitor)
}

struct StampVisitor;

impl RegionVisitor for StampVisitor {
    fn touch_codegen_push(&self) -> bool {
        true
    }

    fn on_function_root_mut(&mut self, root: &mut ArenaNode) -> Result<(), WalkError> {
        root.codegen_push = true;
        Ok(())
    }

    fn enter_region(
        &mut self,
        _site: RegionSite,
        _child: &ArenaNode,
        _body: &HirBlock,
    ) -> Result<(), WalkError> {
        Ok(())
    }

    fn loop_latch(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        Ok(())
    }

    fn exit_region(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        Ok(())
    }

    fn skip_closure(&mut self, _child: &ArenaNode) -> Result<(), WalkError> {
        Ok(())
    }
}

pub fn stamp_codegen_push(program: &HirProgram, report: &mut crate::sema::ArenaReport) {
    let mut visitor = StampVisitor;
    walk_program(program, &mut report.roots, &mut visitor).expect("stamp walk");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::check;

    #[test]
    fn walk_consumes_every_report_child_on_mvp_sample() {
        let result = check(crate::MVP_SAMPLE);
        assert!(result.is_ok());
        let mut report = result.report.unwrap();
        let mut visitor = StampVisitor;
        walk_program(result.hir.as_ref().unwrap(), &mut report.roots, &mut visitor).unwrap();
    }

    #[test]
    fn empty_nested_block_codegen_push_false() {
        let result = check("fun main() {\n    val y = 2\n    { val x = 1 }\n}");
        assert!(result.is_ok());
        let mut report = result.report.unwrap();
        stamp_codegen_push(result.hir.as_ref().unwrap(), &mut report);
        assert!(!report.roots[0].children[0].codegen_push);
    }

    #[cfg(feature = "codegen")]
    mod codegen {
        use super::*;
        use crate::codegen::regions::{RegionEvent, schedule};
        use crate::sema::ArenaReport;

        fn node_by_id(report: &ArenaReport, id: usize) -> Option<&ArenaNode> {
            fn in_tree(node: &ArenaNode, id: usize) -> Option<&ArenaNode> {
                if node.id == id {
                    return Some(node);
                }
                for child in &node.children {
                    if let Some(found) = in_tree(child, id) {
                        return Some(found);
                    }
                }
                None
            }
            report.roots.iter().find_map(|root| in_tree(root, id))
        }

        #[test]
        fn schedule_pushes_match_codegen_push_flags() {
            let samples = [
                crate::MVP_SAMPLE,
                "fun main() { { val x = 1 } }",
                "fun main() {\n    val anchor = 1\n    { val s = \"x\" }\n}",
            ];
            for source in samples {
                let result = check(source);
                assert!(result.is_ok(), "{source:?}");
                let mut report = result.report.unwrap();
                stamp_codegen_push(result.hir.as_ref().unwrap(), &mut report);
                let hir = result.hir.as_ref().unwrap();
                let events = schedule(hir, &report).expect("schedule");
                for event in events {
                    if let RegionEvent::Push { arena } = event {
                        let node = node_by_id(&report, arena).expect("push for unknown arena");
                        let fun_root = report.roots.iter().any(|r| r.id == arena);
                        assert!(
                            fun_root || node.codegen_push,
                            "arena `{}` ({}) pushed without codegen_push",
                            node.label,
                            arena
                        );
                    }
                }
            }
        }
    }
}
