//! Queries and constructors over `TyId`. The "kind" queries (`is_ref`, `array_elem`, ...) look
//! through `T?`; the strict ones (`is_copy`, `is_array`, `is_text`, ...) do not.

use super::{DefId, Prim, TyCtx, TyData, TyId};

impl TyCtx {
    /// `T` for `T?`, otherwise `id` itself.
    pub fn base(&self, id: TyId) -> TyId {
        match self.data(id) {
            TyData::Nullable(inner) => *inner,
            _ => id,
        }
    }

    pub fn is_nullable(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Nullable(_))
    }

    /// a type that cannot be nullable (`Error`) is unchanged.
    pub fn with_nullable(&mut self, id: TyId, nullable: bool) -> TyId {
        let base = self.base(id);
        if nullable {
            self.nullable(base).unwrap_or(base)
        } else {
            base
        }
    }

    pub fn supports_nullable(&self, id: TyId) -> bool {
        !matches!(
            self.data(self.base(id)),
            TyData::Range(_) | TyData::Ref(_) | TyData::ManagedRef(_) | TyData::Vec(_) | TyData::Error
        )
    }

    /// Whether `id` mentions an opaque type parameter anywhere (`Ref<T>`, `Vec<T>`, `T?`, ...).
    pub fn has_param(&self, id: TyId) -> bool {
        struct Find(bool);
        impl crate::ty::TypeVisitor for Find {
            fn visit(&mut self, ctx: &TyCtx, id: TyId) {
                self.0 |= matches!(ctx.data(id), TyData::Param(_));
                crate::ty::walk(self, ctx, id);
            }
        }
        let mut find = Find(false);
        crate::ty::TypeVisitor::visit(&mut find, self, id);
        find.0
    }

    /// An opaque type parameter (through `?`): any concrete type may stand for it after mono.
    pub fn is_param(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::Param(_))
    }

    pub fn is_unknown(&self, id: TyId) -> bool {
        self.is_error(id)
    }

    pub fn is_ref(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::Ref(_))
    }

    pub fn ref_inner(&self, id: TyId) -> Option<TyId> {
        match self.data(self.base(id)) {
            TyData::Ref(inner) => Some(*inner),
            _ => None,
        }
    }

    pub fn is_managed_ref(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::ManagedRef(_))
    }

    pub fn managed_ref_inner(&self, id: TyId) -> Option<TyId> {
        match self.data(self.base(id)) {
            TyData::ManagedRef(inner) => Some(*inner),
            _ => None,
        }
    }

    pub fn is_string(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::String)
    }

    /// non-nullable `String` or a borrow of one.
    pub fn is_text(&self, id: TyId) -> bool {
        match self.data(id) {
            TyData::String => true,
            TyData::Ref(inner) => self.is_text(*inner),
            _ => false,
        }
    }

    /// as the class definition: an object or a borrow of one.
    pub fn record_def(&self, id: TyId) -> Option<DefId> {
        match self.data(self.base(id)) {
            TyData::Class(def, _) => Some(*def),
            TyData::Ref(inner) => self.record_def(*inner),
            _ => None,
        }
    }

    /// The receiver of a field access, through `?`: whether it is a
    /// named type (`String` included) directly or behind one borrow / `Ref`, the class if it is
    /// not `String`, and whether it was reached through a `Ref`.
    pub fn field_receiver(&self, id: TyId) -> (bool, Option<DefId>, bool) {
        let named = |inner: TyId| match self.data(self.base(inner)) {
            TyData::Class(def, _) => (true, Some(*def)),
            TyData::String => (true, None),
            _ => (false, None),
        };
        match self.data(self.base(id)) {
            TyData::Ref(inner) => {
                let (named, class) = named(*inner);
                (named, class, false)
            }
            TyData::ManagedRef(inner) => {
                let (named, class) = named(*inner);
                (named, class, true)
            }
            _ => {
                let (named, class) = named(id);
                (named, class, false)
            }
        }
    }

    pub fn record_name(&self, id: TyId) -> Option<&str> {
        self.record_def(id).map(|def| self.class_name(def))
    }

    pub fn class_def(&self, id: TyId) -> Option<DefId> {
        match self.data(id) {
            TyData::Class(def, _) => Some(*def),
            _ => None,
        }
    }

    /// (strict).
    pub fn is_array(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Array { .. })
    }

    /// through borrows and `?`.
    pub fn array_elem(&self, id: TyId) -> Option<TyId> {
        match self.data(self.base(id)) {
            TyData::Array { elem, .. } => Some(*elem),
            TyData::Ref(inner) => self.array_elem(*inner),
            _ => None,
        }
    }

    pub fn array_len(&self, id: TyId) -> Option<u32> {
        match self.data(self.base(id)) {
            TyData::Array { len, .. } => Some(*len),
            _ => None,
        }
    }

    /// through borrows and `?`.
    pub fn vec_elem(&self, id: TyId) -> Option<TyId> {
        match self.data(self.base(id)) {
            TyData::Vec(elem) => Some(*elem),
            TyData::Ref(inner) => self.vec_elem(*inner),
            _ => None,
        }
    }

    pub fn is_vec(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Vec(_))
    }

    pub fn range_elem(&self, id: TyId) -> Option<TyId> {
        match self.data(self.base(id)) {
            TyData::Range(elem) => Some(*elem),
            _ => None,
        }
    }

    /// `(params, ret)` of a non-nullable function type.
    pub fn func_sig(&self, id: TyId) -> Option<(Vec<TyId>, TyId)> {
        match self.data(id) {
            TyData::Func { params, ret } => Some((params.clone(), *ret)),
            _ => None,
        }
    }

    /// (strict).
    pub fn is_numeric(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Prim(p) if p.is_numeric())
    }

    /// (strict).
    pub fn is_integer(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Prim(p) if p.is_integer())
    }

    /// the target of a borrow, otherwise the type itself.
    pub fn deref_ty(&self, id: TyId) -> TyId {
        self.ref_inner(id).unwrap_or(id)
    }

    /// a value allocated into its consumer's sink arena.
    pub fn owns_result(&self, id: TyId) -> bool {
        self.is_managed_ref(id) || self.record_def(id).is_some()
    }

    /// payload bytes live in an arena (`String`, objects, `[T]`, `Vec`).
    pub fn uses_arena_storage(&self, id: TyId) -> bool {
        match self.data(self.base(id)) {
            TyData::Ref(inner) => self.uses_arena_storage(*inner),
            TyData::String | TyData::Class(..) | TyData::Array { .. } | TyData::Vec(_) => true,
            _ => false,
        }
    }

    pub fn is_array_elem_supported(&self, id: TyId) -> bool {
        self.is_copy(id) || self.is_managed_ref(id) || matches!(self.data(id), TyData::String)
    }

    /// Whether `Ref<inner>` is a valid type (the parameter case is the caller's to exempt).
    pub fn managed_ref_target_ok(&self, inner: TyId) -> bool {
        matches!(self.data(inner), TyData::Class(..))
    }

    /// The message for an invalid `Ref<inner>` target.
    pub fn managed_ref_target_message(&self, inner: TyId) -> String {
        format!("managed Ref target must be a non-nullable class, got `{}`", self.display(inner))
    }

    /// The message for an invalid `Vec<elem>` element.
    pub fn vec_elem_message(&self, elem: TyId) -> String {
        format!(
            "`Vec` element must be a Copy type, a non-null `String` or a `Ref<T>`, got `{}`",
            self.display(elem)
        )
    }

    /// The first rule a concrete type breaks, for instances of generic code: `lower_type` checks
    /// these on written types, but exempts a type parameter ("checked at instantiation"), so the
    /// instantiated type has to be checked again once the parameter is known.
    pub fn ill_formed(&self, id: TyId) -> Option<String> {
        match self.data(id) {
            TyData::ManagedRef(inner) if !self.is_unknown(*inner) && !self.managed_ref_target_ok(*inner) => {
                Some(self.managed_ref_target_message(*inner))
            }
            TyData::Vec(elem) if !self.is_unknown(*elem) && !self.is_array_elem_supported(*elem) => {
                Some(self.vec_elem_message(*elem))
            }
            data => data.children().into_iter().find_map(|child| self.ill_formed(child)),
        }
    }

    /// The id of a primitive without needing `&mut`: `TyCtx::new` interns all of them.
    pub fn prim_id(&self, prim: Prim) -> TyId {
        self.ids[&TyData::Prim(prim)]
    }

    pub fn prim(&mut self, prim: Prim) -> TyId {
        self.add(TyData::Prim(prim))
    }

    pub fn i32(&mut self) -> TyId {
        self.prim(Prim::I32)
    }

    pub fn bool(&mut self) -> TyId {
        self.prim(Prim::Bool)
    }

    pub fn unit(&mut self) -> TyId {
        self.prim(Prim::Unit)
    }

    /// `String` or `String?`.
    pub fn string(&mut self, nullable: bool) -> TyId {
        let string = self.add(TyData::String);
        self.with_nullable(string, nullable)
    }

    pub fn class(&mut self, def: DefId, args: Vec<TyId>) -> TyId {
        self.add(TyData::Class(def, args))
    }

    pub fn enum_ty(&mut self, def: DefId, args: Vec<TyId>) -> TyId {
        self.add(TyData::Enum(def, args))
    }

    /// The enum and type arguments of an enum value (through `?`).
    pub fn enum_args(&self, id: TyId) -> Option<(DefId, &[TyId])> {
        match self.data(self.base(id)) {
            TyData::Enum(def, args) => Some((*def, args)),
            _ => None,
        }
    }

    /// The class and type arguments of an object or a borrow of one (through `?`).
    pub fn class_args(&self, id: TyId) -> Option<(DefId, &[TyId])> {
        match self.data(self.base(id)) {
            TyData::Class(def, args) => Some((*def, args)),
            TyData::Ref(inner) | TyData::ManagedRef(inner) => self.class_args(*inner),
            _ => None,
        }
    }

    pub fn array(&mut self, elem: TyId, len: u32) -> TyId {
        self.add(TyData::Array { elem, len })
    }

    pub fn ref_to(&mut self, inner: TyId) -> TyId {
        self.add(TyData::Ref(inner))
    }

    pub fn managed_ref(&mut self, inner: TyId) -> TyId {
        self.add(TyData::ManagedRef(inner))
    }

    pub fn vec_of(&mut self, elem: TyId) -> TyId {
        self.add(TyData::Vec(elem))
    }

    pub fn range(&mut self, elem: TyId) -> TyId {
        self.add(TyData::Range(elem))
    }

    pub fn func(&mut self, params: Vec<TyId>, ret: TyId) -> TyId {
        self.add(TyData::Func { params, ret })
    }

    /// The name of a class definition, for messages and the legacy view.
    pub fn class_name(&self, def: DefId) -> &str {
        &self.class_names[&def]
    }
}

#[cfg(test)]
mod tests {
    use crate::hir::Prim;
    use crate::ty::tests::{ctx, named, samples};

    #[test]
    fn kind_queries_look_through_nullable_and_strict_ones_do_not() {
        let mut c = ctx();
        let point = named(&mut c, "Point");
        let i = c.i32();
        let (opt_i, opt_point) = (c.with_nullable(i, true), c.with_nullable(point, true));
        let (string, opt_string) = (c.string(false), c.string(true));
        let (arr, vec_ty) = (c.array(i, 4), c.vec_of(i));
        let (ref_arr, ref_vec) = (c.ref_to(arr), c.ref_to(vec_ty));
        let (ref_point, ref_string) = (c.ref_to(point), c.ref_to(string));
        let managed = c.managed_ref(point);
        assert!(c.is_string(opt_string) && !c.is_text(opt_string) && c.is_text(ref_string));
        assert!(c.is_copy(i) && !c.is_copy(opt_i) && c.is_numeric(i) && !c.is_numeric(opt_i));
        assert!(c.is_integer(i) && !c.is_integer(c.prim_id(Prim::F64)));
        assert_eq!(c.record_name(opt_point), Some("Point"));
        assert_eq!(c.record_name(ref_point), Some("Point"));
        assert_eq!(c.record_name(string), None);
        assert_eq!(c.record_name(managed), None);
        assert!(c.owns_result(managed) && c.owns_result(ref_point) && !c.owns_result(string));
        assert!(c.is_array(arr) && !c.is_array(ref_arr) && c.array_elem(ref_arr) == Some(i));
        assert_eq!(c.array_len(arr), Some(4));
        assert!(c.is_vec(vec_ty) && !c.is_vec(ref_vec) && c.vec_elem(ref_vec) == Some(i));
        assert!(c.uses_arena_storage(string) && c.uses_arena_storage(ref_vec) && !c.uses_arena_storage(i));
        assert!(!c.uses_arena_storage(managed));
        assert_eq!(c.deref_ty(ref_point), point);
        assert_eq!(c.deref_ty(point), point);
        assert!(c.is_array_elem_supported(i) && c.is_array_elem_supported(managed));
        assert!(c.is_array_elem_supported(string) && !c.is_array_elem_supported(opt_string));
        assert!(c.supports_nullable(i) && !c.supports_nullable(ref_point) && !c.supports_nullable(vec_ty));
    }

    #[test]
    fn field_receiver_sees_through_borrows_managed_refs_and_string() {
        let mut c = ctx();
        let point = named(&mut c, "Point");
        let (reference, managed, string) = (c.ref_to(point), c.managed_ref(point), c.string(false));
        let def = c.class_def(point);
        assert_eq!(c.field_receiver(point), (true, def, false));
        assert_eq!(c.field_receiver(reference), (true, def, false));
        assert_eq!(c.field_receiver(managed), (true, def, true));
        assert_eq!(c.field_receiver(string), (true, None, false));
        let i = c.i32();
        assert_eq!(c.field_receiver(i), (false, None, false));
    }

    #[test]
    fn with_nullable_toggles_and_leaves_error_alone() {
        let mut c = ctx();
        for id in samples(&mut c) {
            let on = c.with_nullable(id, true);
            assert!(c.is_nullable(on) || c.is_error(id));
            assert_eq!(c.with_nullable(on, false), c.base(id), "{}", c.display(id));
            assert_eq!(c.with_nullable(on, true), on);
        }
        let unknown = c.error();
        assert_eq!(c.with_nullable(unknown, true), unknown);
    }
}
