//! Arena region schedule derived from `sema::ArenaReport` walked in lockstep with HIR.
//!
//! HIR carries no region ids, but sema opens exactly one `ArenaNode` per region site and
//! visits sites in source order, as typeck does when it lowers to HIR. Each HIR region site
//! therefore consumes the next unvisited child of the innermost open node.
//!
//! | `ArenaNode.label`     | HIR site                                   | Events                                 |
//! |-----------------------|--------------------------------------------|----------------------------------------|
//! | `fun {name}`          | `HirFunction.body`                         | push on entry, pop on exit             |
//! | `Block`               | `HirStmt::Block`                           | push / pop when `codegen_push`         |
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

use crate::hir::{peel_blocks, HirBlock, HirProgram};
use crate::region_walk::{self, WalkError};
use crate::sema::{ArenaNode, ArenaReport};

pub use crate::region_walk::RegionSite;

/// Runtime arena operation, keyed by `ArenaNode::id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegionEvent {
    Push { arena: usize },
    Reset { arena: usize },
    Pop { arena: usize },
}

/// Receives arena operations in emission order. LLVM codegen implements this with runtime calls.
pub trait RegionSink {
    fn push(&mut self, arena: usize);
    fn reset(&mut self, arena: usize);
    fn pop(&mut self, arena: usize);
}

impl RegionSink for Vec<RegionEvent> {
    fn push(&mut self, arena: usize) {
        self.push(RegionEvent::Push { arena });
    }

    fn reset(&mut self, arena: usize) {
        self.push(RegionEvent::Reset { arena });
    }

    fn pop(&mut self, arena: usize) {
        self.push(RegionEvent::Pop { arena });
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

struct Open {
    arena_id: usize,
    label: String,
    expected_children: usize,
    next_child: usize,
    pushed: bool,
    /// Children consumed by `region_walk` (schedule), not by `take_child`.
    children_by_walk: bool,
}

/// Tracks the open arenas while a HIR walker emits code and forwards push/reset/pop to `S`.
pub struct RegionEmitter<'report, S> {
    report: Option<&'report ArenaReport>,
    function_count: usize,
    pub(super) next_function: usize,
    stack: Vec<Open>,
    sink: S,
}

impl<'report, S: RegionSink> RegionEmitter<'report, S> {
    pub fn new(report: &'report ArenaReport, sink: S) -> Self {
        Self {
            report: Some(report),
            function_count: report.roots.len(),
            next_function: 0,
            stack: Vec::new(),
            sink,
        }
    }

    pub fn function_index(&self) -> usize {
        self.next_function
    }

    pub fn for_schedule(function_count: usize, sink: S) -> Self {
        Self {
            report: None,
            function_count,
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
        let report = self
            .report
            .ok_or_else(|| mismatch("enter_function requires an arena report".into()))?;
        let node = report
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
        self.open(node, body, None)
    }

    pub fn enter<'h>(
        &mut self,
        site: RegionSite,
        body: &'h HirBlock,
    ) -> Result<&'h HirBlock, ScheduleError> {
        let node = self.take_child(site)?;
        self.open(node, body, Some(site))
    }

    pub fn skip(&mut self, site: RegionSite) -> Result<(), ScheduleError> {
        self.take_child(site).map(|_| ())
    }

    pub fn latch(&mut self) -> Result<(), ScheduleError> {
        let open = self
            .stack
            .last()
            .ok_or_else(|| mismatch("loop latch outside any region".into()))?;
        if !RegionSite::For.matches(&open.label)
            && !RegionSite::While.matches(&open.label)
        {
            return Err(mismatch(format!(
                "loop latch in non-loop arena `{}`",
                open.label
            )));
        }
        if !open.pushed {
            return Ok(());
        }
        self.sink.reset(open.arena_id);
        Ok(())
    }

    pub fn exit(&mut self) -> Result<(), ScheduleError> {
        let open = self
            .stack
            .pop()
            .ok_or_else(|| mismatch("region exit with no open region".into()))?;
        if !open.children_by_walk && open.next_child != open.expected_children {
            return Err(mismatch(format!(
                "arena `{}` has {} child regions, HIR visited {}",
                open.label,
                open.expected_children,
                open.next_child
            )));
        }
        if open.pushed {
            self.sink.pop(open.arena_id);
        }
        Ok(())
    }

    /// Pops the region stack after an early `return` already emitted arena pops via `unwind`.
    pub fn exit_after_return(&mut self) -> Result<(), ScheduleError> {
        let open = self
            .stack
            .pop()
            .ok_or_else(|| mismatch("region exit with no open region".into()))?;
        if open.pushed {
            self.sink.pop(open.arena_id);
        }
        Ok(())
    }

    pub fn finish(self) -> Result<S, ScheduleError> {
        if !self.stack.is_empty() || self.next_function != self.function_count {
            return Err(mismatch(format!(
                "report has {} function arenas, HIR visited {}",
                self.function_count,
                self.next_function
            )));
        }
        Ok(self.sink)
    }

    fn take_child(&mut self, site: RegionSite) -> Result<&'report ArenaNode, ScheduleError> {
        let frame = self
            .stack
            .len()
            .checked_sub(1)
            .ok_or_else(|| mismatch(format!("{site:?} region outside any function")))?;
        let parent_id = self.stack[frame].arena_id;
        let child_index = self.stack[frame].next_child;
        let report = self
            .report
            .ok_or_else(|| mismatch("take_child requires an arena report".into()))?;
        let parent = find_node_by_id(report, parent_id)?;
        let node = parent.children.get(child_index).ok_or_else(|| {
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
        self.stack[frame].next_child += 1;
        Ok(node)
    }

    fn push_open<'h>(
        &mut self,
        node: &ArenaNode,
        body: &'h HirBlock,
        site: Option<RegionSite>,
        children_by_walk: bool,
    ) -> Result<&'h HirBlock, ScheduleError> {
        let (body, peeled) = peel_blocks(body);
        if peeled != node.compacted_braces {
            return Err(mismatch(format!(
                "arena `{}` compacted {} braces, HIR has {peeled}",
                node.label, node.compacted_braces
            )));
        }
        let pushed = site.is_none() || node.codegen_push;
        if pushed {
            self.sink.push(node.id);
        }
        self.stack.push(Open {
            arena_id: node.id,
            label: node.label.clone(),
            expected_children: node.children.len(),
            next_child: 0,
            pushed,
            children_by_walk,
        });
        Ok(body)
    }

    fn open<'h>(
        &mut self,
        node: &'report ArenaNode,
        body: &'h HirBlock,
        site: Option<RegionSite>,
    ) -> Result<&'h HirBlock, ScheduleError> {
        self.push_open(node, body, site, false)
    }

    /// Opens a region whose report child was already matched by `region_walk`.
    pub fn open_known<'h>(
        &mut self,
        node: &ArenaNode,
        body: &'h HirBlock,
        site: Option<RegionSite>,
    ) -> Result<&'h HirBlock, ScheduleError> {
        self.push_open(node, body, site, true)
    }
}

fn find_node_by_id<'a>(report: &'a ArenaReport, id: usize) -> Result<&'a ArenaNode, ScheduleError> {
    for root in &report.roots {
        if let Some(node) = find_node_in_tree(root, id) {
            return Ok(node);
        }
    }
    Err(mismatch(format!("arena id {id} not in report")))
}

fn find_node_in_tree(node: &ArenaNode, id: usize) -> Option<&ArenaNode> {
    if node.id == id {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_node_in_tree(child, id) {
            return Some(found);
        }
    }
    None
}

fn schedule_walk(err: ScheduleError) -> WalkError {
    WalkError::message(err.to_string())
}

struct ScheduleVisitor<'e, 'report, S> {
    emitter: &'e mut RegionEmitter<'report, S>,
}

impl<'e, 'report, S: RegionSink> region_walk::RegionVisitor for ScheduleVisitor<'e, 'report, S> {
    fn begin_function_body(
        &mut self,
        _name: &str,
        root: &ArenaNode,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        self.emitter
            .open_known(root, body, None)
            .map_err(schedule_walk)?;
        Ok(())
    }

    fn end_function_body(&mut self) -> Result<(), WalkError> {
        self.emitter.exit().map_err(schedule_walk)?;
        self.emitter.next_function += 1;
        Ok(())
    }

    fn enter_region(
        &mut self,
        site: RegionSite,
        child: &ArenaNode,
        body: &HirBlock,
    ) -> Result<(), WalkError> {
        self.emitter
            .open_known(child, body, Some(site))
            .map_err(schedule_walk)?;
        Ok(())
    }

    fn loop_latch(&mut self, site: RegionSite) -> Result<(), WalkError> {
        if matches!(site, RegionSite::For | RegionSite::While) {
            self.emitter.latch().map_err(schedule_walk)?;
        }
        Ok(())
    }

    fn exit_region(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        self.emitter.exit().map_err(schedule_walk)
    }

    fn skip_closure(&mut self, _child: &ArenaNode) -> Result<(), WalkError> {
        Ok(())
    }
}

fn walk_err(err: WalkError) -> ScheduleError {
    mismatch(err.as_str().into())
}

/// Static region schedule for `hir`: each site's events once, with a single `Reset` per loop latch.
pub fn schedule(hir: &HirProgram, report: &ArenaReport) -> Result<Vec<RegionEvent>, ScheduleError> {
    let mut emitter = RegionEmitter::for_schedule(report.roots.len(), Vec::new());
    {
        let mut visitor = ScheduleVisitor {
            emitter: &mut emitter,
        };
        region_walk::walk_program_readonly(hir, &report.roots, &mut visitor)
            .map_err(walk_err)?;
    }
    emitter.finish()
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
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Push { .. })), 1);
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Pop { .. })), 1);
        assert_eq!(max_depth(&events), 1);
    }

    #[test]
    fn nested_block_with_arena_alloc_pushes_region() {
        let src = "fun main() {\n    val anchor = 1\n    { val s = \"x\" }\n}";
        let (events, report) = schedule_of(src);
        let root = &report.roots[0];
        assert_eq!(root.children.len(), 1);
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Push { .. })), 2);
        assert_eq!(max_depth(&events), tree_depth(root));
    }

    #[test]
    fn for_loop_resets_at_latch_and_if_branches_nest() {
        let src = r#"
fun main() {
    var total = 0
    for (i in 0..3) {
        val _bump = "."
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
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Push { .. })), 2);
        assert_eq!(max_depth(&events), 2);
    }

    #[test]
    fn move_block_pushes_region() {
        let src = r#"
fun main() {
    val s: String = "hi"
    move (s) {
        val t = "x"
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
        assert_eq!(count(&events, |e| matches!(e, RegionEvent::Push { .. })), 2);
        assert_eq!(max_depth(&events), 1);
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
