//! Id-level mirrors of the `hir::Ty` queries and constructors typeck uses. Each says which
//! `hir::Ty` method it mirrors; the "kind" queries look through `T?` (as `hir::Ty` ignores the
//! `nullable` flag there), the strict ones do not.

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

    /// Mirrors `hir::Ty::with_nullable`; a type that cannot be nullable (`Error`) is unchanged.
    pub fn with_nullable(&mut self, id: TyId, nullable: bool) -> TyId {
        let base = self.base(id);
        if nullable {
            self.nullable(base).unwrap_or(base)
        } else {
            base
        }
    }

    /// Mirrors `hir::Ty::supports_nullable`.
    pub fn supports_nullable(&self, id: TyId) -> bool {
        !matches!(
            self.data(self.base(id)),
            TyData::Range(_) | TyData::Ref(_) | TyData::ManagedRef(_) | TyData::Vec(_) | TyData::Error
        )
    }

    /// Mirrors `hir::Ty::is_unknown`.
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

    /// Mirrors `hir::Ty::is_string`.
    pub fn is_string(&self, id: TyId) -> bool {
        matches!(self.data(self.base(id)), TyData::String)
    }

    /// Mirrors `hir::Ty::is_text`: non-nullable `String` or a borrow of one.
    pub fn is_text(&self, id: TyId) -> bool {
        match self.data(id) {
            TyData::String => true,
            TyData::Ref(inner) => self.is_text(*inner),
            _ => false,
        }
    }

    /// Mirrors `hir::Ty::record_name`, as the class definition: an object or a borrow of one.
    pub fn record_def(&self, id: TyId) -> Option<DefId> {
        match self.data(self.base(id)) {
            TyData::Class(def) => Some(*def),
            TyData::Ref(inner) => self.record_def(*inner),
            _ => None,
        }
    }

    /// The receiver of a field access, as `hir::Ty` kinds see it (through `?`): whether it is a
    /// named type (`String` included) directly or behind one borrow / `Ref`, the class if it is
    /// not `String`, and whether it was reached through a `Ref`.
    pub fn field_receiver(&self, id: TyId) -> (bool, Option<DefId>, bool) {
        let named = |inner: TyId| match self.data(self.base(inner)) {
            TyData::Class(def) => (true, Some(*def)),
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

    pub fn class_def(&self, id: TyId) -> Option<DefId> {
        match self.data(id) {
            TyData::Class(def) => Some(*def),
            _ => None,
        }
    }

    /// Mirrors `hir::Ty::is_array` (strict).
    pub fn is_array(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Array { .. })
    }

    /// Mirrors `hir::Ty::array_elem`: through borrows and `?`.
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

    /// Mirrors `hir::Ty::vec_elem`: through borrows and `?`.
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

    /// Mirrors `hir::Ty::is_numeric` (strict).
    pub fn is_numeric(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Prim(p) if p.is_numeric())
    }

    /// Mirrors `hir::Ty::is_integer` (strict).
    pub fn is_integer(&self, id: TyId) -> bool {
        matches!(self.data(id), TyData::Prim(p) if p.is_integer())
    }

    /// Mirrors `hir::Ty::is_array_elem_supported`.
    pub fn is_array_elem_supported(&self, id: TyId) -> bool {
        self.is_copy(id) || self.is_managed_ref(id) || matches!(self.data(id), TyData::String)
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

    pub fn class(&mut self, def: DefId) -> TyId {
        self.add(TyData::Class(def))
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
    use crate::hir::{Prim, Ty, TyKind};
    use crate::ty::tests::{ctx, named, samples};

    /// Every query agrees with the `hir::Ty` method it mirrors, over every sample shape.
    #[test]
    fn queries_match_hir() {
        let mut c = ctx();
        let mut all = samples();
        all.push(Ty::new(TyKind::Func { params: vec![], ret: Box::new(Ty::i32()) }, true));
        all.push(Ty::new(TyKind::Array { elem: Box::new(Ty::prim(Prim::U8)), len: 4 }, false));
        for ty in all {
            let id = c.intern(&ty).unwrap();
            let s = ty.to_string();
            assert_eq!(c.is_nullable(id), ty.nullable && ty.kind != TyKind::Unknown, "{s}");
            assert_eq!(c.supports_nullable(id), ty.supports_nullable(), "{s}");
            assert_eq!(c.is_unknown(id), ty.is_unknown(), "{s}");
            assert_eq!(c.is_ref(id), ty.is_ref(), "{s}");
            assert_eq!(c.is_managed_ref(id), ty.is_managed_ref(), "{s}");
            assert_eq!(c.is_string(id), ty.is_string(), "{s}");
            assert_eq!(c.is_text(id), ty.is_text(), "{s}");
            assert_eq!(c.is_array(id), ty.is_array(), "{s}");
            assert_eq!(c.is_vec(id), ty.is_vec(), "{s}");
            assert_eq!(c.is_numeric(id), ty.is_numeric(), "{s}");
            assert_eq!(c.is_integer(id), ty.is_integer(), "{s}");
            assert_eq!(c.is_array_elem_supported(id), ty.is_array_elem_supported(), "{s}");
            assert_eq!(c.array_len(id), ty.array_len(), "{s}");
            let back = |o: Option<crate::ty::TyId>| o.map(|i| c.to_ty(i));
            assert_eq!(back(c.ref_inner(id)).as_ref(), ty.ref_inner(), "{s}");
            assert_eq!(back(c.managed_ref_inner(id)).as_ref(), ty.managed_ref_inner(), "{s}");
            assert_eq!(back(c.array_elem(id)).as_ref(), ty.array_elem(), "{s}");
            assert_eq!(back(c.vec_elem(id)).as_ref(), ty.vec_elem(), "{s}");
            let record = c.record_def(id).map(|d| c.class_name(d).to_string());
            assert_eq!(record.as_deref(), ty.record_name(), "{s}");
        }
    }

    #[test]
    fn with_nullable_matches_hir_and_constructors_agree_with_intern() {
        let mut c = ctx();
        for ty in samples().into_iter().filter(|t| t.kind != TyKind::Unknown) {
            let id = c.intern(&ty).unwrap();
            for flag in [true, false] {
                let got = c.with_nullable(id, flag);
                assert_eq!(c.to_ty(got), ty.with_nullable(flag), "{ty} {flag}");
            }
        }
        let point = c.intern(&named("Point")).unwrap();
        let by_hand = c.ref_to(point);
        assert_eq!(by_hand, c.intern(&Ty::new(TyKind::Ref(Box::new(named("Point"))), false)).unwrap());
        let (i, s) = (c.i32(), c.string(true));
        assert_eq!(i, c.intern(&Ty::i32()).unwrap());
        assert_eq!(s, c.intern(&Ty::string(true)).unwrap());
        let unknown = c.error();
        assert_eq!(c.with_nullable(unknown, true), unknown);
    }
}
