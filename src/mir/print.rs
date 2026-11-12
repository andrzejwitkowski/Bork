//! Stable textual form of a [`Body`] (RFC 0006 3.9). Spans are left out so goldens survive edits to
//! unrelated source text; debug labels are printed inline.

use std::fmt::Write;

use super::body::*;
use crate::ty::TyCtx;

pub fn print_body(body: &Body, tcx: &TyCtx) -> String {
    let mut out = String::new();
    let abi = body.abi;
    let _ = writeln!(out, "fn {} (parent={}, sink={}) {{", body.name, abi.caller_parent, abi.result_sink);
    for (i, l) in body.locals.iter().enumerate() {
        let name = l.name.as_deref().map(|n| format!(" {n}")).unwrap_or_default();
        let _ = writeln!(out, "  let _{i}: {} [{:?}]{name}", tcx.display(l.ty), l.kind);
    }
    for (i, r) in body.regions.iter().enumerate() {
        let parent = r.parent.map(|p| format!(" in r{}", p.0)).unwrap_or_default();
        let _ = writeln!(out, "  region r{i} {:?}{parent}", r.site);
    }
    for (i, b) in body.blocks.iter().enumerate() {
        let _ = writeln!(out, "  bb{i}:");
        for s in &b.stmts {
            let label = s.debug.as_ref().map(|d| format!("  // {}", label(d, tcx))).unwrap_or_default();
            let _ = writeln!(out, "    {}{label}", stmt(&s.kind, tcx));
        }
        match &b.term {
            Some(t) => {
                let _ = writeln!(out, "    {}", term(&t.kind));
            }
            None => {
                let _ = writeln!(out, "    <no terminator>");
            }
        }
    }
    out.push_str("}\n");
    out
}

fn label(d: &DebugLabel, tcx: &TyCtx) -> String {
    let mut s = format!("label {:?}: {}", d.name, tcx.display(d.ty));
    if d.declaration {
        s.push_str(" decl");
    }
    if let Some(a) = d.argument {
        let _ = write!(s, " arg{a}");
    }
    if let Some(o) = &d.owner {
        let _ = write!(s, " owner={}", arena(o));
    }
    if let Some(i) = &d.index {
        let _ = write!(s, " index={}", op(i));
    }
    s
}

fn place(p: &Place) -> String {
    let mut s = format!("_{}", p.local.0);
    for pr in &p.proj {
        match pr {
            Proj::Field(i) => { let _ = write!(s, ".{i}"); }
            Proj::Index(o) => { let _ = write!(s, "[{}]", op(o)); }
            Proj::Deref => s.push_str(".*"),
            Proj::Variant(i) => { let _ = write!(s, "@{i}"); }
            Proj::Some => s.push_str(".some"),
        }
    }
    s
}

fn op(o: &Operand) -> String {
    match o {
        Operand::Copy(p) => format!("copy {}", place(p)),
        Operand::Move(p) => format!("move {}", place(p)),
        Operand::Reborrow(p) => format!("reborrow {}", place(p)),
        Operand::Const(c) => match c {
            Const::Int(v) => v.to_string(),
            Const::Float(v) => format!("{v:?}"),
            Const::Bool(v) => v.to_string(),
            Const::Str(v) => format!("{v:?}"),
            Const::Unit => "unit".into(),
            Const::None => "none".into(),
        },
    }
}

fn ops(v: &[Operand]) -> String {
    v.iter().map(op).collect::<Vec<_>>().join(", ")
}

fn arena(a: &ArenaRef) -> String {
    match a {
        ArenaRef::Current => "current".into(),
        ArenaRef::Region(r) => format!("r{}", r.0),
        ArenaRef::Home(l) => format!("home(_{})", l.0),
        ArenaRef::ResultSink => "result_sink".into(),
        ArenaRef::CallerParent => "caller_parent".into(),
        ArenaRef::ObjectArena(l) => format!("object_arena(_{})", l.0),
        ArenaRef::Fresh(n) => format!("fresh{n}"),
        ArenaRef::Root => "root".into(),
    }
}

fn rvalue(r: &Rvalue, tcx: &TyCtx) -> String {
    match r {
        Rvalue::Use(a) => op(a),
        Rvalue::Binary(b, l, r) => format!("{}({}, {})", binop(b), op(l), op(r)),
        Rvalue::Unary(u, a) => format!("{u:?}({})", op(a)),
        Rvalue::Cast(a, t) => format!("cast({}, {})", op(a), tcx.display(*t)),
        Rvalue::Aggregate(k, f) => format!("{k:?}{{{}}}", ops(f)),
        Rvalue::Len(p) => format!("len({})", place(p)),
        Rvalue::Slice { base, lo, hi } => format!("slice({}, {}, {})", op(base), op(lo), op(hi)),
    }
}

fn stmt(k: &StmtKind, tcx: &TyCtx) -> String {
    match k {
        StmtKind::Assign(d, r) => format!("{} = {}", place(d), rvalue(r, tcx)),
        StmtKind::Alloc { dst, kind, arena: a } => format!("{} = alloc {kind:?} in {}", place(dst), arena(a)),
        StmtKind::ArenaCreateDynamic { dst, parent, scoped } => {
            format!("_{} = arena_create_dynamic parent={} scoped={scoped}", dst.0, arena(parent))
        }
        StmtKind::RegisterStrongRoot { owner, arena: a } => format!("register_strong_root {} <- _{}", arena(owner), a.0),
        StmtKind::CopyInto { dst, src, arena: a } => format!("{} = copy_into {} in {}", place(dst), place(src), arena(a)),
        StmtKind::Borrow { dst, src, rw } => format!("{} = borrow{} {}", place(dst), if *rw { "_rw" } else { "" }, place(src)),
        StmtKind::Move { dst, src } => format!("{} = move {}", place(dst), place(src)),
        StmtKind::Promote { dst, src, arena: a } => format!("{} = promote {} in {}", place(dst), place(src), arena(a)),
        StmtKind::Ref(r) => refop(r),
        StmtKind::Observe { obs, object, absent, value } => {
            format!("(_{}, _{}, _{}) = observe {}", obs.0, object.0, absent.0, op(value))
        }
        StmtKind::ObservationRegister { arena: a, obs } => format!("observation_register {} _{}", arena(a), obs.0),
        StmtKind::ObservationRelease(l) => format!("observation_release _{}", l.0),
        StmtKind::RegisterDrop { arena: a, place: p, glue } => format!("register_drop {} {} {glue:?}", arena(a), place(p)),
        StmtKind::RegionEnter(r) => format!("region_enter r{}", r.0),
        StmtKind::RegionExit(r) => format!("region_exit r{}", r.0),
        StmtKind::RegionReset { region, dst } => format!("_{} = region_reset r{}", dst.0, region.0),
        StmtKind::VecOp { dst, op: o, receiver, args } => {
            format!("{}vec_{o:?}({}{})", dst_prefix(dst), op(receiver), args.iter().map(|a| format!(", {}", op(a))).collect::<String>())
        }
        StmtKind::Intrinsic { dst, op: o, args, arena: a } => {
            let a = a.as_ref().map(|a| format!(" in {}", arena(a))).unwrap_or_default();
            format!("{}intrinsic {o:?}({}){a}", dst_prefix(dst), ops(args))
        }
        StmtKind::StorageLive(l) => format!("storage_live _{}", l.0),
        StmtKind::StorageDead(l) => format!("storage_dead _{}", l.0),
    }
}

fn dst_prefix(d: &Option<Place>) -> String {
    d.as_ref().map(|p| format!("{} = ", place(p))).unwrap_or_default()
}

fn refop(r: &RefOp) -> String {
    match r {
        RefOp::Create { dst, source, target, object } => {
            format!("{} = ref_create source={} target={} {}", place(dst), arena(source), arena(target), op(object))
        }
        RefOp::Clone { dst, src } => format!("{} = ref_clone {}", place(dst), op(src)),
        RefOp::MoveOut { dst, src } => format!("{} = ref_move_out {}", place(dst), place(src)),
        RefOp::Store { slot, owner, value, fresh } => format!("ref_store{} {} owner={} {}", if *fresh { "_fresh" } else { "" }, place(slot), arena(owner), op(value)),
        RefOp::Rehome { dst, owner, value } => format!("{} = ref_rehome owner={} {}", place(dst), arena(owner), op(value)),
        RefOp::Drop(v) => format!("ref_drop {}", op(v)),
        RefOp::Eq { dst, lhs, rhs } => format!("{} = ref_eq {}, {}", place(dst), op(lhs), op(rhs)),
        RefOp::Count { dst, value, weak } => format!("{} = ref_count{} {}", place(dst), if *weak { "_weak" } else { "" }, op(value)),
        RefOp::IsNone { dst, value } => format!("{} = ref_is_none {}", place(dst), op(value)),
    }
}

fn term(k: &TermKind) -> String {
    match k {
        TermKind::Goto(b) => format!("goto bb{}", b.0),
        TermKind::Switch { value, cases, otherwise } => {
            let cs: String = cases.iter().map(|(v, b)| format!(" {v} -> bb{},", b.0)).collect();
            format!("switch {} [{cs} else -> bb{}]", op(value), otherwise.0)
        }
        TermKind::Match { scrutinee, arms } => {
            let arms: String = arms
                .iter()
                .map(|a| {
                    let binds: String = a.bindings.iter().map(|(l, p)| format!(" _{}={}", l.0, place(p))).collect();
                    format!("\n      {}{binds} -> bb{}", pat(&a.pat), a.target.0)
                })
                .collect();
            format!("match {} {{{arms}\n    }}", place(scrutinee))
        }
        TermKind::Call { callee, args, hidden, dest, target, ret_borrows } => {
            let h: String = hidden.iter().map(|a| format!(" +{}", arena(a))).collect();
            let t = target.map(|b| format!(" -> bb{}", b.0)).unwrap_or_default();
            let rb = if ret_borrows.is_empty() { String::new() } else { format!(" borrows={ret_borrows:?}") };
            format!("{} = call {callee}({}){h}{t}{rb}", place(dest), ops(args))
        }
        TermKind::Return => "return".into(),
        TermKind::Unreachable => "unreachable".into(),
        TermKind::Abort(r) => format!("abort {r:?}"),
    }
}

/// Spelled out so goldens do not depend on the AST's `Debug` output.
fn binop(b: &crate::ast::BinOp) -> &'static str {
    use crate::ast::BinOp::*;
    match b {
        Add => "add",
        Sub => "sub",
        Mul => "mul",
        Div => "div",
        Gt => "gt",
        Lt => "lt",
        Ge => "ge",
        Le => "le",
        Eq => "eq",
        Ne => "ne",
        RangeTo => "range_to",
        Elvis => "elvis",
        And => "and",
        Or => "or",
    }
}

fn pat(p: &MirPat) -> String {
    match p {
        MirPat::Wildcard => "_".into(),
        MirPat::Bind => "bind".into(),
        MirPat::Lit(crate::ast::PatLit::Int(v)) => v.to_string(),
        MirPat::Lit(crate::ast::PatLit::Bool(v)) => v.to_string(),
        MirPat::Lit(crate::ast::PatLit::Str(v)) => format!("{v:?}"),
        MirPat::Variant { enum_name, index, fields } => {
            format!("{enum_name}#{index}({})", fields.iter().map(pat).collect::<Vec<_>>().join(", "))
        }
        MirPat::Some(inner) => format!("Some({})", pat(inner)),
        MirPat::None => "None".into(),
    }
}
