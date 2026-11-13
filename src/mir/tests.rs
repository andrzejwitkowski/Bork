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
    b.blocks.push(Block { stmts: vec![stmt(set(0, 0)), stmt(StmtKind::RegionExit(RegionId(0)))], term: term(TermKind::Return) });
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
        ["closure", "match-ref", "pat-presence", "move-aggregate", "field-access", "safe-field-enum", "indirect-call", "unit-elem", "unit-payload-bind", "object-equality", "unit-param", "unit-field", "main-signature"]
    );
}

#[test]
fn rejects_match_binding_to_missing_local() {
    let e = env();
    let arm = MirArm { pat: MirPat::Wildcard, bindings: vec![(Local(77), p(0))], target: BlockId(0) };
    let b = body(&e, vec![], TermKind::Match { scrutinee: p(0), arms: vec![arm] });
    has(&messages(&e, &b), "missing local _77");
}

/// `body` plus two `DebugScope` locals `_3`, `_4`.
fn scoped(e: &Env, stmts: Vec<StmtKind>, t: TermKind) -> Body {
    let mut b = body(e, stmts, t);
    b.locals.push(local(e.unit, LocalKind::Hidden(HiddenKind::DebugScope)));
    b.locals.push(local(e.unit, LocalKind::Hidden(HiddenKind::DebugScope)));
    b
}

#[test]
fn balanced_debug_scopes_pass() {
    let e = env();
    let b = scoped(&e, vec![set(0, 0), StmtKind::StorageLive(Local(3)), StmtKind::StorageLive(Local(4)), StmtKind::StorageDead(Local(4)), StmtKind::StorageDead(Local(3))], TermKind::Return);
    assert_eq!(messages(&e, &b), Vec::<String>::new());
}

#[test]
fn rejects_unbalanced_debug_scopes() {
    let e = env();
    has(&messages(&e, &scoped(&e, vec![StmtKind::StorageLive(Local(3))], TermKind::Return)), "leaves debug scopes live");
    has(&messages(&e, &scoped(&e, vec![StmtKind::StorageDead(Local(3))], TermKind::Return)), "innermost live debug scope");
    let crossed = scoped(&e, vec![StmtKind::StorageLive(Local(3)), StmtKind::StorageLive(Local(4)), StmtKind::StorageDead(Local(3))], TermKind::Return);
    has(&messages(&e, &crossed), "innermost live debug scope");
    has(&messages(&e, &scoped(&e, vec![StmtKind::StorageLive(Local(3)), StmtKind::StorageLive(Local(3))], TermKind::Return)), "already live");
    has(&messages(&e, &scoped(&e, vec![StmtKind::StorageLive(Local(1))], TermKind::Return)), "not a DebugScope");
}

#[test]
fn rejects_paths_that_disagree_about_live_debug_scopes() {
    let e = env();
    let mut b = scoped(&e, vec![], TermKind::Switch { value: Operand::Const(Const::Bool(true)), cases: vec![(1, BlockId(1))], otherwise: BlockId(2) });
    b.blocks.push(Block { stmts: vec![stmt(StmtKind::StorageLive(Local(3)))], term: term(TermKind::Goto(BlockId(3))) });
    b.blocks.push(Block { stmts: vec![], term: term(TermKind::Goto(BlockId(3))) });
    b.blocks.push(Block { stmts: vec![], term: term(TermKind::Abort(AbortReason::Bounds)) });
    has(&messages(&e, &b), "different live debug scopes");
}

#[test]
fn an_arena_must_exist_where_an_op_names_it() {
    let e = env();
    // `_0` is the `i32` return local: not a `Ref` temporary, so the allocation leaves nothing to consume.
    let alloc = |arena: ArenaRef| vec![StmtKind::Alloc { dst: p(0), kind: AllocKind::String, arena }];
    // `r0` is not entered yet.
    has(&messages(&e, &body(&e, alloc(ArenaRef::Region(RegionId(0))), TermKind::Return)), "not open here");
    // No open region and no caller parent: `current` names nothing.
    has(&messages(&e, &body(&e, alloc(ArenaRef::Current), TermKind::Return)), "no open region and no caller parent");
    // The local is a plain temporary, not a dynamic arena.
    has(&messages(&e, &body(&e, alloc(ArenaRef::Fresh(1)), TermKind::Return)), "not a DynamicArena");
    let mut with_parent = body(&e, [alloc(ArenaRef::Current), vec![set(0, 0)]].concat(), TermKind::Return);
    with_parent.abi.caller_parent = true;
    assert_eq!(messages(&e, &with_parent), Vec::<String>::new());
    let open = body(&e, [vec![StmtKind::RegionEnter(RegionId(0))], alloc(ArenaRef::Region(RegionId(0))), vec![set(0, 0), StmtKind::RegionExit(RegionId(0))]].concat(), TermKind::Return);
    assert_eq!(messages(&e, &open), Vec::<String>::new());
}

/// `_3: i32` and `_4: i32` plain temporaries on top of `body`.
fn with_ints(e: &Env, stmts: Vec<StmtKind>, t: TermKind) -> Body {
    let mut b = body(e, stmts, t);
    b.locals.push(local(e.int, LocalKind::Temp));
    b.locals.push(local(e.int, LocalKind::Temp));
    b
}

fn set(dst: u32, v: i64) -> StmtKind {
    StmtKind::Assign(p(dst), Rvalue::Use(Operand::Const(Const::Int(v))))
}

fn mv(dst: u32, src: u32) -> StmtKind {
    StmtKind::Assign(p(dst), Rvalue::Use(Operand::Move(p(src))))
}

fn cp(dst: u32, src: u32) -> StmtKind {
    StmtKind::Assign(p(dst), Rvalue::Use(Operand::Copy(p(src))))
}

#[test]
fn rejects_a_read_before_assignment_and_after_a_move() {
    let e = env();
    has(&messages(&e, &with_ints(&e, vec![cp(0, 3)], TermKind::Return)), "_3 is read before it was assigned");
    has(&messages(&e, &with_ints(&e, vec![set(3, 1), mv(4, 3), cp(0, 3)], TermKind::Return)), "_3 is read after it was moved");
    // Assigning again makes it readable.
    assert_eq!(messages(&e, &with_ints(&e, vec![set(3, 1), mv(4, 3), set(3, 2), cp(0, 3)], TermKind::Return)), Vec::<String>::new());
    // The return local must be assigned on every path.
    has(&messages(&e, &with_ints(&e, vec![], TermKind::Return)), "not assigned on every path");
}

/// bb0 branches to bb1 (runs `then`) or bb2 (nothing); both join at bb3, which runs `after`.
fn diamond(e: &Env, then: Vec<StmtKind>, after: Vec<StmtKind>) -> Body {
    let mut b = with_ints(e, vec![set(3, 1)], TermKind::Switch { value: Operand::Const(Const::Bool(true)), cases: vec![(1, BlockId(1))], otherwise: BlockId(2) });
    b.blocks.push(Block { stmts: then.into_iter().map(stmt).collect(), term: term(TermKind::Goto(BlockId(3))) });
    b.blocks.push(Block { stmts: vec![], term: term(TermKind::Goto(BlockId(3))) });
    b.blocks.push(Block { stmts: after.into_iter().map(stmt).collect(), term: term(TermKind::Return) });
    b
}

#[test]
fn a_conditional_move_is_only_an_error_when_read_afterwards() {
    let e = env();
    // Moved on one branch, never read again: fine.
    assert_eq!(messages(&e, &diamond(&e, vec![mv(4, 3)], vec![set(0, 0)])), Vec::<String>::new());
    // Moved on one branch, read after the join: maybe-moved.
    has(&messages(&e, &diamond(&e, vec![mv(4, 3)], vec![cp(0, 3)])), "_3 is read after it was moved");
    // Assigned on one branch only, read after the join: not definitely assigned.
    let mut b = diamond(&e, vec![set(4, 2)], vec![cp(0, 4)]);
    b.blocks[0].stmts.clear();
    has(&messages(&e, &b), "_4 is read before it was assigned");
}

#[test]
fn unreachable_blocks_are_not_analysed() {
    let e = env();
    let mut b = with_ints(&e, vec![set(0, 1)], TermKind::Return);
    b.blocks.push(Block { stmts: vec![stmt(cp(0, 3))], term: term(TermKind::Return) });
    assert_eq!(messages(&e, &b), Vec::<String>::new());
}
