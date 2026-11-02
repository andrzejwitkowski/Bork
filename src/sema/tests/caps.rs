//! The AST-side `is_copy` / `is_handle_clone` must agree with `TyCtx` (`TyCaps`, #60).
use crate::defs::DefTable;
use crate::ty::TyCtx;

#[test]
fn ast_twin_agrees_with_ty_caps() {
    let program = crate::parse("class Node {\n y: i32\n}\nenum Option<T> {\n Some(value: T)\n None\n}\nfun main() {}\n").unwrap();
    let defs = DefTable::build(&program);
    let mut c = TyCtx::new(&defs);
    let def = defs.lookup_class("Node").unwrap();
    let (i, unit, s) = (c.i32(), c.unit(), c.string(false));
    let node = c.class(def, Vec::new());
    let (r, b, v, a) = (c.managed_ref(node), c.ref_to(node), c.vec_of(i), c.array(i, 2));
    let opt = c.with_nullable(i, true);
    let opt_ref = c.with_nullable(r, true);
    for t in [i, unit, s, node, r, b, v, a, opt, opt_ref] {
        let ast = super::super::walk::ty_from_id(&c, t);
        assert_eq!(ast.is_copy(), c.is_trivially_copy(t), "copy {}", c.is_trivially_copy(t));
        assert_eq!(ast.is_handle_clone(), c.is_handle_clone(t));
    }
}
