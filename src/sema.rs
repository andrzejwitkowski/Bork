//! Compile-time region / ownership analysis.

use crate::ast::{BindingKind, Block, Expr, Function, Program, Stmt, Type};
use crate::span::{Span, SpannedName};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    Local,
    Copy,
    Shared { from: String },
    Moved { from: String },
}

impl Ownership {
    pub fn dump_tag(&self) -> String {
        match self {
            Ownership::Local => "[Local]".into(),
            Ownership::Copy => "[Copy]".into(),
            Ownership::Shared { from } => format!("[Shared ← {from}]"),
            Ownership::Moved { from } => format!("[Moved ← {from}]"),
        }
    }

    pub fn hover_label(&self) -> String {
        match self {
            Ownership::Local => "Local".into(),
            Ownership::Copy => "Copy".into(),
            Ownership::Shared { from } => format!("Shared ← {from}"),
            Ownership::Moved { from } => format!("Moved ← {from}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BindingInfo {
    pub name: String,
    pub ownership: Ownership,
    pub ty: Option<Type>,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArenaNode {
    pub id: usize,
    pub label: String,
    pub compacted_braces: usize,
    pub bindings: Vec<BindingInfo>,
    pub children: Vec<ArenaNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArenaReport {
    pub roots: Vec<ArenaNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemaError {
    pub message: String,
    pub name: Option<String>,
    pub span: Option<Span>,
}

#[derive(Clone)]
struct EnvBinding {
    arena_id: usize,
    arena_label: String,
    ty: Option<Type>,
    kind: BindingKind,
    moved: bool,
}

struct Analyzer {
    next_id: usize,
    errors: Vec<SemaError>,
    env: HashMap<String, EnvBinding>,
}

impl Analyzer {
    fn new() -> Self {
        Self {
            next_id: 0,
            errors: Vec::new(),
            env: HashMap::new(),
        }
    }

    fn alloc_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn error(
        &mut self,
        message: impl Into<String>,
        name: Option<String>,
        span: Option<Span>,
    ) {
        self.errors.push(SemaError {
            message: message.into(),
            name,
            span,
        });
    }
}

struct Shadow(String, Option<EnvBinding>);

fn shadow_insert(az: &mut Analyzer, name: String, binding: EnvBinding) -> Shadow {
    let prev = az.env.insert(name.clone(), binding);
    Shadow(name, prev)
}

fn bind(
    az: &mut Analyzer,
    name: &str,
    arena_id: usize,
    arena_label: &str,
    ty: Option<Type>,
    kind: BindingKind,
) -> Shadow {
    shadow_insert(
        az,
        name.to_string(),
        EnvBinding {
            arena_id,
            arena_label: arena_label.to_string(),
            ty,
            kind,
            moved: false,
        },
    )
}

fn shadow_restore(az: &mut Analyzer, Shadow(name, prev): Shadow) {
    match prev {
        Some(prev) => {
            az.env.insert(name, prev);
        }
        None => {
            az.env.remove(&name);
        }
    }
}

/// Analyze `program` for arena hierarchy and Copy/Move ownership.
pub fn analyze(program: &Program) -> (ArenaReport, Vec<SemaError>) {
    let mut az = Analyzer::new();
    let roots = program
        .functions
        .iter()
        .map(|f| analyze_function(&mut az, f))
        .collect();
    (ArenaReport { roots }, az.errors)
}

/// Peel pure nested bare-block wrappers; leave nested region structure intact.
pub fn collapse_block(block: Block) -> (Block, usize) {
    let mut compacted = 0;
    let mut current = block;
    while let [Stmt::Block(inner)] = current.stmts.as_slice() {
        compacted += 1;
        current = inner.clone();
    }
    (current, compacted)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionKind {
    Ordinary,
    Move,
}

fn analyze_function(az: &mut Analyzer, func: &Function) -> ArenaNode {
    let params: Vec<(String, Option<Type>)> = func
        .params
        .iter()
        .map(|p| (p.name.clone(), Some(p.ty.clone())))
        .collect();
    open_region(
        az,
        &format!("fun {}", func.name),
        &func.body,
        None,
        &params,
        RegionKind::Ordinary,
    )
}

fn resolve_move_captures(
    az: &Analyzer,
    body: &Block,
    explicit_captures: Option<&[SpannedName]>,
    param_names: &[String],
) -> Vec<SpannedName> {
    match explicit_captures {
        Some(caps) => caps.to_vec(),
        None => free_vars_in_block(body)
            .into_iter()
            .filter(|n| !param_names.contains(&n.name))
            .filter(|n| az.env.get(&n.name).is_some_and(|b| !b.moved))
            .collect(),
    }
}

fn open_region(
    az: &mut Analyzer,
    label: &str,
    body: &Block,
    explicit_captures: Option<&[SpannedName]>,
    params: &[(String, Option<Type>)],
    kind: RegionKind,
) -> ArenaNode {
    let (body, compacted) = collapse_block(body.clone());
    let id = az.alloc_id();
    let is_move = matches!(kind, RegionKind::Move);
    let label = if is_move {
        format!("{label} (move)")
    } else {
        label.to_string()
    };

    let param_names: Vec<String> = params.iter().map(|(n, _)| n.clone()).collect();
    let captures = if is_move {
        resolve_move_captures(az, &body, explicit_captures, &param_names)
    } else {
        Vec::new()
    };

    let mut node = ArenaNode {
        id,
        label: label.clone(),
        compacted_braces: compacted,
        bindings: Vec::new(),
        children: Vec::new(),
    };

    let mut shadows = Vec::new();
    let mut moved_parents = Vec::new();

    for (name, ty) in params {
        shadows.push(bind(
            az,
            name,
            id,
            &label,
            ty.clone(),
            BindingKind::Val,
        ));
        node.bindings.push(BindingInfo {
            name: name.clone(),
            ownership: Ownership::Local,
            ty: ty.clone(),
            span: None,
        });
    }

    for cap in &captures {
        match az.env.get(&cap.name).cloned() {
            None => az.error(
                format!("cannot move unknown name `{}`", cap.name),
                Some(cap.name.clone()),
                Some(cap.span),
            ),
            Some(b) if b.moved => az.error(
                format!(
                    "cannot move `{}`: already moved from {}",
                    cap.name, b.arena_label
                ),
                Some(cap.name.clone()),
                Some(cap.span),
            ),
            Some(b) => {
                moved_parents.push(cap.name.clone());
                shadows.push(bind(
                    az,
                    &cap.name,
                    id,
                    &label,
                    b.ty.clone(),
                    BindingKind::Val,
                ));
                node.bindings.push(BindingInfo {
                    name: cap.name.clone(),
                    ownership: Ownership::Moved {
                        from: b.arena_label,
                    },
                    ty: b.ty,
                    span: Some(cap.span),
                });
            }
        }
    }

    let mut locals = param_names;
    locals.extend(captures.iter().map(|c| c.name.clone()));
    let mut var_shadows = Vec::new();
    walk_block(az, &body, &mut node, &mut locals, &mut var_shadows);

    for n in &locals {
        az.env.remove(n);
    }
    for s in var_shadows.into_iter().rev() {
        shadow_restore(az, s);
    }
    for s in shadows.into_iter().rev() {
        shadow_restore(az, s);
    }
    for cap in moved_parents {
        if let Some(b) = az.env.get_mut(&cap) {
            b.moved = true;
        }
    }

    node
}

fn walk_block(
    az: &mut Analyzer,
    block: &Block,
    node: &mut ArenaNode,
    local_names: &mut Vec<String>,
    var_shadows: &mut Vec<Shadow>,
) {
    for stmt in &block.stmts {
        walk_stmt(az, stmt, node, local_names, var_shadows);
    }
}

fn walk_stmt(
    az: &mut Analyzer,
    stmt: &Stmt,
    node: &mut ArenaNode,
    local_names: &mut Vec<String>,
    var_shadows: &mut Vec<Shadow>,
) {
    match stmt {
        Stmt::Block(body) => {
            node.children.push(open_region(
                az,
                "Block",
                body,
                None,
                &[],
                RegionKind::Ordinary,
            ));
        }
        Stmt::MoveBlock { captures, body } => {
            node.children.push(open_region(
                az,
                "MoveBlock",
                body,
                captures.as_deref(),
                &[],
                RegionKind::Move,
            ));
        }
        Stmt::VarDecl {
            kind,
            name,
            name_span,
            ty,
            value,
        } => {
            walk_expr(az, value, node);
            let inferred = ty.clone().or_else(|| infer_type(az, value));
            var_shadows.push(shadow_insert(
                az,
                name.clone(),
                EnvBinding {
                    arena_id: node.id,
                    arena_label: node.label.clone(),
                    ty: inferred.clone(),
                    kind: *kind,
                    moved: false,
                },
            ));
            local_names.push(name.clone());
            node.bindings.push(BindingInfo {
                name: name.clone(),
                ownership: Ownership::Local,
                ty: inferred,
                span: Some(*name_span),
            });
        }
        Stmt::Assign { name, name_span, value } => {
            note_use(az, name, Some(*name_span), node);
            walk_expr(az, value, node);
        }
        Stmt::For { name, iter, body } => {
            walk_expr(az, iter, node);
            let params = [(name.clone(), Some(Type::from_ident("Int", false)))];
            node.children.push(open_region(
                az,
                &format!("ForLoop ({name})"),
                body,
                None,
                &params,
                RegionKind::Ordinary,
            ));
        }
        Stmt::Return(Some(e)) => walk_expr(az, e, node),
        Stmt::Return(None) => {}
        Stmt::Expr(e) => walk_expr(az, e, node),
    }
}

fn walk_expr(az: &mut Analyzer, expr: &Expr, node: &mut ArenaNode) {
    match expr {
        Expr::Ident { name, span } => note_use(az, name, Some(*span), node),
        Expr::Some(e) => walk_expr(az, e, node),
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(az, lhs, node);
            walk_expr(az, rhs, node);
        }
        Expr::Unary { expr, .. } => walk_expr(az, expr, node),
        Expr::Field { receiver, .. } => walk_expr(az, receiver, node),
        Expr::Call {
            callee,
            args,
            trailing,
        } => {
            walk_expr(az, callee, node);
            for a in args {
                walk_expr(az, a, node);
            }
            if let Some(c) = trailing {
                let params: Vec<(String, Option<Type>)> =
                    c.params.iter().map(|p| (p.name.clone(), None)).collect();
                node.children.push(open_region(
                    az,
                    "Closure",
                    &c.body,
                    c.captures.as_deref(),
                    &params,
                    if c.is_move {
                        RegionKind::Move
                    } else {
                        RegionKind::Ordinary
                    },
                ));
            }
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            walk_expr(az, cond, node);
            let env_before = az.env.clone();
            node.children.push(open_region(
                az,
                "IfThen",
                then_block,
                None,
                &[],
                RegionKind::Ordinary,
            ));
            let env_after_then = az.env.clone();
            az.env = env_before.clone();
            let env_after_else = if let Some(else_b) = else_block {
                node.children.push(open_region(
                    az,
                    "IfElse",
                    else_b,
                    None,
                    &[],
                    RegionKind::Ordinary,
                ));
                Some(az.env.clone())
            } else {
                None
            };
            az.env = env_before;
            for (name, binding) in az.env.iter_mut() {
                let then_moved = env_after_then.get(name).is_some_and(|b| b.moved);
                binding.moved = match &env_after_else {
                    Some(env_else) => {
                        let else_moved = env_else.get(name).is_some_and(|b| b.moved);
                        then_moved && else_moved
                    }
                    None => then_moved && binding.moved,
                };
            }
        }
        Expr::Int(_) | Expr::Str(_) | Expr::None => {}
    }
}

fn note_use(az: &mut Analyzer, name: &str, span: Option<Span>, node: &mut ArenaNode) {
    let Some(b) = az.env.get(name).cloned() else {
        return;
    };
    if b.moved {
        az.error(
            format!("use of `{name}` after move from {}", b.arena_label),
            Some(name.to_string()),
            span,
        );
        return;
    }
    if b.arena_id == node.id {
        return;
    }
    let is_copy = match &b.ty {
        Some(t) => t.is_copy(),
        None => false,
    };
    if is_copy {
        record_obs(node, name, Ownership::Copy, b.ty, span);
        return;
    }
    if matches!(b.kind, BindingKind::Val) {
        record_obs(
            node,
            name,
            Ownership::Shared {
                from: b.arena_label,
            },
            b.ty,
            span,
        );
        return;
    }
    az.error(
        format!(
            "`{name}` is not Copy; move it into `{}` with `move`",
            node.label
        ),
        Some(name.to_string()),
        span,
    );
}

fn record_obs(
    node: &mut ArenaNode,
    name: &str,
    ownership: Ownership,
    ty: Option<Type>,
    span: Option<Span>,
) {
    if node.bindings.iter().any(|x| x.name == name) {
        return;
    }
    node.bindings.push(BindingInfo {
        name: name.to_string(),
        ownership,
        ty,
        span,
    });
}

fn infer_type(az: &Analyzer, expr: &Expr) -> Option<Type> {
    match expr {
        Expr::Int(_) => Some(Type::from_ident("Int", false)),
        Expr::Str(_) => Some(Type::Named {
            name: "String".into(),
            nullable: false,
        }),
        Expr::Ident { name, .. } => az.env.get(name).and_then(|b| b.ty.clone()),
        Expr::Some(inner) => infer_type(az, inner).map(|ty| match ty {
            Type::Primitive { name, .. } => Type::Primitive {
                name,
                nullable: true,
            },
            Type::Named { name, .. } => Type::Named {
                name,
                nullable: true,
            },
            Type::Func { params, ret, .. } => Type::Func {
                params,
                ret,
                nullable: true,
            },
        }),
        _ => None,
    }
}

fn free_vars_in_block(block: &Block) -> Vec<SpannedName> {
    let mut free: HashMap<String, Span> = HashMap::new();
    let mut bound = HashSet::new();
    collect_block(block, &mut free, &mut bound);
    free.into_iter()
        .map(|(name, span)| SpannedName::new(name, span))
        .collect()
}

fn collect_block(
    block: &Block,
    free: &mut HashMap<String, Span>,
    bound: &mut HashSet<String>,
) {
    for stmt in &block.stmts {
        collect_stmt(stmt, free, bound);
    }
}

fn collect_stmt(
    stmt: &Stmt,
    free: &mut HashMap<String, Span>,
    bound: &mut HashSet<String>,
) {
    match stmt {
        Stmt::Block(b) => {
            let mut inner = bound.clone();
            collect_block(b, free, &mut inner);
        }
        Stmt::MoveBlock { body: b, captures } => {
            if let Some(caps) = captures {
                for cap in caps {
                    if !bound.contains(&cap.name) {
                        free.entry(cap.name.clone()).or_insert(cap.span);
                    }
                }
            }
            let mut inner = bound.clone();
            if let Some(caps) = captures {
                for cap in caps {
                    inner.insert(cap.name.clone());
                }
            }
            collect_block(b, free, &mut inner);
        }
        Stmt::VarDecl { name, value, .. } => {
            collect_expr(value, free, bound);
            bound.insert(name.clone());
        }
        Stmt::Assign {
            name,
            name_span,
            value,
        } => {
            if !bound.contains(name) {
                free.entry(name.clone()).or_insert(*name_span);
            }
            collect_expr(value, free, bound);
        }
        Stmt::For { name, iter, body } => {
            collect_expr(iter, free, bound);
            let mut inner = bound.clone();
            inner.insert(name.clone());
            collect_block(body, free, &mut inner);
        }
        Stmt::Return(Some(e)) | Stmt::Expr(e) => collect_expr(e, free, bound),
        Stmt::Return(None) => {}
    }
}

fn collect_expr(
    expr: &Expr,
    free: &mut HashMap<String, Span>,
    bound: &mut HashSet<String>,
) {
    match expr {
        Expr::Ident { name, span } => {
            if !bound.contains(name) {
                free.entry(name.clone()).or_insert(*span);
            }
        }
        Expr::Some(e) => collect_expr(e, free, bound),
        Expr::Binary { lhs, rhs, .. } => {
            collect_expr(lhs, free, bound);
            collect_expr(rhs, free, bound);
        }
        Expr::Unary { expr, .. } => collect_expr(expr, free, bound),
        Expr::Field { receiver, .. } => collect_expr(receiver, free, bound),
        Expr::Call {
            callee,
            args,
            trailing,
        } => {
            collect_expr(callee, free, bound);
            for a in args {
                collect_expr(a, free, bound);
            }
            if let Some(c) = trailing {
                let mut inner = bound.clone();
                for p in &c.params {
                    inner.insert(p.name.clone());
                }
                if let Some(caps) = &c.captures {
                    for cap in caps {
                        if !bound.contains(&cap.name) {
                            free.entry(cap.name.clone()).or_insert(cap.span);
                        }
                        inner.insert(cap.name.clone());
                    }
                }
                collect_block(&c.body, free, &mut inner);
            }
        }
        Expr::If {
            cond,
            then_block,
            else_block,
        } => {
            collect_expr(cond, free, bound);
            collect_block(then_block, free, &mut bound.clone());
            if let Some(e) = else_block {
                collect_block(e, free, &mut bound.clone());
            }
        }
        Expr::Int(_) | Expr::Str(_) | Expr::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dump::dump_arenas;
    use crate::parse;

    #[test]
    fn collapses_nested_bare_braces() {
        let (b, n) = collapse_block(Block {
            stmts: vec![Stmt::Block(Block {
                stmts: vec![Stmt::Block(Block {
                    stmts: vec![Stmt::Return(None)],
                })],
            })],
        });
        assert_eq!(n, 2);
        assert!(matches!(b.stmts[0], Stmt::Return(None)));
    }

    #[test]
    fn mvp_sample_analyzes_without_move_errors() {
        let prog = parse(crate::MVP_SAMPLE).expect("parse");
        let (report, errs) = analyze(&prog);
        assert!(errs.is_empty(), "{errs:?}");
        assert!(!report.roots.is_empty());
    }

    #[test]
    fn move_block_marks_binding_moved() {
        let src = r#"
fun main() {
    val s: String = "hi"
    move (s) {
        return 1
    }
    return 0
}
"#;
        let prog = parse(src).expect("parse");
        let (report, errs) = analyze(&prog);
        assert!(errs.is_empty(), "{errs:?}");
        let main = &report.roots[0];
        let move_child = main
            .children
            .iter()
            .find(|c| c.label.contains("MoveBlock"))
            .expect("move child");
        assert!(move_child.bindings.iter().any(|b| {
            b.name == "s" && matches!(b.ownership, Ownership::Moved { .. })
        }));
    }

    #[test]
    fn use_after_move_errors() {
        let src = r#"
fun main() {
    val s: String = "hi"
    move (s) {
        return 1
    }
    val t = s
}
"#;
        let prog = parse(src).expect("parse");
        let (_, errs) = analyze(&prog);
        assert!(
            errs.iter().any(|e| e.message.contains("after move")),
            "{errs:?}"
        );
    }

    #[test]
    fn non_copy_var_without_move_errors() {
        let src = r#"
fun main() {
    var s: String = "hi"
    {
        val t = s
    }
}
"#;
        let prog = parse(src).expect("parse");
        let (_, errs) = analyze(&prog);
        assert!(
            errs.iter().any(|e| e.message.contains("not Copy")),
            "{errs:?}"
        );
    }

    #[test]
    fn val_string_shared_across_arenas() {
        let src = r#"
fun main() {
    val s: String = "hi"
    {
        val t = s
    }
}
"#;
        let prog = parse(src).expect("parse");
        let (report, errs) = analyze(&prog);
        assert!(errs.is_empty(), "{errs:?}");
        let block = &report.roots[0].children[0];
        assert!(
            block.bindings.iter().any(|b| {
                b.name == "s" && matches!(b.ownership, Ownership::Shared { .. })
            }),
            "{:?}",
            block.bindings
        );
    }

    #[test]
    fn dump_annotates_compacted_braces() {
        let src = r#"
fun main() {
    val s: String = "hi"
    {
        {
            move (s) {
                return 1
            }
        }
    }
}
"#;
        let prog = parse(src).unwrap();
        let (report, _) = analyze(&prog);
        let text = dump_arenas(&report);
        assert!(
            text.contains("compacted"),
            "expected compaction annotation in:\n{text}"
        );
    }

    #[test]
    fn copy_crossing_recorded_in_child() {
        let src = r#"
fun main() {
    val n = 1
    {
        val m = n
    }
}
"#;
        let prog = parse(src).unwrap();
        let (report, errs) = analyze(&prog);
        assert!(errs.is_empty(), "{errs:?}");
        let block = &report.roots[0].children[0];
        assert!(block.bindings.iter().any(|b| {
            b.name == "n" && matches!(b.ownership, Ownership::Copy)
        }));
    }

    #[test]
    fn inferred_move_captures_free_parent() {
        let src = r#"
fun main() {
    val s: String = "hi"
    move {
        val t = s
    }
    val u = s
}
"#;
        let prog = parse(src).unwrap();
        let (_, errs) = analyze(&prog);
        assert!(
            errs.iter().any(|e| e.message.contains("after move")),
            "{errs:?}"
        );
    }

    #[test]
    fn explicit_empty_captures_do_not_infer() {
        let src = r#"
fun main() {
    val s: String = "hi"
    move () {
        val t = s
    }
    val u = s
}
"#;
        let prog = parse(src).unwrap();
        let (report, errs) = analyze(&prog);
        assert!(errs.is_empty(), "empty capture list must not move s: {errs:?}");
        let move_child = report.roots[0]
            .children
            .iter()
            .find(|c| c.label.contains("MoveBlock"))
            .expect("move child");
        assert!(
            !move_child.bindings.iter().any(|b| {
                b.name == "s" && matches!(b.ownership, Ownership::Moved { .. })
            }),
            "s must not be moved with explicit empty captures: {:?}",
            move_child.bindings
        );
        assert!(
            move_child.bindings.iter().any(|b| {
                b.name == "s" && matches!(b.ownership, Ownership::Shared { .. })
            }),
            "s should be Shared inside move (): {:?}",
            move_child.bindings
        );
    }

    #[test]
    fn nested_var_shadow_restores_parent() {
        let src = r#"
fun main() {
    val s: String = "hi"
    {
        val s: String = "inner"
    }
    move {
        val t = s
    }
    val u = s
}
"#;
        let prog = parse(src).unwrap();
        let (_, errs) = analyze(&prog);
        assert!(
            errs.iter().any(|e| e.message.contains("after move")),
            "parent s should still be capturable after inner shadow ends: {errs:?}"
        );
    }

    #[test]
    fn if_branches_do_not_share_move_state() {
        let src = r#"
fun main() {
    var s: String = "hi"
    if (true) {
        move (s) {
            return 1
        }
    } else {
        val t = s
    }
}
"#;
        let prog = parse(src).unwrap();
        let (_, errs) = analyze(&prog);
        assert!(
            !errs.iter().any(|e| e.message.contains("after move")),
            "else must not see then-branch move: {errs:?}"
        );
        assert!(
            errs.iter().any(|e| e.message.contains("not Copy")),
            "else reading var s without move should error: {errs:?}"
        );
    }

    #[test]
    fn unknown_type_is_not_treated_as_copy() {
        let src = r#"
fun main() {
    var s: String = "hi"
    var t = s
    {
        val u = t
    }
}
"#;
        let prog = parse(src).unwrap();
        let (_, errs) = analyze(&prog);
        assert!(
            errs.iter().any(|e| e.message.contains("not Copy")),
            "t should inherit non-Copy from s: {errs:?}"
        );
    }
}
