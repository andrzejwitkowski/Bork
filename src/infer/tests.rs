use super::*;
use crate::defs::DefTable;

fn tcx() -> (TyCtx, DefTable) {
    let program = crate::parse("class Point {\n x: i32\n}\nenum Option<T> {\n Some(value: T)\n None\n}\nfun main() {}\n").unwrap();
    let defs = DefTable::build(&program);
    (TyCtx::new(&defs), defs)
}

struct Fx {
    t: TyCtx,
    i: InferCtxt,
    defs: DefTable,
}

impl Fx {
    fn new() -> Fx {
        let (t, defs) = tcx();
        Fx { t, i: InferCtxt::new(), defs }
    }
    fn var(&mut self) -> TyId {
        self.i.new_var(&mut self.t, VarKind::General)
    }
    fn kinded(&mut self, kind: VarKind) -> TyId {
        self.i.new_var(&mut self.t, kind)
    }
    fn unify(&mut self, a: TyId, b: TyId) -> Result<(), TypeError> {
        self.i.unify(&mut self.t, a, b)
    }
    fn show(&mut self, ty: TyId) -> String {
        let r = self.i.resolve(&mut self.t, ty).unwrap();
        self.t.display(r).to_string()
    }
}

#[test]
fn var_binds_to_a_type_and_resolves() {
    let mut f = Fx::new();
    let (v, i32_ty) = (f.var(), f.t.i32());
    f.unify(v, i32_ty).unwrap();
    assert_eq!(f.show(v), "i32");
    let (w, i64_ty) = (f.var(), f.t.prim(Prim::I64));
    f.unify(w, v).unwrap();
    assert_eq!(f.show(w), "i32");
    assert!(matches!(f.unify(w, i64_ty), Err(TypeError::Mismatch { .. })));
}

#[test]
fn var_var_links_share_a_later_binding() {
    let mut f = Fx::new();
    let (a, b, c) = (f.var(), f.var(), f.var());
    f.unify(a, b).unwrap();
    f.unify(b, c).unwrap();
    let s = f.t.string(false);
    f.unify(c, s).unwrap();
    assert_eq!((f.show(a), f.show(b)), ("String".into(), "String".into()));
    assert_eq!(f.i.resolve_shallow(&mut f.t, a), s);
}

#[test]
fn structural_cases_unify_inner_types() {
    let mut f = Fx::new();
    let (i32_ty, bool_ty) = (f.t.i32(), f.t.bool());
    let point = f.t.class(f.defs.lookup_class("Point").unwrap(), Vec::new());
    type Make = fn(&mut TyCtx, TyId) -> TyId;
    let makers: [(&str, Make); 6] = [
        ("[{}; 3]", |t, x| t.array(x, 3)),
        ("&{}", |t, x| t.ref_to(x)),
        ("Vec<{}>", |t, x| t.vec_of(x)),
        ("Range<{}>", |t, x| t.range(x)),
        ("{}?", |t, x| t.nullable(x).unwrap()),
        ("(i32)->{}", |t, x| {
            let i = t.i32();
            t.func(vec![i], x)
        }),
    ];
    for (shape, make) in makers {
        let v = f.var();
        let (with_var, with_i32, with_bool) = (make(&mut f.t, v), make(&mut f.t, i32_ty), make(&mut f.t, bool_ty));
        f.unify(with_var, with_i32).unwrap();
        assert_eq!(f.show(v), "i32", "{shape}");
        assert!(matches!(f.unify(with_var, with_bool), Err(TypeError::Mismatch { .. })), "{shape}");
    }
    let m = f.t.managed_ref(point);
    let v = f.var();
    let mv = f.t.managed_ref(v);
    f.unify(mv, m).unwrap();
    assert_eq!(f.show(v), "Point");
    let r = f.t.ref_to(point);
    assert!(matches!(f.unify(m, r), Err(TypeError::Mismatch { .. })));
}

#[test]
fn array_length_and_function_arity_are_checked() {
    let mut f = Fx::new();
    let i32_ty = f.t.i32();
    let (a2, a3) = (f.t.array(i32_ty, 2), f.t.array(i32_ty, 3));
    assert!(matches!(f.unify(a2, a3), Err(TypeError::Mismatch { .. })));
    let (f1, f2) = (f.t.func(vec![i32_ty], i32_ty), f.t.func(vec![i32_ty, i32_ty], i32_ty));
    assert_eq!(f.unify(f1, f2), Err(TypeError::Arity { expected: 1, found: 2 }));
}

#[test]
fn nullable_does_not_unify_with_its_base_but_a_var_binds_to_it() {
    let mut f = Fx::new();
    let i32_ty = f.t.i32();
    let opt = f.t.nullable(i32_ty).unwrap();
    assert!(matches!(f.unify(opt, i32_ty), Err(TypeError::Mismatch { .. })));
    let v = f.var();
    f.unify(v, opt).unwrap();
    assert_eq!(f.show(v), "i32?");
    let w = f.var();
    let wopt = f.t.nullable(w).unwrap();
    f.unify(wopt, opt).unwrap();
    assert_eq!(f.show(w), "i32");
}

#[test]
fn resolving_into_a_nullable_nests_now_that_t_question_is_option() {
    let mut f = Fx::new();
    let (v, i32_ty) = (f.var(), f.t.i32());
    let opt_v = f.t.nullable(v).unwrap();
    let opt_i = f.t.nullable(i32_ty).unwrap();
    f.unify(v, opt_i).unwrap();
    let nested = f.t.nullable(opt_i).unwrap();
    assert_eq!(f.i.resolve(&mut f.t, opt_v), Ok(nested));
}

#[test]
fn occurs_check_rejects_infinite_types() {
    let mut f = Fx::new();
    let v = f.var();
    let arr = f.t.array(v, 3);
    assert!(matches!(f.unify(v, arr), Err(TypeError::Occurs { .. })));
    // indirect: ?A = Vec<?B>, then ?B = &?A
    let (a, b) = (f.var(), f.var());
    let vec_b = f.t.vec_of(b);
    f.unify(a, vec_b).unwrap();
    let ref_a = f.t.ref_to(a);
    let err = f.unify(b, ref_a).unwrap_err();
    assert!(matches!(err, TypeError::Occurs { .. }));
    assert!(err.describe(&f.t).contains("infinite"));
    // the failed unify left `b` unbound
    let shallow = f.i.resolve_shallow(&mut f.t, b);
    assert!(matches!(f.t.data(shallow), TyData::Infer(_)));
    // sharing a variable without a cycle is fine
    let (c, i32_ty) = (f.var(), f.t.i32());
    let pair = f.t.func(vec![c], c);
    let want = f.t.func(vec![i32_ty], i32_ty);
    f.unify(pair, want).unwrap();
    assert_eq!(f.show(c), "i32");
}

#[test]
fn literal_kinds_restrict_what_a_variable_can_become() {
    let mut f = Fx::new();
    let (u8_ty, f64_ty, string, bool_ty) = (f.t.prim(Prim::U8), f.t.prim(Prim::F64), f.t.string(false), f.t.bool());
    let int = f.kinded(VarKind::Int);
    for bad in [f64_ty, string, bool_ty] {
        assert!(matches!(f.unify(int, bad), Err(TypeError::Kind { kind: VarKind::Int, .. })));
    }
    let opt_u8 = f.t.nullable(u8_ty).unwrap();
    assert!(matches!(f.unify(int, opt_u8), Err(TypeError::Kind { .. })));
    f.unify(int, u8_ty).unwrap();
    assert_eq!(f.show(int), "u8");
    let float = f.kinded(VarKind::Float);
    let i32_ty = f.t.i32();
    assert!(matches!(f.unify(float, i32_ty), Err(TypeError::Kind { kind: VarKind::Float, .. })));
    f.unify(float, f64_ty).unwrap();
    // classes join; a general var adopts the literal kind
    let (int2, float2, general) = (f.kinded(VarKind::Int), f.kinded(VarKind::Float), f.var());
    assert!(matches!(f.unify(int2, float2), Err(TypeError::Kind { .. })));
    f.unify(general, int2).unwrap();
    assert_eq!(f.i.kind_of(InferVar(general_index(&f.t, general))), VarKind::Int);
    assert!(matches!(f.unify(general, f64_ty), Err(TypeError::Kind { .. })));
}

fn general_index(tcx: &TyCtx, ty: TyId) -> u32 {
    match tcx.data(ty) {
        TyData::Infer(v) => v.0,
        other => panic!("{other:?}"),
    }
}

#[test]
fn error_type_absorbs_without_cascading() {
    let mut f = Fx::new();
    let (err, i32_ty, v) = (f.t.error(), f.t.i32(), f.var());
    f.unify(err, i32_ty).unwrap();
    f.unify(v, err).unwrap();
    assert_eq!(f.show(v), "<unknown>");
    let int = f.kinded(VarKind::Int);
    f.unify(int, err).unwrap();
}

#[test]
fn params_unify_only_with_themselves() {
    let mut f = Fx::new();
    let (p0, p1) = (f.t.param(crate::ty::ParamIdx(0)), f.t.param(crate::ty::ParamIdx(1)));
    f.unify(p0, p0).unwrap();
    assert!(matches!(f.unify(p0, p1), Err(TypeError::Mismatch { .. })));
}

#[test]
fn rollback_restores_state_exactly_and_commit_keeps_it() {
    let mut f = Fx::new();
    let (a, b) = (f.var(), f.var());
    let before = f.i.clone();
    let snap = f.i.snapshot();
    let extra = f.var();
    let i32_ty = f.t.i32();
    f.unify(a, i32_ty).unwrap();
    f.unify(b, extra).unwrap();
    f.i.rollback_to(snap);
    assert_eq!(f.i, before);
    let shallow = f.i.resolve_shallow(&mut f.t, a);
    assert!(matches!(f.t.data(shallow), TyData::Infer(_)), "a is unbound again");

    let outer = f.i.snapshot();
    f.unify(a, i32_ty).unwrap();
    let inner = f.i.snapshot();
    let bool_ty = f.t.bool();
    f.unify(b, bool_ty).unwrap();
    f.i.rollback_to(inner);
    f.i.commit(outer);
    assert_eq!(f.show(a), "i32");
    let shallow = f.i.resolve_shallow(&mut f.t, b);
    assert!(matches!(f.t.data(shallow), TyData::Infer(_)), "the inner binding was rolled back");
}

#[test]
fn a_failed_unify_changes_nothing() {
    let mut f = Fx::new();
    let (a, b, i32_ty, bool_ty) = (f.var(), f.var(), f.t.i32(), f.t.bool());
    let left = f.t.func(vec![a, b], i32_ty);
    let right = f.t.func(vec![i32_ty, bool_ty], bool_ty);
    let before = f.i.clone();
    assert!(f.unify(left, right).is_err());
    assert_eq!(f.i, before, "bindings made before the failure must be undone");
}

/// Tiny deterministic generator so the randomized test needs no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn random_type(rng: &mut Rng, t: &mut TyCtx, pool: &[TyId], depth: u32) -> TyId {
    let pick = if depth == 0 { rng.below(5) } else { rng.below(11) };
    match pick {
        0 => t.i32(),
        1 => t.bool(),
        2 => t.string(false),
        3 | 4 => pool[rng.below(pool.len() as u64) as usize],
        5 => {
            let e = random_type(rng, t, pool, depth - 1);
            t.array(e, 1 + rng.below(2) as u32)
        }
        6 => {
            let e = random_type(rng, t, pool, depth - 1);
            t.ref_to(e)
        }
        7 => {
            let e = random_type(rng, t, pool, depth - 1);
            t.vec_of(e)
        }
        8 => {
            let e = random_type(rng, t, pool, depth - 1);
            t.nullable(e).unwrap_or(e)
        }
        9 => {
            let e = random_type(rng, t, pool, depth - 1);
            t.range(e)
        }
        _ => {
            let n = rng.below(3) as usize;
            let params = (0..n).map(|_| random_type(rng, t, pool, depth - 1)).collect();
            let ret = random_type(rng, t, pool, depth - 1);
            t.func(params, ret)
        }
    }
}

fn resolved_pool(f: &mut Fx, pool: &[TyId]) -> Vec<Result<TyId, TypeError>> {
    pool.iter().map(|&v| f.i.resolve(&mut f.t, v)).collect()
}

#[test]
fn randomized_unify_and_rollback_restore_state() {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    for _round in 0..300 {
        let mut f = Fx::new();
        let kinds = [VarKind::General, VarKind::General, VarKind::General, VarKind::Int, VarKind::Float];
        let pool: Vec<TyId> = kinds.iter().map(|&k| f.kinded(k)).collect();
        // some committed history first
        for _ in 0..rng.below(4) {
            let (a, b) = (random_type(&mut rng, &mut f.t, &pool, 2), random_type(&mut rng, &mut f.t, &pool, 2));
            let _ = f.unify(a, b);
        }
        let (state, before) = (f.i.clone(), resolved_pool(&mut f, &pool));
        let snap = f.i.snapshot();
        for _ in 0..1 + rng.below(6) {
            let (a, b) = (random_type(&mut rng, &mut f.t, &pool, 3), random_type(&mut rng, &mut f.t, &pool, 3));
            let pre = f.i.clone();
            if f.unify(a, b).is_err() {
                assert_eq!(f.i, pre, "a failed unify must not change the context");
            }
            if rng.below(3) == 0 {
                f.var(); // variables created inside the snapshot are dropped on rollback
            }
        }
        f.i.rollback_to(snap);
        assert_eq!(f.i, state);
        assert_eq!(resolved_pool(&mut f, &pool), before);
    }
}

#[test]
fn randomized_unify_makes_both_sides_resolve_equal() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    let mut agreed = 0;
    for _ in 0..500 {
        let mut f = Fx::new();
        let pool: Vec<TyId> = (0..4).map(|_| f.var()).collect();
        let (a, b) = (random_type(&mut rng, &mut f.t, &pool, 3), random_type(&mut rng, &mut f.t, &pool, 3));
        if f.unify(a, b).is_ok() {
            let (ra, rb) = (f.i.resolve(&mut f.t, a), f.i.resolve(&mut f.t, b));
            assert_eq!(ra, rb, "{} vs {}", f.t.display(a), f.t.display(b));
            agreed += 1;
        }
    }
    assert!(agreed > 50, "the generator should produce many unifiable pairs, got {agreed}");
}

#[test]
fn enum_arguments_unify_and_mismatch_by_definition() {
    let program = crate::parse("enum Opt<T> {\n Empty\n Has(v: T)\n}\nenum Other<T> {\n Empty\n}\nfun main() {}\n").unwrap();
    let defs = DefTable::build(&program);
    let mut fx = Fx { t: TyCtx::new(&defs), i: InferCtxt::new(), defs };
    let (opt, other) = (fx.defs.lookup_enum("Opt").unwrap(), fx.defs.lookup_enum("Other").unwrap());
    let (var, i32_ty) = (fx.var(), fx.t.i32());
    let (with_var, concrete) = (fx.t.enum_ty(opt, vec![var]), fx.t.enum_ty(opt, vec![i32_ty]));
    assert!(fx.unify(with_var, concrete).is_ok());
    assert_eq!(fx.show(with_var), "Opt<i32>");
    let other_ty = fx.t.enum_ty(other, vec![i32_ty]);
    assert!(fx.unify(concrete, other_ty).is_err());
}
