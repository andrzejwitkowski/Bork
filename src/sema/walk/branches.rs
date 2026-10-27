//! `if` and `if val` for the ownership walk: the branch arms, their moves, and the presence guards.

use super::{open_ordinary, walk, walk_block};
use crate::sema::env::{
    join_branch_moves, moved_names, restore_moved_flags, Analyzer, BranchEnd, Shadow, Ty,
};
use crate::sema::peel_blocks;
use crate::sema::policy::TransferSink;
use crate::sema::region::{RegionFrame, RegionParam};
use crate::sema::report::ArenaNode;
use crate::ast::{BindingKind, Block, ConditionalBinding, Expr, Pattern, Stmt};
use crate::span::SpannedName;
use std::collections::HashSet;

pub(super) fn walk_if(
    az: &mut Analyzer,
    cond: &Expr,
    then_block: &Block,
    else_block: Option<&Block>,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
    record_borrow: bool,
) {
    walk(az, cond, node, None, record_borrow);
    let tail_transfer = transfer.filter(|sink| matches!(sink, TransferSink::ManagedRef));
    let before_moved = moved_names(&az.env);
    node.children
        .push(open_ordinary(az, "IfThen", then_block, &[], tail_transfer));
    let then_end = branch_end(az, then_block, &[]);
    restore_moved_flags(&mut az.env, &before_moved);
    let else_end = else_arm_end(
        az,
        node,
        else_block,
        "IfElse",
        tail_transfer,
        before_moved,
    );
    join_branch_moves(&mut az.env, &[then_end, else_end]);
}

pub(super) fn walk_if_val(
    az: &mut Analyzer,
    bindings: &crate::ast::ConditionalBindings,
    then_block: &Block,
    else_block: Option<&Block>,
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
    record_borrow: bool,
) {
    walk(az, &bindings.head.value, node, None, record_borrow);
    let tail_transfer = transfer.filter(|sink| matches!(sink, TransferSink::ManagedRef));
    if let (true, Some(else_block)) = (az.option_if_vals.contains(&bindings.head.name.id), else_block) {
        walk_arms(az, &[then_block, else_block], node, tail_transfer);
        return;
    }
    let (body, compacted) = peel_blocks(then_block);
    let mut frame = RegionFrame::new(az.alloc_id(), "IfValSome".into(), compacted);
    bind_presence_guard(az, &mut frame, &bindings.head);
    for binding in &bindings.tail {
        walk(az, &binding.value, &mut frame.node, None, record_borrow);
        bind_presence_guard(az, &mut frame, binding);
    }
    // The else path sees the header's moves but not the body's.
    let header_moved = outer_moved_names(az, &frame.shadows);
    walk_block(az, body, &mut frame.node, &mut frame.shadows, tail_transfer);
    let then_end = branch_end(az, then_block, &frame.shadows);
    node.children.push(frame.finish(az));
    restore_moved_flags(&mut az.env, &header_moved);
    let else_end = else_arm_end(
        az,
        node,
        else_block,
        "IfValNone",
        tail_transfer,
        header_moved,
    );
    join_branch_moves(&mut az.env, &[then_end, else_end]);
}

/// `when` over patterns: each arm is an ordinary region. Pattern bindings are not tracked here;
/// ownership of matched payloads is #53's job.
pub(super) fn walk_match(
    az: &mut Analyzer,
    scrutinee: &Expr,
    arms: &[crate::ast::MatchArm],
    node: &mut ArenaNode,
    transfer: Option<&TransferSink>,
    record_borrow: bool,
) {
    walk(az, scrutinee, node, None, record_borrow);
    let tail_transfer = transfer.filter(|sink| matches!(sink, TransferSink::ManagedRef));
    let before_moved = moved_names(&az.env);
    let mut ends = Vec::with_capacity(arms.len());
    for arm in arms {
        let (body, compacted) = peel_blocks(&arm.body);
        let mut frame = RegionFrame::new(az.alloc_id(), "MatchArm".into(), compacted);
        let mut names = Vec::new();
        pattern_bindings(&arm.pat, &mut names);
        for name in names {
            // A bare payload-free variant parses as a binding but typeck never types it.
            let Some(ty) = az.node_tys.get(&name.id).cloned() else { continue };
            frame.bind_param(
                az,
                &RegionParam {
                    name: name.name.clone(),
                    ty,
                    kind: BindingKind::Val,
                    span: Some(name.span),
                    borrow_from: "pattern binding",
                },
            );
        }
        walk_block(az, body, &mut frame.node, &mut frame.shadows, tail_transfer);
        ends.push(branch_end(az, &arm.body, &frame.shadows));
        node.children.push(frame.finish(az));
        restore_moved_flags(&mut az.env, &before_moved);
    }
    join_branch_moves(&mut az.env, &ends);
}

fn walk_arms(az: &mut Analyzer, bodies: &[&Block], node: &mut ArenaNode, tail_transfer: Option<&TransferSink>) {
    let before_moved = moved_names(&az.env);
    let mut ends = Vec::with_capacity(bodies.len());
    for body in bodies {
        node.children.push(open_ordinary(az, "MatchArm", body, &[], tail_transfer));
        ends.push(branch_end(az, body, &[]));
        restore_moved_flags(&mut az.env, &before_moved);
    }
    join_branch_moves(&mut az.env, &ends);
}

fn pattern_bindings<'a>(pat: &'a Pattern, out: &mut Vec<&'a SpannedName>) {
    match pat {
        Pattern::Binding(name) => out.push(name),
        Pattern::Variant { args, .. } => args.iter().for_each(|arg| pattern_bindings(arg, out)),
        Pattern::Some { inner, .. } => pattern_bindings(inner, out),
        Pattern::Wildcard { .. } | Pattern::Literal { .. } | Pattern::None { .. } => {}
    }
}

/// True when control never falls out of `block` (ends in `return`/`break`/`continue`, or in a
/// nested block or `if` / `if val` whose every path does).
fn block_diverges(block: &Block) -> bool {
    match block.stmts.last() {
        Some(Stmt::Return(_) | Stmt::Break { .. } | Stmt::Continue { .. }) => true,
        Some(Stmt::Block(body) | Stmt::MoveBlock { body, .. }) => block_diverges(body),
        Some(Stmt::Expr(expr)) => expr_diverges(expr),
        _ => false,
    }
}

fn expr_diverges(expr: &Expr) -> bool {
    match expr {
        Expr::If {
            then_block,
            else_block: Some(else_block),
            ..
        }
        | Expr::IfVal {
            then_block,
            else_block: Some(else_block),
            ..
        } => block_diverges(then_block) && block_diverges(else_block),
        _ => false,
    }
}

/// Names moved in the current env, with shadowed names read from the outer binding.
/// A name can be shadowed more than once in a frame; the earliest shadow holds the outer
/// binding, so walk newest to oldest and let the oldest decide.
fn outer_moved_names(az: &Analyzer, shadows: &[Shadow]) -> HashSet<String> {
    let mut moved = moved_names(&az.env);
    for Shadow(name, prev) in shadows.iter().rev() {
        if prev.as_ref().is_some_and(|prev| prev.moved) {
            moved.insert(name.clone());
        } else {
            moved.remove(name);
        }
    }
    moved
}

fn branch_end(az: &Analyzer, block: &Block, shadows: &[Shadow]) -> BranchEnd {
    BranchEnd {
        falls_through: !block_diverges(block),
        moved: outer_moved_names(az, shadows),
    }
}

/// Walks the optional `else` arm. Without one, control falls through with `moved_without_else`.
fn else_arm_end(
    az: &mut Analyzer,
    node: &mut ArenaNode,
    else_block: Option<&Block>,
    label: &str,
    tail_transfer: Option<&TransferSink>,
    moved_without_else: HashSet<String>,
) -> BranchEnd {
    match else_block {
        Some(block) => {
            node.children
                .push(open_ordinary(az, label, block, &[], tail_transfer));
            branch_end(az, block, &[])
        }
        None => BranchEnd {
            falls_through: true,
            moved: moved_without_else,
        },
    }
}

/// Binds one `if val` / `when` name inside its `Some` region frame.
fn bind_presence_guard(az: &mut Analyzer, frame: &mut RegionFrame, binding: &ConditionalBinding) {
    let ty = az
        .node_tys
        .get(&binding.name.id)
        .cloned()
        .unwrap_or(Ty::Unknown);
    frame.bind_param(
        az,
        &RegionParam {
            name: binding.name.name.clone(),
            ty,
            kind: BindingKind::Val,
            span: Some(binding.name.span),
            borrow_from: "presence guard",
        },
    );
}
