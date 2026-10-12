//! DWARF types matching the target ABI, including recursive class payloads.
use super::DebugInfo;
use crate::{
    codegen::llvm::context::Codegen,
    hir::{Prim, Ty, TyKind},
};
use inkwell::{
    debug_info::{AsDIScope, DIFlags, DIFlagsConstants, DIType},
    types::{BasicTypeEnum, StructType},
    AddressSpace,
};

impl<'ctx> DebugInfo<'ctx> {
    fn pointer(&self, cx: &Codegen<'ctx>, ty: DIType<'ctx>) -> DIType<'ctx> {
        let ptr = cx.context.ptr_type(AddressSpace::default());
        self.builder
            .create_pointer_type(
                "",
                ty,
                cx.target_data.get_pointer_byte_size(None) as u64 * 8,
                cx.target_data.get_abi_alignment(&ptr) * 8,
                AddressSpace::default(),
            )
            .as_type()
    }

    fn structure(
        &self,
        cx: &Codegen<'ctx>,
        name: &str,
        llvm: StructType<'ctx>,
        fields: Vec<(&str, DIType<'ctx>)>,
    ) -> DIType<'ctx> {
        let scope = self.file.as_debug_info_scope();
        let members: Vec<_> = fields
            .into_iter()
            .enumerate()
            .map(|(i, (name, ty))| {
                let llvm_field = llvm.get_field_type_at_index(i as u32).unwrap();
                self.builder
                    .create_member_type(
                        scope,
                        name,
                        self.file,
                        0,
                        cx.target_data.get_abi_size(&llvm_field) * 8,
                        cx.target_data.get_abi_alignment(&llvm_field) * 8,
                        cx.target_data.offset_of_element(&llvm, i as u32).unwrap() * 8,
                        DIFlags::PUBLIC,
                        ty,
                    )
                    .as_type()
            })
            .collect();
        self.builder
            .create_struct_type(
                scope,
                name,
                self.file,
                0,
                cx.target_data.get_abi_size(&llvm) * 8,
                cx.target_data.get_abi_alignment(&llvm) * 8,
                DIFlags::PUBLIC,
                None,
                &members,
                0,
                None,
                name,
            )
            .as_type()
    }

    pub fn ty(&self, cx: &Codegen<'ctx>, ty: &Ty) -> DIType<'ctx> {
        if let Some(inner) = ty.ref_inner() {
            return self.ty(cx, inner);
        }
        let key = ty.to_string();
        if let Some(found) = self.types.borrow().get(&key) {
            return *found;
        }
        // Recursive class graphs refer to this temporary typedef until their layout is complete.
        let placeholder = unsafe { self.builder.create_placeholder_derived_type(cx.context) };
        self.types
            .borrow_mut()
            .insert(key.clone(), placeholder.as_type());
        let u8_ty = || {
            self.builder
                .create_basic_type("u8", 8, 0x07, DIFlags::PUBLIC)
                .unwrap()
                .as_type()
        };
        let result = if ty.is_managed_ref() {
            let target = ty.managed_ref_inner().expect("Ref inner");
            let object = self.pointer(
                cx,
                self.payload(cx, target.record_name().expect("Ref record")),
            );
            let control = self
                .builder
                .create_basic_type(
                    "usize",
                    cx.target_data.get_pointer_byte_size(None) as u64 * 8,
                    0x07,
                    DIFlags::PUBLIC,
                )
                .unwrap()
                .as_type();
            self.structure(
                cx,
                &format!("bork::{key}"),
                cx.ref_handle_type(),
                vec![("control", control), ("object", object), ("kind", u8_ty())],
            )
        } else if ty.is_nullable() && !ty.with_nullable(false).is_array() && !ty.is_string() {
            let inner = ty.with_nullable(false);
            let llvm = cx.basic_type(ty).unwrap().into_struct_type();
            let bool_ty = self.ty(
                cx,
                &Ty {
                    kind: TyKind::Prim(Prim::Bool),
                    nullable: false,
                },
            );
            self.structure(
                cx,
                &key,
                llvm,
                vec![("present", bool_ty), ("value", self.ty(cx, &inner))],
            )
        } else if ty.is_string()
            || ty.is_array()
            || (ty.is_nullable()
                && (ty.with_nullable(false).is_array() || ty.with_nullable(false).is_string()))
        {
            let element = ty
                .array_elem()
                .map(|t| self.ty(cx, t))
                .unwrap_or_else(u8_ty);
            let data = self.pointer(cx, element);
            let length = self
                .builder
                .create_basic_type("usize", 64, 0x07, DIFlags::PUBLIC)
                .unwrap()
                .as_type();
            self.structure(
                cx,
                &key,
                cx.buffer_descriptor_type(),
                vec![("data", data), ("length", length)],
            )
        } else if let TyKind::Named(name) = &ty.kind {
            let object = self.pointer(cx, self.payload(cx, name));
            let arena = self
                .builder
                .create_basic_type(
                    "usize",
                    cx.target_data.get_pointer_byte_size(None) as u64 * 8,
                    0x07,
                    DIFlags::PUBLIC,
                )
                .unwrap()
                .as_type();
            self.structure(
                cx,
                &format!("bork::Object<{name}>"),
                cx.object_handle_type(),
                vec![("arena", arena), ("object", object)],
            )
        } else {
            let llvm = cx.basic_type(ty);
            let bits = llvm.map_or(0, |t| match t {
                BasicTypeEnum::IntType(i) => i.get_bit_width() as u64,
                _ => cx.target_data.get_abi_size(&t) * 8,
            });
            let encoding = match ty.kind {
                TyKind::Prim(Prim::F32 | Prim::F64) => 0x04,
                TyKind::Prim(Prim::Bool) => 0x02,
                TyKind::Prim(Prim::U8 | Prim::U16 | Prim::U32 | Prim::U64) => 0x07,
                _ => 0x05,
            };
            self.builder
                .create_basic_type(&key, bits, encoding, DIFlags::PUBLIC)
                .unwrap()
                .as_type()
        };
        let alias = self.builder.create_typedef(
            result,
            &key,
            self.file,
            0,
            self.file.as_debug_info_scope(),
            0,
        );
        unsafe {
            self.builder
                .replace_placeholder_derived_type(placeholder, alias);
        }
        self.types.borrow_mut().insert(key, result);
        result
    }

    fn payload(&self, cx: &Codegen<'ctx>, name: &str) -> DIType<'ctx> {
        let key = format!("bork::Payload<{name}>");
        if let Some(found) = self.types.borrow().get(&key) {
            return *found;
        }
        let placeholder = unsafe { self.builder.create_placeholder_derived_type(cx.context) };
        self.types
            .borrow_mut()
            .insert(key.clone(), placeholder.as_type());
        let class = cx.classes.iter().find(|c| c.name == name).unwrap();
        let fields = class
            .fields
            .iter()
            .map(|f| (f.name.as_str(), self.ty(cx, &f.ty)))
            .collect();
        let result = self.structure(cx, &key, cx.record_type(name).unwrap(), fields);
        let alias = self.builder.create_typedef(
            result,
            &key,
            self.file,
            0,
            self.file.as_debug_info_scope(),
            0,
        );
        unsafe {
            self.builder
                .replace_placeholder_derived_type(placeholder, alias);
        }
        self.types.borrow_mut().insert(key, result);
        result
    }
}
