//! Type capability queries (TS-P6-01, RFC 0002 section 3): the one place that decides whether a
//! value is copied bitwise, cloned as a handle, moved, dropped, or holds a borrow.
//!
//! A generic `T` is conservative (RFC 0002 section 4): move-only, maybe-droppable, not handle-clone.

use std::collections::HashSet;

use super::{TyCtx, TyData, TyId};

impl TyCtx {
    /// Bitwise copy: a non-nullable primitive. Never a `Ref<T>`, a `T?` or a borrow.
    pub fn is_trivially_copy(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Prim(_))
    }

    /// Copying the value is a refcount operation on a managed `Ref<T>` (also through `T?`).
    pub fn is_handle_clone(&self, id: TyId) -> bool {
        self.is_managed_ref(id)
    }

    /// Using the value consumes it: neither a bitwise copy, a handle clone nor a `&T` borrow.
    /// Unknown (`Error`) and unresolved (`Infer`) types are not move-only, so they do not cascade.
    pub fn is_move_only(&self, id: TyId) -> bool {
        !(self.is_trivially_copy(id) || self.is_handle_clone(id) || self.is_ref(id) || self.is_unknown(id) || matches!(self.data(id), TyData::Infer(_)))
    }

    /// Whether `id` mentions a borrow `&T` anywhere.
    pub fn contains_borrow(&self, id: TyId) -> bool {
        struct Find(bool);
        impl super::TypeVisitor for Find {
            fn visit(&mut self, ctx: &TyCtx, id: TyId) {
                self.0 |= matches!(ctx.data(id), TyData::Ref(..));
                super::walk(self, ctx, id);
            }
        }
        let mut find = Find(false);
        super::TypeVisitor::visit(&mut find, self, id);
        find.0
    }
}

/// Queries that need the fields of classes and enums.
pub struct TyCaps<'a> {
    tcx: &'a TyCtx,
    /// The instantiated field (or all variant payload) types of a `Class`/`Enum` type.
    fields: &'a dyn Fn(TyId) -> Vec<TyId>,
}

impl<'a> TyCaps<'a> {
    pub fn new(tcx: &'a TyCtx, fields: &'a dyn Fn(TyId) -> Vec<TyId>) -> Self {
        Self { tcx, fields }
    }

    /// Whether destroying a value of `id` runs code (a managed `Ref` release). `fields` supplies
    /// the instantiated payload types of a user class or enum; `T?` is read from the type itself.
    pub fn needs_drop(&self, id: TyId) -> bool {
        self.drop_rec(id, &mut Vec::new())
    }

    // shortcut: no memo of `false` results, so a heavily shared type graph is revisited per path.
    fn drop_rec(&self, id: TyId, path: &mut Vec<TyId>) -> bool {
        let tcx = self.tcx;
        if tcx.is_nullable(id) {
            return self.drop_rec(tcx.base(id), path);
        }
        match tcx.data(id) {
            TyData::ManagedRef(_) => true,
            // Unresolved: assume the worst (RFC 0002 section 4).
            TyData::Param(_) | TyData::Projection(..) | TyData::Infer(_) => true,
            TyData::Vec(e) | TyData::Array { elem: e, .. } | TyData::Range(e) => self.drop_rec(*e, path),
            TyData::Class(..) | TyData::Enum(..) => {
                // Revisiting a type on the current path is a cycle and adds nothing new. The path
                // is popped on exit, so a sibling never sees a half-evaluated answer.
                if path.contains(&id) {
                    return false;
                }
                path.push(id);
                let any = (self.fields)(id).into_iter().any(|f| self.drop_rec(f, path));
                path.pop();
                any
            }
            TyData::String | TyData::Prim(_) | TyData::Ref(..) | TyData::Func { .. } | TyData::Error => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{ctx, named};
    use super::*;

    #[test]
    fn copy_move_handle_borrow() {
        let mut c = ctx();
        let i = c.i32();
        let unit = c.unit();
        let s = c.string(false);
        let point = named(&mut c, "Point");
        let node = named(&mut c, "Node");
        let r = c.managed_ref(node);
        let opt = c.with_nullable(i, true);
        let b = c.ref_to(point);
        let v = c.vec_of(i);
        let a = c.array(i, 3);
        for t in [i, unit] {
            assert!(c.is_trivially_copy(t) && !c.is_move_only(t) && !c.is_handle_clone(t));
        }
        assert!(!c.is_trivially_copy(r) && c.is_handle_clone(r) && !c.is_move_only(r));
        for t in [s, point, opt, v, a] {
            assert!(!c.is_trivially_copy(t) && !c.is_handle_clone(t) && c.is_move_only(t));
        }
        assert!(!c.is_move_only(b) && c.contains_borrow(b) && !c.contains_borrow(r));
        let arr_of_borrow = c.array(b, 1);
        assert!(c.contains_borrow(arr_of_borrow));
    }

    #[test]
    fn an_editable_borrow_is_a_borrow_with_its_own_type() {
        let mut c = ctx();
        let point = named(&mut c, "Point");
        let (shared, editable) = (c.ref_to(point), c.ref_to_rw(point));
        assert!(c.is_ref(editable) && c.is_rw_ref(editable) && !c.is_rw_ref(shared));
        assert_eq!(c.ref_inner(editable), Some(point));
        assert!(c.contains_borrow(editable) && c.first_borrow_arg(&[editable]) == Some(editable));
        assert!(!c.is_move_only(editable) && !c.is_trivially_copy(editable));
        assert_eq!(c.display(editable).to_string(), "&rw Point");
    }

    #[test]
    fn needs_drop_follows_fields_and_survives_cycles() {
        let mut c = ctx();
        let i = c.i32();
        let point = named(&mut c, "Point");
        let node = named(&mut c, "Node");
        let r = c.managed_ref(node);
        let (v_ref, v_int) = (c.vec_of(r), c.vec_of(i));
        let opt_ref = c.with_nullable(r, true);
        let opt_int = c.with_nullable(i, true);
        // `Point` holds an i32; `Node` holds a `Ref<Node>` and itself (a cycle).
        let fields = |t: TyId| if t == point { vec![i] } else { vec![r, t] };
        let caps = TyCaps::new(&c, &fields);
        assert!(caps.needs_drop(r) && caps.needs_drop(v_ref) && caps.needs_drop(node) && caps.needs_drop(opt_ref));
        assert!(!caps.needs_drop(i) && !caps.needs_drop(v_int) && !caps.needs_drop(point) && !caps.needs_drop(opt_int));
    }

    #[test]
    fn needs_drop_is_independent_of_visit_order_in_mutual_recursion() {
        let mut c = ctx();
        let (a, b) = (named(&mut c, "Point"), named(&mut c, "Node"));
        let r = c.managed_ref(b);
        // a -> b -> a, and only b holds the `Ref`: both must need drop, whichever is asked first.
        let fields = |t: TyId| if t == a { vec![b] } else { vec![a, r] };
        let caps = TyCaps::new(&c, &fields);
        assert!(caps.needs_drop(b) && caps.needs_drop(a));
        let fields_none = |t: TyId| if t == a { vec![b] } else { vec![a] };
        let caps = TyCaps::new(&c, &fields_none);
        assert!(!caps.needs_drop(a) && !caps.needs_drop(b));
    }

    #[test]
    fn unresolved_types_are_not_move_only() {
        let mut c = ctx();
        let (e, v) = (c.error(), c.infer(super::super::InferVar(0)));
        assert!(!c.is_move_only(e) && !c.is_move_only(v));
    }
}
