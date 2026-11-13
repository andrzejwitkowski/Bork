use super::body::*;
use super::capability::Capability;
use super::print::print_body;
use super::verify::verify;
use crate::hir::Prim;
use crate::region_walk::RegionSite;
use crate::span::Span;
use crate::ty::{TyCtx, TyId};

struct Env {
    tcx: TyCtx,
    int: TyId,
    unit: TyId,
    handle: TyId,
}

fn env() -> Env {
    let mut tcx = TyCtx::new(&Default::default());
    let int = tcx.prim_id(Prim::I32);
    let unit = tcx.prim_id(Prim::Unit);
    let handle = tcx.managed_ref(int);
    Env { tcx, int, unit, handle }
}

fn stmt(kind: StmtKind) -> Statement {
    Statement { span: Span::default(), node: None, kind, debug: None, region: RegionId(0) }
}

fn term(kind: TermKind) -> Option<Terminator> {
    Some(Terminator { span: Span::default(), node: None, kind, region: RegionId(0) })
}

fn local(ty: TyId, kind: LocalKind) -> LocalDecl {
    LocalDecl { ty, kind, name: None, region: RegionId(0) }
}

fn p(n: u32) -> Place {
    Place::local(Local(n))
}

/// `_0: i32` (return), `_1: Ref<i32>` temp, `_2`: region arena; one region `r0`; one block.
fn body(e: &Env, stmts: Vec<StmtKind>, t: TermKind) -> Body {
    Body {
        name: "f".into(),
        locals: vec![
            local(e.int, LocalKind::Return),
            local(e.handle, LocalKind::Temp),
            local(e.unit, LocalKind::Hidden(HiddenKind::RegionArena)),
        ],
        blocks: vec![Block { stmts: stmts.into_iter().map(stmt).collect(), term: term(t) }],
        regions: vec![RegionInfo { parent: None, site: RegionSite::Block, arena: 0 }],
        abi: CallAbi::default(),
        ret: Local(0),
    }
}

fn ref_create(dst: u32) -> StmtKind {
    StmtKind::Ref(RefOp::Create {
        dst: p(dst),
        source: ArenaRef::Current,
        target: ArenaRef::Root,
        object: Operand::Const(Const::Int(0)),
    })
}

fn messages(e: &Env, b: &Body) -> Vec<String> {
    verify(b, &e.tcx).into_iter().map(|e| e.message).collect()
}

fn has(msgs: &[String], needle: &str) {
    assert!(msgs.iter().any(|m| m.contains(needle)), "expected `{needle}` in {msgs:?}");
}

#[test]
fn valid_body_with_region_and_consumed_ref_passes() {
    let e = env();
    let b = body(
        &e,
        vec![
            StmtKind::RegionEnter(RegionId(0)),
            ref_create(1),
            StmtKind::Ref(RefOp::Drop(Operand::Move(p(1)))),
            StmtKind::Assign(p(0), Rvalue::Use(Operand::Const(Const::Int(7)))),
            StmtKind::RegionExit(RegionId(0)),
        ],
        TermKind::Return,
    );
    assert_eq!(messages(&e, &b), Vec::<String>::new());
}

#[test]
fn loop_with_reset_on_the_back_edge_passes() {
    let e = env();
    let mut b = body(&e, vec![StmtKind::RegionEnter(RegionId(0))], TermKind::Goto(BlockId(1)));
    b.blocks.push(Block {
        stmts: vec![stmt(StmtKind::RegionReset { region: RegionId(0), dst: Local(2) })],
        term: term(TermKind::Switch { value: Operand::Const(Const::Bool(true)), cases: vec![(1, BlockId(1))], otherwise: BlockId(2) }),
    });
    b.blocks.push(Block { stmts: vec![stmt(StmtKind::RegionExit(RegionId(0)))], term: term(TermKind::Return) });
    assert_eq!(messages(&e, &b), Vec::<String>::new());
}

#[test]
fn rejects_missing_terminator() {
    let e = env();
    let mut b = body(&e, vec![], TermKind::Return);
    b.blocks[0].term = None;
    has(&messages(&e, &b), "no terminator");
}

#[test]
fn rejects_jump_to_missing_block_and_missing_local() {
    let e = env();
    has(&messages(&e, &body(&e, vec![], TermKind::Goto(BlockId(9)))), "missing bb9");
    let b = body(&e, vec![StmtKind::Assign(p(0), Rvalue::Use(Operand::Copy(p(42))))], TermKind::Return);
    has(&messages(&e, &b), "missing local _42");
}

#[test]
fn rejects_unbalanced_regions() {
    let e = env();
    has(&messages(&e, &body(&e, vec![StmtKind::RegionEnter(RegionId(0))], TermKind::Return)), "leaves regions open");
    has(&messages(&e, &body(&e, vec![StmtKind::RegionExit(RegionId(0))], TermKind::Return)), "innermost open region");
}

#[test]
fn rejects_hidden_arena_without_abi() {
    let e = env();
    let b = body(
        &e,
        vec![StmtKind::Alloc { dst: p(0), kind: AllocKind::String, arena: ArenaRef::ResultSink }],
        TermKind::Return,
    );
    has(&messages(&e, &b), "no result sink");
    let mut ok = b.clone();
    ok.abi.result_sink = true;
    assert_eq!(messages(&e, &ok), Vec::<String>::new());
}

#[test]
fn rejects_ref_temporary_consumed_twice_or_leaked() {
    let e = env();
    let twice = body(
        &e,
        vec![ref_create(1), StmtKind::Ref(RefOp::Drop(Operand::Move(p(1)))), StmtKind::Ref(RefOp::Drop(Operand::Move(p(1))))],
        TermKind::Return,
    );
    has(&messages(&e, &twice), "not live");
    has(&messages(&e, &body(&e, vec![ref_create(1)], TermKind::Return)), "unconsumed Ref temporaries");
}

#[test]
fn rejects_branches_that_disagree_about_open_regions() {
    let e = env();
    let mut b = body(&e, vec![], TermKind::Switch { value: Operand::Const(Const::Bool(true)), cases: vec![(1, BlockId(1))], otherwise: BlockId(2) });
    b.blocks.push(Block { stmts: vec![stmt(StmtKind::RegionEnter(RegionId(0)))], term: term(TermKind::Goto(BlockId(3))) });
    b.blocks.push(Block { stmts: vec![], term: term(TermKind::Goto(BlockId(3))) });
    b.blocks.push(Block { stmts: vec![], term: term(TermKind::Unreachable) });
    has(&messages(&e, &b), "different open regions");
}

#[test]
fn prints_a_stable_form() {
    let e = env();
    let b = body(
        &e,
        vec![StmtKind::RegionEnter(RegionId(0)), ref_create(1), StmtKind::Ref(RefOp::Drop(Operand::Move(p(1)))), StmtKind::RegionExit(RegionId(0))],
        TermKind::Return,
    );
    assert_eq!(
        print_body(&b, &e.tcx),
        "fn f (parent=false, sink=false) {\n  let _0: i32 [Return]\n  let _1: Ref<i32> [Temp]\n  let _2: unit [Hidden(RegionArena)]\n  region r0 Block\n  bb0:\n    region_enter r0\n    _1 = ref_create source=current target=root 0\n    ref_drop move _1\n    region_exit r0\n    return\n}\n"
    );
}

#[test]
fn capability_ids_match_rfc_3_10() {
    let ids: Vec<_> = Capability::ALL.iter().map(|c| c.to_string()).collect();
    assert_eq!(
        ids,
        ["closure", "match-ref", "pat-presence", "move-aggregate", "field-access", "safe-field-enum", "indirect-call", "unit-elem", "unit-payload-bind", "object-equality"]
    );
}

#[test]
fn rejects_match_binding_to_missing_local() {
    let e = env();
    let arm = MirArm { pat: MirPat::Wildcard, bindings: vec![(Local(77), p(0))], target: BlockId(0) };
    let b = body(&e, vec![], TermKind::Match { scrutinee: p(0), arms: vec![arm] });
    has(&messages(&e, &b), "missing local _77");
}
