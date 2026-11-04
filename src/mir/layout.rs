//! Size/alignment of every Bork type, computed Bork-side (RFC 0006 3.5).
//!
//! Mirrors what `codegen/llvm` builds today (`context.rs::basic_type`, `enum_layout_of`, the Vec
//! header in `vec/layout.rs`); the test `mir_layout_matches_llvm_over_corpus` keeps the two equal until
//! codegen reads this module (M5).

use std::collections::HashMap;

use crate::hir::{HirClass, HirClassField, HirEnum, HirProgram, Prim};
use crate::ty::{TyCtx, TyData, TyId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataLayout {
    pub pointer_size: u64,
    pub pointer_align: u64,
}

impl Default for DataLayout {
    fn default() -> Self {
        DataLayout { pointer_size: 8, pointer_align: 8 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repr {
    Scalar,
    /// `String` / array `{ptr, len}`.
    Descriptor,
    /// Class, enum or `T?` value `{arena, object}`.
    ObjectHandle,
    /// Managed `Ref<T>` `{target, object, kind}`.
    RefHandle,
    /// `Vec<T>` value: a pointer to the arena header.
    VecPtr,
    /// Struct body of a class record or of one enum variant (`i32` tag first).
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    pub size: u64,
    pub align: u64,
    /// Byte offset of each field (empty for non-aggregates).
    pub fields: Vec<u64>,
    pub repr: Repr,
}

/// Layout of an enum: the variant bodies and the union (max size, max align).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumLayout {
    pub size: u64,
    pub align: u64,
    pub variants: Vec<Layout>,
}

pub struct Layouts<'a> {
    tcx: &'a TyCtx,
    classes: HashMap<&'a str, &'a HirClass>,
    enums: HashMap<&'a str, &'a HirEnum>,
    pub data: DataLayout,
}

fn scalar(size: u64, repr: Repr) -> Layout {
    Layout { size, align: size.max(1), fields: Vec::new(), repr }
}

/// C-style struct layout of `(size, align)` members.
fn aggregate(members: impl IntoIterator<Item = (u64, u64)>) -> Layout {
    let (mut offset, mut align, mut fields) = (0u64, 1u64, Vec::new());
    for (size, field_align) in members {
        offset = offset.next_multiple_of(field_align);
        fields.push(offset);
        offset += size;
        align = align.max(field_align);
    }
    Layout { size: offset.next_multiple_of(align), align, fields, repr: Repr::Aggregate }
}

impl<'a> Layouts<'a> {
    pub fn new(tcx: &'a TyCtx, hir: &'a HirProgram, data: DataLayout) -> Self {
        Layouts {
            tcx,
            classes: hir.classes.iter().map(|c| (c.name.as_str(), c)).collect(),
            enums: hir.enums.iter().filter(|e| e.generics.is_empty()).map(|e| (e.name.as_str(), e)).collect(),
            data,
        }
    }

    /// Layout of a value of `ty` as held in a local, field or array slot; `None` for `unit` and for
    /// types with no runtime value (functions, ranges, generics).
    pub fn value(&self, ty: TyId) -> Option<Layout> {
        let (tcx, ptr) = (self.tcx, self.data.pointer_size);
        // Same order as `Codegen::basic_type`: `T?` is an enum slot whatever `T` is.
        if tcx.is_nullable(ty) {
            return Some(Layout { size: 2 * ptr, align: self.data.pointer_align, fields: vec![0, ptr], repr: Repr::ObjectHandle });
        }
        if let Some(inner) = tcx.ref_inner(ty) {
            return self.value(inner);
        }
        if tcx.is_managed_ref(ty) {
            // `{ptr, ptr, u8}` padded to pointer alignment.
            let mut l = aggregate([(ptr, self.data.pointer_align), (ptr, self.data.pointer_align), (1, 1)]);
            l.repr = Repr::RefHandle;
            return Some(l);
        }
        if tcx.is_string(ty) || tcx.is_array(ty) {
            return Some(Layout { size: 2 * ptr, align: self.data.pointer_align, fields: vec![0, ptr], repr: Repr::Descriptor });
        }
        if tcx.is_vec(ty) {
            return Some(Layout { size: ptr, align: self.data.pointer_align, fields: Vec::new(), repr: Repr::VecPtr });
        }
        if tcx.class_def(ty).is_some() || matches!(tcx.data(ty), TyData::Enum(..)) {
            return Some(Layout { size: 2 * ptr, align: self.data.pointer_align, fields: vec![0, ptr], repr: Repr::ObjectHandle });
        }
        match tcx.data(ty) {
            TyData::Prim(Prim::Unit) => None,
            TyData::Prim(p) => Some(scalar(prim_size(*p), Repr::Scalar)),
            _ => None,
        }
    }

    /// The `{buf, len, cap, arena}` header a `Vec` value points at.
    pub fn vec_header(&self) -> Layout {
        let (p, a) = (self.data.pointer_size, self.data.pointer_align);
        aggregate([(p, a), (8, 8), (8, 8), (p, a)])
    }

    /// `RefObservation`: `{ptr, ptr}`.
    pub fn observation(&self) -> Layout {
        let (p, a) = (self.data.pointer_size, self.data.pointer_align);
        aggregate([(p, a), (p, a)])
    }

    /// Element size/align inside an array buffer; only primitives, `String` and `Ref` (as codegen).
    pub fn array_elem(&self, elem: TyId) -> Option<(u64, u64)> {
        let tcx = self.tcx;
        if !(tcx.is_string(elem) || tcx.is_managed_ref(elem) || matches!(tcx.data(elem), TyData::Prim(p) if *p != Prim::Unit)) {
            return None;
        }
        self.value(elem).map(|l| (l.size, l.align))
    }

    /// Body of the mono class `name`, fields in declaration order.
    pub fn record(&self, name: &str) -> Option<Layout> {
        let class = self.classes.get(name)?;
        self.fields(&class.fields, None)
    }

    pub fn enum_(&self, name: &str) -> Option<EnumLayout> {
        let e = self.enums.get(name)?;
        let variants = e
            .variants
            .iter()
            .map(|v| self.fields(&v.fields, Some((4, 4))))
            .collect::<Option<Vec<_>>>()?;
        let align = variants.iter().map(|v| v.align).max().unwrap_or(1);
        let size = variants.iter().map(|v| v.size).max().unwrap_or(0).next_multiple_of(align);
        Some(EnumLayout { size, align, variants })
    }

    /// `head` is a leading `(size, align)` member (the enum tag); `unit` fields are zero-sized.
    fn fields(&self, fields: &[HirClassField], head: Option<(u64, u64)>) -> Option<Layout> {
        let mut members: Vec<(u64, u64)> = head.into_iter().collect();
        for f in fields {
            let l = match self.value(f.ty) {
                Some(l) => (l.size, l.align),
                None if head.is_some() && self.tcx.data(f.ty) == &TyData::Prim(Prim::Unit) => (0, 1),
                None => return None,
            };
            members.push(l);
        }
        Some(aggregate(members))
    }
}

fn prim_size(p: Prim) -> u64 {
    match p {
        Prim::I8 | Prim::U8 | Prim::Bool => 1,
        Prim::I16 | Prim::U16 => 2,
        Prim::I32 | Prim::U32 | Prim::F32 => 4,
        Prim::I64 | Prim::U64 | Prim::F64 => 8,
        Prim::Unit => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_padding() {
        let l = aggregate([(1, 1), (8, 8), (4, 4)]);
        assert_eq!((l.size, l.align, l.fields), (24, 8, vec![0, 8, 16]));
    }

    #[test]
    fn handles_match_runtime_shapes() {
        let tcx = TyCtx::new(&Default::default());
        let hir = HirProgram::default();
        let ls = Layouts::new(&tcx, &hir, DataLayout::default());
        assert_eq!(ls.vec_header().size, 32);
        assert_eq!(ls.observation().size, 16);
        let s = tcx.prim_id(Prim::I32);
        assert_eq!(ls.value(s).map(|l| (l.size, l.align)), Some((4, 4)));
        assert_eq!(ls.value(tcx.prim_id(Prim::Unit)), None);
    }
}
