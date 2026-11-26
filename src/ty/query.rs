//! Queries and constructors over `TyId`. The "kind" queries (`is_ref`, `array_elem`, ...) look
//! through `T?`; the strict ones (`is_copy`, `is_array`, `is_text`, ...) do not.

use crate::defs::LangItem;
use super::{ClosureKind, DefId, Prim, TyCtx, TyData, TyId};

impl TyCtx {
    /// A class or enum value, nullable or not: a handle into some arena.
    pub fn holds_object(&self, ty: TyId) -> bool {
        let ty = self.base(ty);
        self.class_def(ty).is_some() || matches!(self.data(ty), TyData::Enum(..))
    }

    /// `T` for `T?`, otherwise `id` itself.
    pub fn base(&self, id: TyId) -> TyId {
        match self.data(id) {
            TyData::Enum(def, args) if Some(*def) == self.option_def && args.len() == 1 && self.supports_nullable_payload(args[0]) => {
                args[0]
            }
            _ => id,
        }
    }

    /// `T?`, which is the prelude's `Option<T>` with a payload `T?` can be written for. An
    /// `Option<Ref<U>>` is only an enum: the kind queries and `?:` / `!!` do not apply to it.
    pub fn is_nullable(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Enum(def, args)
            if Some(*def) == self.option_def && args.len() == 1 && self.supports_nullable_payload(args[0]))
    }

    /// Whether `payload` could have been written `payload?`. An `Option<Ref<T>>` is only an enum:
    /// the kind queries do not look through it.
    fn supports_nullable_payload(&self, payload: TyId) -> bool {
        !self.is_intrinsic_class(payload) && !self.is_vec(payload) && !matches!(self.data(payload), TyData::Ref(..) | TyData::ManagedRef(_) | TyData::Error)
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
        let base = self.base(id);
        !self.is_intrinsic_class(base) && !self.is_vec(base) && !matches!(self.data(base), TyData::Ref(..) | TyData::ManagedRef(_) | TyData::Error)
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

    /// Whether `ty` mentions a projection `T::Item` anywhere.
    pub fn has_projection(&self, ty: TyId) -> bool {
        matches!(self.data(ty), TyData::Projection(..)) || self.data(ty).children().into_iter().any(|c| self.has_projection(c))
    }

    /// Whether `ty` still contains a projection whose base is `base`.
    pub fn has_projection_of(&self, ty: TyId, base: TyId) -> bool {
        matches!(self.data(ty), TyData::Projection(b, _, _) if *b == base)
            || self.data(ty).children().into_iter().any(|c| self.has_projection_of(c, base))
    }

    /// An opaque type parameter (through `?`): any concrete type may stand for it after mono.
    pub fn is_param(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::Param(_))
    }

    /// Type shape: `Ref<T>`, also through `T?`. Ownership questions ask `is_handle_clone`.
    pub fn is_managed_ref(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::ManagedRef(_))
    }

    pub fn is_unknown(&self, id: TyId) -> bool {
        self.is_error(id)
    }

    pub fn is_ref(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::Ref(..))
    }

    pub fn ref_inner(&self, id: TyId) -> Option<TyId> {
        match self.data(self.base(id)) {
            TyData::Ref(inner, _) => Some(*inner),
            _ => None,
        }
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
            TyData::Ref(inner, _) => self.is_text(*inner),
            _ => false,
        }
    }

    /// as the class definition: an object or a borrow of one.
    pub fn record_def(&self, id: TyId) -> Option<DefId> {
        match self.data(self.base(id)) {
            // `Vec<T>` is a class only in name: it has no fields and is not an object.
            TyData::Class(def, _) if !self.is_intrinsic_def(*def) => Some(*def),
            TyData::Ref(inner, _) => self.record_def(*inner),
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
            // `&Ref<T>` (a `Vec` element view) reads like the `Ref` itself.
            TyData::Ref(inner, _) if self.is_managed_ref(*inner) => self.field_receiver(*inner),
            TyData::Ref(inner, _) => {
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

    /// The element type of `id` when it is the intrinsic class `def` (`Vec<T>` or `Range<T>`).
    fn intrinsic_arg(&self, id: TyId, def: Option<DefId>) -> Option<TyId> {
        match self.data(id) {
            TyData::Class(d, args) if Some(*d) == def && args.len() == 1 => Some(args[0]),
            _ => None,
        }
    }

    fn vec_elem_unsupported(&self, id: TyId) -> bool {
        self.vec_elem(id).is_some_and(|elem| !self.is_unknown(elem) && !self.is_vec_elem_supported(elem))
    }

    pub fn range_def(&self) -> Option<DefId> {
        self.lang[LangItem::Range as usize]
    }

    pub fn buf_def(&self) -> Option<DefId> {
        self.lang[LangItem::Buf as usize]
    }

    /// The slot type of a `Buf<T>`, through borrows and `?`.
    pub fn buf_elem(&self, id: TyId) -> Option<TyId> {
        let base = self.base(id);
        match self.data(base) {
            TyData::Ref(inner, _) => self.buf_elem(*inner),
            _ => self.intrinsic_arg(base, self.buf_def()),
        }
    }

    /// `Vec` and `Range` are classes in name only: one type argument, no fields, no object layout.
    pub fn is_intrinsic_def(&self, def: DefId) -> bool {
        self.lang.contains(&Some(def))
    }

    /// A user class or an enum: the types that can implement traits. `Vec` and `Range` are not.
    pub fn is_object_type(&self, id: TyId) -> bool {
        match self.data(id) {
            TyData::Class(def, _) => !self.is_intrinsic_def(*def),
            TyData::Enum(..) => true,
            _ => false,
        }
    }

    pub fn is_intrinsic_class(&self, id: TyId) -> bool {
        self.lang.iter().any(|&def| self.intrinsic_arg(id, def).is_some())
    }

    pub fn class_def(&self, id: TyId) -> Option<DefId> {
        match self.data(id) {
            TyData::Class(def, _) if !self.is_intrinsic_def(*def) => Some(*def),
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
            TyData::Ref(inner, _) => self.array_elem(*inner),
            _ => None,
        }
    }

    pub fn array_len(&self, id: TyId) -> Option<u32> {
        match self.data(self.base(id)) {
            TyData::Array { len, .. } => Some(*len),
            _ => None,
        }
    }

    /// Whether `id` (or a borrow of it) is the Bork `Vec` class.
    pub fn is_vec_class(&self, id: TyId) -> bool {
        self.vec_elem(id).is_some()
    }

    /// Element type of the Bork `Vec` (`Vec<T>`, or a borrow of it); the one `opaque` class of that name.
    pub fn vec_elem(&self, id: TyId) -> Option<TyId> {
        let base = self.base(id);
        match self.data(base) {
            TyData::Ref(inner, _) => self.vec_elem(*inner),
            TyData::Class(def, args) if self.vec_class == Some(*def) => args.first().copied(),
            _ => None,
        }
    }

    pub fn is_vec(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Class(def, _) if self.vec_class == Some(*def))
    }

    pub fn range_elem(&self, id: TyId) -> Option<TyId> {
        self.intrinsic_arg(self.base(id), self.range_def())
    }

    /// `(params, ret)` of a non-nullable function type.
    pub fn func_sig(&self, id: TyId) -> Option<(Vec<TyId>, TyId)> {
        match self.data(id) {
            TyData::Func { params, ret, .. } | TyData::Closure { params, ret, .. } => Some((params.clone(), *ret)),
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
        self.is_managed_ref(id) || self.record_def(id).is_some() || self.enum_args(id).is_some()
    }

    /// payload bytes live in an arena (`String`, objects, `[T]`, `Vec`).
    pub fn uses_arena_storage(&self, id: TyId) -> bool {
        if self.is_nullable(id) {
            return true;
        }
        match self.data(self.base(id)) {
            TyData::Ref(inner, _) => self.uses_arena_storage(*inner),
            TyData::String | TyData::Class(..) | TyData::Enum(..) | TyData::Array { .. } => true,
            // A closure value points at an environment that may live in the caller's arena (RFC 0007).
            TyData::Func { .. } | TyData::Closure { .. } => true,
            _ => false,
        }
    }

    pub fn is_array_elem_supported(&self, id: TyId) -> bool {
        self.is_trivially_copy(id) || self.is_managed_ref(id) || matches!(self.data(id), TyData::String)
    }

    /// `Vec` elements: array elements, plus function values that can be called repeatedly (not `FnOnce`).
    pub fn is_vec_elem_supported(&self, id: TyId) -> bool {
        self.is_array_elem_supported(id) || matches!(self.data(id), TyData::Func { kind, .. } if *kind != ClosureKind::FnOnce)
    }

    /// Whether `Ref<inner>` is a valid type (the parameter case is the caller's to exempt).
    pub fn managed_ref_target_ok(&self, inner: TyId) -> bool {
        !self.is_nullable(inner) && matches!(self.data(inner), TyData::Class(..) | TyData::Enum(..))
    }

    /// The message for an invalid `Ref<inner>` target.
    pub fn managed_ref_target_message(&self, inner: TyId) -> String {
        format!("managed Ref target must be a non-nullable class or enum, got `{}`", self.display(inner))
    }

    /// The message for an invalid `Vec<elem>` element.
    pub fn array_elem_message(&self, elem: TyId) -> String {
        format!("array element type `{}` is not supported", self.display(elem))
    }

    pub fn vec_elem_message(&self, elem: TyId) -> String {
        format!(
            "`Vec` element must be a Copy type, a non-null `String`, a `Ref<T>` or a function type, got `{}`",
            self.display(elem)
        )
    }

    /// Whether a value of `id` stores a borrow. A function type's own parameters do not: a callback
    /// that takes `&P` holds none.
    fn stores_borrow(&self, id: TyId) -> bool {
        match self.data(id) {
            TyData::Ref(..) => true,
            TyData::Func { .. } => false,
            // A plain closure views what it captures; a `move` one stores what its captures store.
            TyData::Closure { id, captures, .. } => {
                if self.closure_kinds[*id as usize].1 {
                    captures.iter().any(|&c| self.stores_borrow(c))
                } else {
                    !captures.is_empty()
                }
            }
            data => data.children().into_iter().any(|child| self.stores_borrow(child)),
        }
    }

    /// The first of `args` that stores a borrow (RFC 0003 section 2). `args` must be fully resolved.
    pub fn first_borrow_arg(&self, args: &[TyId]) -> Option<TyId> {
        // A closure passed for a function's type parameter (`F: Fn`) lives for the call; a class or enum holding one is
        // checked separately (`borrow_arg_at`).
        args.iter().copied().find(|&arg| !matches!(self.data(arg), TyData::Closure { .. }) && self.stores_borrow(arg))
    }

    /// The borrow-type-argument error for this node alone (not its children).
    fn borrow_arg_at(&self, data: &TyData) -> Option<String> {
        let (TyData::Class(def, args) | TyData::Enum(def, args)) = data else { return None };
        // Strict: a class or enum that holds a view closure stores it, unlike a call's `F: Fn` argument.
        args.iter().copied().find(|&arg| self.stores_borrow(arg)).map(|arg| self.borrow_type_arg_message(arg, self.class_name(*def)))
    }

    /// The first class or enum anywhere in `id` that takes a borrow as a type argument.
    pub fn borrow_type_arg_in(&self, id: TyId) -> Option<String> {
        let data = self.data(id);
        self.borrow_arg_at(data).or_else(|| data.children().into_iter().find_map(|child| self.borrow_type_arg_in(child)))
    }

    /// RFC 0003 section 2: a type argument is never a borrow.
    pub fn borrow_type_arg_message(&self, arg: TyId, owner: &str) -> String {
        format!("`{}` cannot be a type argument of `{owner}`: a borrow cannot be stored in a generic type", self.display(arg))
    }

    /// The first rule a concrete type breaks, for instances of generic code: `lower_type` checks
    /// these on written types, but exempts a type parameter ("checked at instantiation"), so the
    /// instantiated type has to be checked again once the parameter is known.
    pub fn ill_formed(&self, id: TyId) -> Option<String> {
        match self.data(id) {
            TyData::ManagedRef(inner) if !self.is_unknown(*inner) && !self.managed_ref_target_ok(*inner) => {
                Some(self.managed_ref_target_message(*inner))
            }
            TyData::Class(..) if self.vec_elem_unsupported(id) => self.vec_elem(id).map(|elem| self.vec_elem_message(elem)),
            data => self.borrow_arg_at(data).or_else(|| data.children().into_iter().find_map(|child| self.ill_formed(child))),
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
        match self.data(id) {
            TyData::Enum(def, args) => Some((*def, args)),
            _ => None,
        }
    }

    /// The class and type arguments of an object or a borrow of one (through `?`).
    pub fn class_args(&self, id: TyId) -> Option<(DefId, &[TyId])> {
        match self.data(self.base(id)) {
            TyData::Class(def, args) => Some((*def, args)),
            TyData::Ref(inner, _) | TyData::ManagedRef(inner) => self.class_args(*inner),
            _ => None,
        }
    }

    pub fn array(&mut self, elem: TyId, len: u32) -> TyId {
        self.add(TyData::Array { elem, len })
    }

    pub fn ref_to(&mut self, inner: TyId) -> TyId {
        self.ref_to_flag(inner, false)
    }

    pub fn ref_to_flag(&mut self, inner: TyId, rw: bool) -> TyId {
        self.add(TyData::Ref(inner, rw))
    }

    /// Whether `id` is `&rw T`.
    pub fn is_rw_ref(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::Ref(_, true))
    }

    pub fn managed_ref(&mut self, inner: TyId) -> TyId {
        self.add(TyData::ManagedRef(inner))
    }

    pub fn vec_of(&mut self, elem: TyId) -> TyId {
        // Without the prelude `Vec` (a program that defines its own) there is no such type; the
        // lowering of the written type has already reported why.
        let Some(def) = self.vec_class else { return self.error() };
        self.add(TyData::Class(def, vec![elem]))
    }

    /// `Buf<elem>`, or the error type without the prelude's `Buf`.
    pub fn buf_of(&mut self, elem: TyId) -> TyId {
        let Some(def) = self.buf_def() else { return self.error() };
        self.add(TyData::Class(def, vec![elem]))
    }

    /// The prelude's opaque `Arena` (#285).
    pub fn is_arena(&self, id: TyId) -> bool {
        let base = self.base(id);
        matches!(self.data(base), TyData::Class(def, _) if self.lang[LangItem::Arena as usize] == Some(*def))
    }

    /// The prelude's opaque `Arena`, or the error type without it.
    pub fn arena(&mut self) -> TyId {
        let Some(def) = self.lang[LangItem::Arena as usize] else { return self.error() };
        self.add(TyData::Class(def, Vec::new()))
    }

    pub fn range(&mut self, elem: TyId) -> TyId {
        let Some(def) = self.range_def() else { return self.error() };
        self.add(TyData::Class(def, vec![elem]))
    }

    pub fn func(&mut self, params: Vec<TyId>, ret: TyId) -> TyId {
        self.func_of(ClosureKind::Fn, params, ret)
    }

    pub fn func_of(&mut self, kind: ClosureKind, params: Vec<TyId>, ret: TyId) -> TyId {
        self.add(TyData::Func { params, ret, kind })
    }

    /// A closure or function value: one pointer to an environment (RFC 0007).
    pub fn is_callable(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Func { .. } | TyData::Closure { .. })
    }

    /// Whether `def` is `Fn`, `FnMut` or `FnOnce`.
    pub fn is_fn_trait(&self, def: DefId) -> bool {
        self.fn_traits.contains(&Some(def))
    }

    /// The weakest kind of closure that satisfies the `Fn*` trait `def`.
    pub fn fn_trait_kind(&self, def: DefId) -> Option<ClosureKind> {
        let at = self.fn_traits.iter().position(|t| *t == Some(def))?;
        Some([ClosureKind::Fn, ClosureKind::FnMut, ClosureKind::FnOnce][at])
    }

    /// A plain closure that captures something: it views its captures, so it cannot leave them.
    pub fn is_view_closure(&self, ty: TyId) -> bool {
        matches!(self.data(ty), TyData::Closure { .. }) && self.stores_borrow(ty)
    }

    pub fn closure_is_move(&self, id: u32) -> bool {
        self.closure_kinds[id as usize].1
    }

    /// The kind of a closure type; a plain function type is `Fn`.
    pub fn closure_kind(&self, ty: TyId) -> Option<ClosureKind> {
        match self.data(ty) {
            TyData::Closure { id, .. } => Some(self.closure_kinds[*id as usize].0),
            TyData::Func { kind, .. } => Some(*kind),
            _ => None,
        }
    }

    /// A fresh closure type: each call is a new closure expression (RFC 0007).
    pub fn closure(&mut self, params: Vec<TyId>, ret: TyId, kind: ClosureKind, is_move: bool, captures: Vec<TyId>) -> TyId {
        let id = self.closure_kinds.len() as u32;
        self.closure_kinds.push((kind, is_move));
        self.add(TyData::Closure { id, params, ret, captures })
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
        assert!(c.is_trivially_copy(i) && !c.is_trivially_copy(opt_i) && c.is_numeric(i) && !c.is_numeric(opt_i));
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
    fn ill_formed_rejects_a_borrow_type_argument() {
        let mut c = ctx();
        let point = named(&mut c, "Point");
        let borrow = c.ref_to(point);
        let (def, _) = c.class_args(point).unwrap();
        let holder = c.class(def, vec![borrow]);
        let message = c.ill_formed(holder).expect("a borrow argument is ill-formed");
        assert!(message.contains("cannot be a type argument of `Point`"), "{message}");
        let owned = c.class(def, vec![point]);
        assert_eq!(c.ill_formed(owned), None);
        let callback = c.func(vec![borrow], point);
        let with_callback = c.class(def, vec![callback]);
        assert_eq!(c.ill_formed(with_callback), None);
        let opt_point = c.nullable(point).unwrap();
        let (option_def, _) = c.enum_args(opt_point).unwrap();
        let option = c.enum_ty(option_def, vec![borrow]);
        assert!(c.ill_formed(option).is_some_and(|m| m.contains("of `Option`")), "enum owner name");
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
            // `Ref<T>` and the like cannot be written `T?`; the rest round-trips.
            if !c.supports_nullable(id) {
                continue;
            }
            let on = c.with_nullable(id, true);
            assert!(c.is_nullable(on));
            assert_eq!(c.with_nullable(on, false), c.base(id), "{}", c.display(id));
            assert_eq!(c.with_nullable(on, true), on);
        }
        let unknown = c.error();
        assert_eq!(c.with_nullable(unknown, true), unknown);
    }
}
