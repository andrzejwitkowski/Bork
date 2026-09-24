//! Arena region schedule derived from `sema::ArenaReport` walked in lockstep with HIR.
//!
//! HIR carries no region ids, but sema opens exactly one `ArenaNode` per region site and
//! visits sites in source order, as typeck does when it lowers to HIR. Each HIR region site
//! therefore consumes the next unvisited child of the innermost open node.
//!
//! | `ArenaNode.label`     | HIR site                                   | Events                                 |
//! |-----------------------|--------------------------------------------|----------------------------------------|
//! | `fun {name}`          | `HirFunction.body`                         | push on entry, pop on exit             |
//! | `Block`               | `HirStmt::Block`                           | push / pop                             |
//! | `MoveBlock (move)`    | `HirStmt::MoveBlock`                       | push / pop                             |
//! | `ForLoop ({name})`    | `HirStmt::For` body (after `iter`)         | push before loop, reset at latch, pop after loop |
//! | `IfThen`              | `HirExprKind::If` `then_block` (after `cond`) | push / pop around the branch        |
//! | `IfElse`              | `HirExprKind::If` `else_block`             | push / pop around the branch           |
//! | `Closure`, `Closure (move)` | `HirExprKind::Call` with `has_trailing_closure` (after args) | none: HIR drops the body, the node is skipped |
//!
//! Peel rule: sema opens a region on the body with `[Block(inner)]` wrappers stripped
//! (`sema::peel_blocks`) and records the count in `compacted_braces`. Those wrapper
//! `HirStmt::Block`s get no region; `RegionEmitter::enter` peels the same wrappers and checks
//! the count against the report.

use std::fmt;

use crate::hir::{HirBlock, HirExpr, HirExprKind, HirProgram, HirStmt};
use crate::sema::{ArenaNode, ArenaReport};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionSite {
    Block,
    MoveBlock,
    For,
    IfThen,
    IfElse,
    Closure,
}

impl RegionSite {
    fn matches(self, label: &str) -> bool {
        match self {
            RegionSite::Block => label == "Block",
            RegionSite::MoveBlock => label == "MoveBlock (move)",
            RegionSite::For => label.starts_with("ForLoop ("),
            RegionSite::IfThen => label == "IfThen",
            RegionSite::IfElse => label == "IfElse",
            RegionSite::Closure => label == "Closure" || label == "Closure (move)",
        }
    }
}

/// Runtime arena operation, keyed by `ArenaNode::id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionEvent {
    Push { arena: usize },
    Reset { arena: usize },
    Pop { arena: usize },
}

/// Receives arena operations in emission order. LLVM codegen implements this with runtime calls.
pub trait RegionSink {
    fn push(&mut self, node: &ArenaNode);
    fn reset(&mut self, node: &ArenaNode);
    fn pop(&mut self, node: &ArenaNode);
}

impl RegionSink for Vec<RegionEvent> {
    fn push(&mut self, node: &ArenaNode) {
        self.push(RegionEvent::Push { arena: node.id });
    }

    fn reset(&mut self, node: &ArenaNode) {
        self.push(RegionEvent::Reset { arena: node.id });
    }

    fn pop(&mut self, node: &ArenaNode) {
        self.push(RegionEvent::Pop { arena: node.id });
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduleError {
    pub message: String,
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ScheduleError {}

fn mismatch(message: String) -> ScheduleError {
    ScheduleError { message }
}

struct Open<'r> {
    node: &'r ArenaNode,
    next_child: usize,
}

/// Tracks the open arenas while a HIR walker emits code and forwards push/reset/pop to `S`.
pub struct RegionEmitter<'r, S> {
    report: &'r ArenaReport,
    next_function: usize,
    stack: Vec<Open<'r>>,
    sink: S,
}

impl<'r, S: RegionSink> RegionEmitter<'r, S> {
    pub fn new(report: &'r ArenaReport, sink: S) -> Self {
        Self {
            report,
            next_function: 0,
            stack: Vec::new(),
            sink,
        }
    }

    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    pub fn into_sink(self) -> S {
        self.sink
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    pub fn enter_function<'h>(
        &mut self,
        name: &str,
        body: &'h HirBlock,
    ) -> Result<&'h HirBlock, ScheduleError> {
        if !self.stack.is_empty() {
            return Err(mismatch(format!(
                "function `{name}` entered inside an open region"
            )));
        }
        let node = self
            .report
            .roots
            .get(self.next_function)
            .ok_or_else(|| mismatch(format!("no arena for function `{name}`")))?;
        if node.label != format!("fun {name}") {
            return Err(mismatch(format!(
                "function `{name}` does not match arena `{}`",
                node.label
            )));
        }
        self.next_function += 1;
        self.open(node, body)
    }

    pub fn enter<'h>(
        &mut self,
        site: RegionSite,
        body: &'h HirBlock,
    ) -> Result<&'h HirBlock, ScheduleError> {
        let node = self.take_child(site)?;
        self.open(node, body)
    }

    pub fn skip(&mut self, site: RegionSite) -> Result<(), ScheduleError> {
        self.take_child(site).map(|_| ())
    }

    pub fn latch(&mut self) -> Result<(), ScheduleError> {
        let open = self
            .stack
            .last()
            .ok_or_else(|| mismatch("loop latch outside any region".into()))?;
        if !RegionSite::For.matches(&open.node.label) {
            return Err(mismatch(format!(
                "loop latch in non-loop arena `{}`",
                open.node.label
            )));
        }
        self.sink.reset(open.node);
        Ok(())
    }

    pub fn exit(&mut self) -> Result<(), ScheduleError> {
        let open = self
            .stack
            .pop()
            .ok_or_else(|| mismatch("region exit with no open region".into()))?;
        if open.next_child != open.node.children.len() {
            return Err(mismatch(format!(
                "arena `{}` has {} child regions, HIR visited {}",
                open.node.label,
                open.node.children.len(),
                open.next_child
            )));
        }
        self.sink.pop(open.node);
        Ok(())
    }

    pub fn finish(self) -> Result<S, ScheduleError> {
        if !self.stack.is_empty() || self.next_function != self.report.roots.len() {
            return Err(mismatch(format!(
                "report has {} function arenas, HIR visited {}",
                self.report.roots.len(),
                self.next_function
            )));
        }
        Ok(self.sink)
    }

    fn take_child(&mut self, site: RegionSite) -> Result<&'r ArenaNode, ScheduleError> {
        let open = self
            .stack
            .last_mut()
            .ok_or_else(|| mismatch(format!("{site:?} region outside any function")))?;
        let parent: &'r ArenaNode = open.node;
        let node = parent.children.get(open.next_child).ok_or_else(|| {
            mismatch(format!(
                "{site:?} region has no matching child in arena `{}`",
                parent.label
            ))
        })?;
        if !site.matches(&node.label) {
            return Err(mismatch(format!(
                "{site:?} region does not match arena `{}`",
                node.label
            )));
        }
        open.next_child += 1;
        Ok(node)
    }

    fn open<'h>(
        &mut self,
        node: &'r ArenaNode,
        body: &'h HirBlock,
    ) -> Result<&'h HirBlock, ScheduleError> {
        let (body, peeled) = peel_blocks(body);
        if peeled != node.compacted_braces {
            return Err(mismatch(format!(
                "arena `{}` compacted {} braces, HIR has {peeled}",
                node.label, node.compacted_braces
            )));
        }
        self.sink.push(node);
        self.stack.push(Open {
            node,
            next_child: 0,
        });
        Ok(body)
    }
}

/// HIR counterpart of `sema::peel_blocks`.
pub(super) fn peel_blocks(block: &HirBlock) -> (&HirBlock, usize) {
    let mut compacted = 0;
    let mut current = block;
    while let [HirStmt::Block(inner)] = current.stmts.as_slice() {
        compacted += 1;
        current = inner;
    }
    (current, compacted)
}

/// Static region schedule for `hir`: each site's events once, with a single `Reset` per loop latch.
pub fn schedule(hir: &HirProgram, report: &ArenaReport) -> Result<Vec<RegionEvent>, ScheduleError> {
    let mut emitter = RegionEmitter::new(report, Vec::new());
    for function in &hir.functions {
        let body = emitter.enter_function(&function.name, &function.body)?;
        schedule_block(&mut emitter, body)?;
        emitter.exit()?;
    }
    emitter.finish()
}

fn schedule_region<S: RegionSink>(
    emitter: &mut RegionEmitter<'_, S>,
    site: RegionSite,
    body: &HirBlock,
) -> Result<(), ScheduleError> {
    let body = emitter.enter(site, body)?;
    schedule_block(emitter, body)?;
    if site == RegionSite::For {
        emitter.latch()?;
    }
    emitter.exit()
}

fn schedule_block<S: RegionSink>(
    emitter: &mut RegionEmitter<'_, S>,
    block: &HirBlock,
) -> Result<(), ScheduleError> {
    for stmt in &block.stmts {
        match stmt {
            HirStmt::Block(body) => schedule_region(emitter, RegionSite::Block, body)?,
            HirStmt::MoveBlock { body, .. } => {
                schedule_region(emitter, RegionSite::MoveBlock, body)?
            }
            HirStmt::For { iter, body, .. } => {
                schedule_expr(emitter, iter)?;
                schedule_region(emitter, RegionSite::For, body)?;
            }
            HirStmt::VarDecl { value, .. }
            | HirStmt::Assign { value, .. }
            | HirStmt::Expr(value)
            | HirStmt::Return { value: Some(value) } => schedule_expr(emitter, value)?,
            HirStmt::Return { value: None } => {}
        }
    }
    Ok(())
}

fn schedule_expr<S: RegionSink>(
    emitter: &mut RegionEmitter<'_, S>,
    expr: &HirExpr,
) -> Result<(), ScheduleError> {
    match &expr.kind {
        HirExprKind::Int { .. }
        | HirExprKind::Str { .. }
        | HirExprKind::Ident { .. }
        | HirExprKind::None => Ok(()),
        HirExprKind::Some(inner)
        | HirExprKind::Unary { expr: inner, .. }
        | HirExprKind::Field {
            receiver: inner, ..
        } => schedule_expr(emitter, inner),
        HirExprKind::Binary { lhs, rhs, .. } => {
            schedule_expr(emitter, lhs)?;
            schedule_expr(emitter, rhs)
        }
        HirExprKind::Call {
            callee,
            args,
            has_trailing_closure,
        } => {
            schedule_expr(emitter, callee)?;
            for arg in args {
                schedule_expr(emitter, arg)?;
            }
            if *has_trailing_closure {
                emitter.skip(RegionSite::Closure)?;
            }
            Ok(())
        }
        HirExprKind::If {
            cond,
            then_block,
            else_block,
        } => {
            schedule_expr(emitter, cond)?;
            schedule_region(emitter, RegionSite::IfThen, then_block)?;
            if let Some(else_block) = else_block {
                schedule_region(emitter, RegionSite::IfElse, else_block)?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::check;

    fn schedule_of(source: &str) -> (Vec<RegionEvent>, ArenaReport) {
        let result = check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let report = result.report.unwrap();
        let events = schedule(result.hir.as_ref().unwrap(), &report).expect("schedule");
        (events, report)
    }

    fn max_depth(events: &[RegionEvent]) -> usize {
        let mut depth = 0usize;
        let mut max = 0;
        for event in events {
            match event {
                RegionEvent::Push { .. } => {
                    depth += 1;
                    max = max.max(depth);
                }
                RegionEvent::Pop { .. } => depth -= 1,
                RegionEvent::Reset { .. } => {}
            }
        }
        assert_eq!(depth, 0, "unbalanced: {events:?}");
        max
    }

    fn tree_depth(node: &ArenaNode) -> usize {
        1 + node.children.iter().map(tree_depth).max().unwrap_or(0)
    }

    fn count(events: &[RegionEvent], pred: fn(&RegionEvent) -> bool) -> usize {
        events.iter().filter(|e| pred(e)).count()
    }

    #[test]
    fn peeled_bare_block_opens_no_extra_region() {
        let (events, report) = schedule_of("fun main() { { val x = 1 } }");
        let root = &report.roots[0];
        assert_eq!(root.compacted_braces, 1);
        assert!(root.children.is_empty());
        assert_eq!(
            events,
            vec![
                RegionEvent::Push { arena: root.id },
                RegionEvent::Pop { arena: root.id },
            ]
        );
        assert_eq!(max_depth(&events), tree_depth(root));
    }

    #[test]
    fn unpeeled_nested_block_pushes_per_dump_node() {
        let src = "fun main() {\n    val y = 2\n    { val x = 1 }\n}";
        let (events, report) = schedule_of(src);
        let root = &report.roots[0];
        assert_eq!(root.children.len(), 1);
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Push { .. })), 2);
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Pop { .. })), 2);
        assert_eq!(max_depth(&events), tree_depth(root));
    }

    #[test]
    fn for_loop_resets_at_latch_and_if_branches_nest() {
        let src = r#"
fun main() {
    var total = 0
    for (i in 0..3) {
        if (i > 1) {
            total = total + i
        } else {
            total = total - 1
        }
    }
}
"#;
        let (events, report) = schedule_of(src);
        let root = &report.roots[0];
        let for_node = &root.children[0];
        assert!(for_node.label.starts_with("ForLoop"));
        let resets: Vec<_> = events
            .iter()
            .filter(|e| matches!(e, RegionEvent::Reset { .. }))
            .collect();
        assert_eq!(resets, vec![&RegionEvent::Reset { arena: for_node.id }]);
        let reset_at = events
            .iter()
            .position(|e| matches!(e, RegionEvent::Reset { .. }));
        let pop_for = events
            .iter()
            .position(|e| *e == RegionEvent::Pop { arena: for_node.id });
        assert_eq!(reset_at.map(|i| i + 1), pop_for);
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Push { .. })), 4);
        assert_eq!(max_depth(&events), tree_depth(root));
    }

    #[test]
    fn move_block_pushes_region() {
        let src = r#"
fun main() {
    val s: String = "hi"
    move (s) {
        val t = s
    }
}
"#;
        let (events, report) = schedule_of(src);
        let child = &report.roots[0].children[0];
        assert_eq!(child.label, "MoveBlock (move)");
        assert!(events.contains(&RegionEvent::Push { arena: child.id }));
        assert_eq!(max_depth(&events), 2);
    }

    #[test]
    fn trailing_closure_region_is_skipped() {
        let (events, report) = schedule_of(crate::MVP_SAMPLE);
        let main = &report.roots[1];
        let closure = &main.children[0].children[0];
        assert!(closure.label.starts_with("Closure"));
        assert!(!events.iter().any(|e| matches!(
            e,
            RegionEvent::Push { arena } if *arena == closure.id
        )));
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Push { .. })), 3);
        assert_eq!(max_depth(&events), 2);
    }

    #[test]
    fn mismatched_report_is_rejected() {
        let result = check("fun main() {\n    val y = 2\n    { val x = 1 }\n}");
        let mut report = result.report.unwrap();
        report.roots[0].children.clear();
        let err = schedule(result.hir.as_ref().unwrap(), &report).unwrap_err();
        assert!(err.message.contains("Block"), "{err}");
    }
}
