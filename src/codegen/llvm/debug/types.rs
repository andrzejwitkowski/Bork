//! DWARF types matching the target ABI, including recursive class payloads.
use super::DebugInfo;
use crate::ty::{TyData, TyId};
use crate::{codegen::llvm::context::Codegen, hir::Prim};
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

    pub fn ty(&self, cx: &Codegen<'ctx>, ty: TyId) -> DIType<'ctx> {
        if let Some(inner) = cx.tcx.ref_inner(ty) {
            return self.ty(cx, inner);
        }
        let key = cx.tcx.display(ty).to_string();
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
        let result = if cx.tcx.is_managed_ref(ty) {
            let target = cx.tcx.managed_ref_inner(ty).expect("Ref inner");
            let object = self.pointer(
                cx,
                self.payload(cx, cx.tcx.record_name(target).expect("Ref record")),
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
        } else if cx.tcx.is_nullable(ty) && !cx.tcx.is_array(cx.tcx.base(ty)) && !cx.tcx.is_string(ty) {
            let inner = cx.tcx.base(ty);
            let llvm = cx.basic_type(ty).unwrap().into_struct_type();
            let bool_ty = self.ty(cx, cx.bool_ty());
            self.structure(
                cx,
                &key,
                llvm,
                vec![("present", bool_ty), ("value", self.ty(cx, inner))],
            )
        } else if cx.tcx.is_string(ty)
            || cx.tcx.is_array(ty)
            || (cx.tcx.is_nullable(ty)
                && (cx.tcx.is_array(cx.tcx.base(ty)) || cx.tcx.is_string(cx.tcx.base(ty))))
        {
            let element = cx.tcx.array_elem(ty)
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
        } else if let Some(class) = cx.tcx.class_def(ty) {
            let name = cx.tcx.class_name(class);
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
            let encoding = match cx.tcx.data(ty) {
                TyData::Prim(Prim::F32 | Prim::F64) => 0x04,
                TyData::Prim(Prim::Bool) => 0x02,
                TyData::Prim(Prim::U8 | Prim::U16 | Prim::U32 | Prim::U64) => 0x07,
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
            .map(|f| (f.name.as_str(), self.ty(cx, f.ty)))
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
