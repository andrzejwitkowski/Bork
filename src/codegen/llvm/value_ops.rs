//! Arena-parameterised value helpers used by the MIR lowering (`lower.rs`):
//! text, buffer descriptors, copies into another arena and `T?` slots. Ops that branch take the `llvm_fn`
//! to add blocks to.

use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::codegen_error;
use super::context::Codegen;
use super::not_yet_supported;

/// Leading slots of a closure environment before its captures: the code pointer and the copy glue.
pub(super) const CLOSURE_ENV_HEAD: usize = 2;

impl<'ctx> Codegen<'ctx> {
    /// Literal bytes live in a private constant; the descriptor borrows them.
    pub(super) fn str_literal(&self, value: &str) -> StructValue<'ctx> {
        let bytes = self.context.const_string(value.as_bytes(), false);
        let global = self.module.add_global(bytes.get_type(), None, "str");
        global.set_initializer(&bytes);
        global.set_constant(true);
        global.set_linkage(inkwell::module::Linkage::Private);
        global.set_unnamed_addr(true);
        let len = self.context.i64_type().const_int(value.len() as u64, false);
        self.string_type().const_named_struct(&[global.as_pointer_value().into(), len.into()])
    }

    /// `{ ptr, len }` descriptor of a buffer.
    pub(super) fn descriptor(&self, ptr: PointerValue<'ctx>, len: IntValue<'ctx>) -> StructValue<'ctx> {
        let empty = self.buffer_descriptor_type().const_named_struct(&[]);
        let value = self.builder.build_insert_value(empty, ptr, 0, "desc.ptr").expect("descriptor ptr");
        self.builder.build_insert_value(value, len, 1, "desc.len").expect("descriptor len").into_struct_value()
    }

    /// Whether two text descriptors hold the same bytes (an `i1`).
    pub(super) fn string_equal(&self, left: StructValue<'ctx>, right: StructValue<'ctx>) -> Result<IntValue<'ctx>, Diagnostic> {
        let size = self.size_type();
        let mut args = Vec::new();
        for desc in [left, right] {
            args.push(self.builder.build_extract_value(desc, 0, "text.ptr")?.into());
            let len = self.builder.build_extract_value(desc, 1, "text.len")?.into_int_value();
            args.push(self.builder.build_int_cast(len, size, "text.len.usize")?.into());
        }
        let call = self.builder.build_call(self.string_equal_fn(), &args, "text.eq")?.try_as_basic_value().unwrap_basic().into_int_value();
        Ok(self.builder.build_int_compare(IntPredicate::NE, call, call.get_type().const_zero(), "text.cmp")?)
    }

    /// `concat(left, right)` into `arena`.
    pub(super) fn concat_into(&self, arena: PointerValue<'ctx>, left: StructValue<'ctx>, right: StructValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let builder = &self.builder;
        let left_ptr = builder.build_extract_value(left, 0, "concat.l.ptr")?.into_pointer_value();
        let left_len = builder.build_extract_value(left, 1, "concat.l.len")?.into_int_value();
        let right_ptr = builder.build_extract_value(right, 0, "concat.r.ptr")?.into_pointer_value();
        let right_len = builder.build_extract_value(right, 1, "concat.r.len")?.into_int_value();
        let total = builder.build_int_add(left_len, right_len, "concat.len")?;
        let dst = self.arena_alloc(arena, total, 1)?;
        self.debug_allocation(dst, "", "String", false)?;
        builder.build_memcpy(dst, 1, left_ptr, 1, left_len).map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let offset = unsafe { builder.build_gep(self.context.i8_type(), dst, &[left_len], "concat.r.off")? };
        builder.build_memcpy(offset, 1, right_ptr, 1, right_len).map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        Ok(self.descriptor(dst, total))
    }

    /// A copy of a string's bytes in `arena` (`move` / `promote`).
    pub(super) fn copy_string_into(&self, arena: PointerValue<'ctx>, source: StructValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let src = self.builder.build_extract_value(source, 0, "move.src")?.into_pointer_value();
        let len = self.builder.build_extract_value(source, 1, "move.len")?.into_int_value();
        let dst = self.arena_alloc(arena, len, 1)?;
        self.builder.build_memcpy(dst, 1, src, 1, len).map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let moved = self.builder.build_insert_value(source, dst, 0, "moved")?;
        self.debug_copy_origin(src, dst)?;
        self.debug_allocation(dst, "", "String", false)?;
        Ok(moved.into_struct_value())
    }

    /// A copy of an array of plain (`Copy`, non-`Ref`) elements in `arena`.
    pub(super) fn copy_plain_array_into(&self, arena: PointerValue<'ctx>, source: StructValue<'ctx>, array_ty: TyId) -> Result<StructValue<'ctx>, Diagnostic> {
        let elem_ty = self.tcx.array_elem(array_ty).ok_or_else(|| not_yet_supported("array copy type", None))?;
        let base = self.builder.build_extract_value(source, 0, "arr.base")?.into_pointer_value();
        let len = self.builder.build_extract_value(source, 1, "arr.len")?.into_int_value();
        let (_, stride, align) = self.elem_storage(elem_ty)?;
        let bytes = self.builder.build_int_mul(len, self.word(stride as u64), "copy.bytes")?;
        let dst = self.arena_alloc(arena, bytes, align)?;
        if self.debug.is_some() {
            self.debug_allocation(dst, "", &self.tcx.display(array_ty).to_string(), false)?;
            self.debug_copy_origin(base, dst)?;
        }
        self.builder.build_memcpy(dst, align as u32, base, align as u32, bytes).map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        Ok(self.descriptor(dst, len))
    }

    /// One element of a copied buffer of text or `Ref`s: a `Ref` is cloned and stored into `dst` with ownership in
    /// `arena`, text bytes are copied into `arena`.
    fn copy_owned_element(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, elem_ty: TyId, llvm_elem: inkwell::types::BasicTypeEnum<'ctx>, src: PointerValue<'ctx>, dst: PointerValue<'ctx>, register: bool) -> Result<(), Diagnostic> {
        let loaded = self.builder.build_load(llvm_elem, src, "elem.copy")?;
        if self.tcx.is_managed_ref(elem_ty) {
            let cloned = self.ref_clone(llvm_fn, loaded)?;
            self.builder.build_store(dst, self.ref_handle_type().const_zero())?;
            self.ref_store(llvm_fn, dst, arena, cloned)?;
            // An array slot has no header glue to release it; a `Vec` registers one glue for all its slots.
            return if register { self.ref_register_drop(arena, dst) } else { Ok(()) };
        }
        let copied = self.copy_string_into(arena, loaded.into_struct_value())?;
        self.builder.build_store(dst, copied)?;
        Ok(())
    }

    /// A copy of an array of text or `Ref`s in `arena`: text bytes are copied, `Ref`s cloned and stored with ownership.
    pub(super) fn copy_owning_array_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, source: StructValue<'ctx>, array_ty: TyId) -> Result<StructValue<'ctx>, Diagnostic> {
        let elem_ty = self.tcx.array_elem(array_ty).ok_or_else(|| not_yet_supported("array copy type", None))?;
        let base = self.builder.build_extract_value(source, 0, "arr.base")?.into_pointer_value();
        let len = self.builder.build_extract_value(source, 1, "arr.len")?.into_int_value();
        let (llvm_elem, stride, align) = self.elem_storage(elem_ty)?;
        let bytes = self.builder.build_int_mul(len, self.word(stride as u64), "copy.bytes")?;
        let dst = self.arena_alloc(arena, bytes, align)?;
        if self.debug.is_some() {
            self.debug_allocation(dst, "", &self.tcx.display(array_ty).to_string(), false)?;
            self.debug_copy_origin(base, dst)?;
        }
        self.counted_loop(llvm_fn, len, |i| {
            self.copy_owned_element(llvm_fn, arena, elem_ty, llvm_elem, self.vec_slot(base, i, stride)?, self.vec_slot(dst, i, stride)?, true)
        })?;
        Ok(self.descriptor(dst, len))
    }

    pub(super) fn counted_loop(&self, llvm_fn: FunctionValue<'ctx>, count: IntValue<'ctx>, mut body: impl FnMut(IntValue<'ctx>) -> Result<(), Diagnostic>) -> Result<(), Diagnostic> {
        let ctx = self.context;
        let entry = self.builder.get_insert_block().expect("positioned");
        let (head, work, done) = (ctx.append_basic_block(llvm_fn, "vec.loop.hdr"), ctx.append_basic_block(llvm_fn, "vec.loop.body"), ctx.append_basic_block(llvm_fn, "vec.loop.done"));
        self.builder.build_unconditional_branch(head)?;
        self.position_at_end(head);
        let index = self.builder.build_phi(ctx.i64_type(), "vec.i")?;
        index.add_incoming(&[(&self.word(0), entry)]);
        let i = index.as_basic_value().into_int_value();
        let more = self.builder.build_int_compare(IntPredicate::ULT, i, count, "vec.more")?;
        self.builder.build_conditional_branch(more, work, done)?;
        self.position_at_end(work);
        body(i)?;
        let next = self.builder.build_int_add(i, self.word(1), "vec.next")?;
        let tail = self.builder.get_insert_block().expect("positioned");
        self.builder.build_unconditional_branch(head)?;
        index.add_incoming(&[(&next, tail)]);
        self.position_at_end(done);
        Ok(())
    }

    /// The closure environment `env` re-homed into `arena` by its glue (slot 1); no glue means a constant environment, shared as is.
    pub(super) fn copy_closure_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, env: PointerValue<'ctx>) -> Result<PointerValue<'ctx>, Diagnostic> {
        let ptr = self.context.ptr_type(inkwell::AddressSpace::default());
        let slot = self.builder.build_struct_gep(self.context.struct_type(&[ptr.into(), ptr.into()], false), env, 1, "env.glue.slot")?;
        let glue = self.builder.build_load(ptr, slot, "env.glue")?.into_pointer_value();
        let has_glue = self.builder.build_is_not_null(glue, "env.has_glue")?;
        let (call_bb, end_bb) = (self.context.append_basic_block(llvm_fn, "closure.copy"), self.context.append_basic_block(llvm_fn, "closure.copy.end"));
        let entry = self.builder.get_insert_block().expect("positioned");
        self.builder.build_conditional_branch(has_glue, call_bb, end_bb)?;
        self.position_at_end(call_bb);
        let glue_ty = ptr.fn_type(&[ptr.into(), ptr.into()], false);
        let copied = self.builder.build_indirect_call(glue_ty, glue, &[env.into(), arena.into()], "env.copy")?.try_as_basic_value().basic().expect("glue returns a pointer");
        let call_end = self.builder.get_insert_block().expect("positioned");
        self.builder.build_unconditional_branch(end_bb)?;
        self.position_at_end(end_bb);
        let phi = self.builder.build_phi(ptr, "env.copied")?;
        phi.add_incoming(&[(&env, entry), (&copied, call_end)]);
        Ok(phi.as_basic_value().into_pointer_value())
    }


    /// A copy of a `Vec` (the Bork class) in `arena`: a new `{buf, n}` object whose buffer holds the `n` live elements, each
    /// copied the way its type needs (bytes, a cloned `Ref`, a re-homed closure, a copied `String`).
    pub(super) fn copy_vec_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, ty: TyId, value: StructValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let elem_ty = self.tcx.vec_elem(ty).ok_or_else(|| not_yet_supported("vec copy type", None))?;
        let name = crate::mono::class_instance_name(&self.tcx, ty).ok_or_else(|| not_yet_supported("vec copy class", None))?;
        let record = self.record_type(&name).ok_or_else(|| not_yet_supported("vec copy record", None))?;
        let source = self.builder.build_extract_value(value, 1, "vec.src")?.into_pointer_value();
        let field = |object: PointerValue<'ctx>, i: u32| self.builder.build_struct_gep(record, object, i, "vec.field");
        let buf = self.builder.build_load(self.buf_type(), field(source, 0)?, "vec.buf")?.into_struct_value();
        let n = self.builder.build_load(self.context.i32_type(), field(source, 1)?, "vec.n")?.into_int_value();
        let size = record.size_of().ok_or_else(|| not_yet_supported("unsized record", None))?;
        use inkwell::types::BasicType;
        let align = record.get_alignment().get_zero_extended_constant().unwrap_or(8) as usize;
        let object = self.arena_alloc(arena, size, align)?;
        self.debug_allocation(object, "", &name, false)?;
        // An empty vector keeps a `bufEmpty` buffer: no allocation, no drop registration.
        let entry = self.builder.get_insert_block().expect("inside a function");
        let fill = self.context.append_basic_block(llvm_fn, "vec.copy.fill");
        let done = self.context.append_basic_block(llvm_fn, "vec.copy.done");
        let nonempty = self.builder.build_int_compare(inkwell::IntPredicate::NE, n, self.context.i32_type().const_zero(), "vec.nonempty")?;
        self.builder.build_conditional_branch(nonempty, fill, done)?;
        self.builder.position_at_end(fill);
        let copied = self.buf_alloc(arena, elem_ty, n)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let from = self.builder.build_extract_value(buf, 0, "vec.from")?.into_pointer_value();
        let to = self.builder.build_extract_value(copied, 0, "vec.to")?.into_pointer_value();
        let count = self.builder.build_int_s_extend(n, self.context.i64_type(), "vec.count")?;
        self.counted_loop(llvm_fn, count, |i| {
            let (src, dst) = (self.vec_slot(from, i, stride)?, self.vec_slot(to, i, stride)?);
            if self.tcx.is_callable(elem_ty) {
                let env = self.builder.build_load(self.context.ptr_type(inkwell::AddressSpace::default()), src, "vec.env")?.into_pointer_value();
                let moved = self.copy_closure_into(llvm_fn, arena, env)?;
                self.builder.build_store(dst, moved)?;
                Ok(())
            } else if self.tcx.is_managed_ref(elem_ty) || self.tcx.is_text(elem_ty) {
                self.copy_owned_element(llvm_fn, arena, elem_ty, llvm_elem, src, dst, false)
            } else {
                let plain = self.builder.build_load(llvm_elem, src, "vec.elem")?;
                self.builder.build_store(dst, plain)?;
                Ok(())
            }
        })?;
        let filled = self.builder.get_insert_block().expect("inside a function");
        self.builder.build_unconditional_branch(done)?;
        self.builder.position_at_end(done);
        let buf_phi = self.builder.build_phi(self.buf_type(), "vec.copied")?;
        buf_phi.add_incoming(&[(&self.buf_empty()?, entry), (&copied, filled)]);
        self.builder.build_store(field(object, 0)?, buf_phi.as_basic_value())?;
        self.builder.build_store(field(object, 1)?, n)?;
        let handle = self.object_handle_type().const_named_struct(&[]);
        let handle = self.builder.build_insert_value(handle, arena, 0, "vec.arena")?;
        Ok(self.builder.build_insert_value(handle, object, 1, "vec.ptr")?.into_struct_value())
    }

    /// An object is not copied with its holder: keep its arena alive as long as `arena`. No-op for other types.
    fn root_object_arena(&self, arena: PointerValue<'ctx>, ty: TyId, value: BasicValueEnum<'ctx>) -> Result<(), Diagnostic> {
        if self.tcx.holds_object(ty) && !self.tcx.is_nullable(ty) {
            let object_arena = self.builder.build_extract_value(value.into_struct_value(), 0, "object.arena")?;
            self.builder.build_call(self.arena_root_strong_if_dynamic_fn(), &[arena.into(), object_arena.into()], "")?;
        }
        Ok(())
    }

    /// `bork.closure.copy.<function>`: `(env, arena) -> env'`, a copy of a by-value environment in `arena` whose
    /// captures are re-homed like any other copied value (owned `Ref`s cloned and registered). Built once per closure function.
    pub(super) fn closure_copy_glue(&self, function: &str, env_ty: inkwell::types::StructType<'ctx>, tys: &[TyId], owned: &[bool]) -> Result<FunctionValue<'ctx>, Diagnostic> {
        let name = format!("bork.closure.copy.{function}");
        if let Some(glue) = self.module.get_function(&name) {
            return Ok(glue);
        }
        let ptr = self.context.ptr_type(inkwell::AddressSpace::default());
        let glue = self.module.add_function(&name, ptr.fn_type(&[ptr.into(), ptr.into()], false), Some(inkwell::module::Linkage::Internal));
        let resume = self.builder.get_insert_block();
        // The glue has no scope of its own: it must not inherit the caller's DILocation.
        let location = self.builder.get_current_debug_location();
        self.builder.unset_current_debug_location();
        self.position_at_end(self.context.append_basic_block(glue, "entry"));
        let (src, arena) = (glue.get_nth_param(0).expect("env").into_pointer_value(), glue.get_nth_param(1).expect("arena").into_pointer_value());
        let size = env_ty.size_of().ok_or_else(|| not_yet_supported("unsized closure environment", None))?;
        let align = self.context.i64_type().const_int(8, false);
        let raw = self.builder.build_call(self.arena_alloc_fn(), &[arena.into(), size.into(), align.into()], "closure.env")?;
        let dst = raw.try_as_basic_value().basic().expect("arena_alloc returns a pointer").into_pointer_value();
        for i in 0..env_ty.count_fields() {
            let (from, to) = (self.builder.build_struct_gep(env_ty, src, i, "env.src")?, self.builder.build_struct_gep(env_ty, dst, i, "env.dst")?);
            let field = env_ty.get_field_type_at_index(i).expect("field");
            let value = self.builder.build_load(field, from, "env.field")?;
            // The head slots (code, glue) are copied as they are.
            let Some(k) = (i as usize).checked_sub(CLOSURE_ENV_HEAD) else {
                self.builder.build_store(to, value)?;
                continue;
            };
            if owned[k] {
                let cloned = self.ref_clone(glue, value)?;
                self.builder.build_store(to, self.ref_handle_type().const_zero())?;
                self.ref_store(glue, to, arena, cloned)?;
                self.ref_register_drop(arena, to)?;
            } else {
                let copied = self.copy_value_into(glue, arena, tys[k], value)?;
                self.builder.build_store(to, copied)?;
                self.root_object_arena(arena, tys[k], copied)?;
            }
        }
        self.builder.build_return(Some(&dst))?;
        if let Some(block) = resume {
            self.position_at_end(block);
        }
        if let Some(location) = location {
            self.builder.set_current_debug_location(location);
        }
        Ok(glue)
    }

    /// The `Option` instance behind `T?` and its payload type.
    pub(super) fn option_parts(&self, ty: TyId) -> Result<(String, TyId), Diagnostic> {
        let name = crate::mono::enum_instance_name(&self.tcx, ty)
            .ok_or_else(|| not_yet_supported(&format!("nullable type `{}`", self.tcx.display(ty)), None))?;
        let payload = self.tcx.enum_args(ty).and_then(|(_, args)| args.first().copied());
        payload.map(|payload| (name, payload)).ok_or_else(|| not_yet_supported(&format!("nullable type `{}`", self.tcx.display(ty)), None))
    }

    /// Whether the `T?` slot behind `value` holds `None` (an `i1`).
    pub(super) fn nullable_is_none(&self, ty: TyId, value: StructValue<'ctx>) -> Result<IntValue<'ctx>, Diagnostic> {
        let (name, _) = self.option_parts(ty)?;
        let (_, none_tag) = self.enum_variant(&name, "None").ok_or_else(|| not_yet_supported("the prelude `Option`", None))?;
        let record = self.enum_layout(&name).expect("laid out with its enum").variants[none_tag as usize];
        let object = self.builder.build_extract_value(value, 1, "object.ptr")?.into_pointer_value();
        let i32_ty = self.context.i32_type();
        let tag_ptr = self.builder.build_struct_gep(record, object, 0, "opt.tag.ptr")?;
        let tag = self.builder.build_load(i32_ty, tag_ptr, "opt.tag")?.into_int_value();
        Ok(self.builder.build_int_compare(IntPredicate::EQ, tag, i32_ty.const_int(u64::from(none_tag), false), "opt.is_null")?)
    }

    /// An `Option` slot in `arena` holding variant `tag` and, for `Some`, `payload` (stored as is: the
    /// caller has already put any bytes it owns into `arena`). No drop glue: plain payloads only.
    pub(super) fn option_slot(&self, arena: PointerValue<'ctx>, name: &str, tag: u32, payload: Option<BasicValueEnum<'ctx>>) -> Result<StructValue<'ctx>, Diagnostic> {
        let layout = self.enum_layout(name).ok_or_else(|| not_yet_supported(&format!("enum `{name}`"), None))?;
        let (record, size, align) = (layout.variants[tag as usize], layout.size, layout.align);
        let object = self.arena_alloc(arena, self.word(size), align as usize)?;
        self.debug_allocation(object, "", name, false)?;
        let tag_slot = self.builder.build_struct_gep(record, object, 0, "enum.tag")?;
        self.builder.build_store(tag_slot, self.context.i32_type().const_int(u64::from(tag), false))?;
        if let Some(payload) = payload {
            let slot = self.builder.build_struct_gep(record, object, 1, "payload")?;
            self.builder.build_store(slot, payload)?;
        }
        let handle = self.object_handle_type().const_named_struct(&[]);
        let handle = self.builder.build_insert_value(handle, arena, 0, "object.arena")?;
        Ok(self.builder.build_insert_value(handle, object, 1, "object.ptr")?.into_struct_value())
    }

    /// `stringFromUtf8(bytes, start, count)` into `arena`: a bad range aborts, invalid UTF-8 gives
    /// `None` before any allocation, a valid range is copied.
    pub(super) fn string_from_utf8(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, buffer: StructValue<'ctx>, start: IntValue<'ctx>, count: IntValue<'ctx>, result_ty: TyId) -> Result<StructValue<'ctx>, Diagnostic> {
        let builder = &self.builder;
        let (i64_ty, i8_ty) = (self.context.i64_type(), self.context.i8_type());
        let base = builder.build_extract_value(buffer, 0, "utf8.base")?.into_pointer_value();
        let len = builder.build_extract_value(buffer, 1, "utf8.len")?.into_int_value();
        let start = builder.build_int_s_extend(start, i64_ty, "utf8.start")?;
        let count = builder.build_int_s_extend(count, i64_ty, "utf8.count")?;
        // `len - start` may wrap when `start > len`, but that case is already `bad`.
        let rest = builder.build_int_sub(len, start, "utf8.rest")?;
        let zero = i64_ty.const_zero();
        let neg_start = builder.build_int_compare(IntPredicate::SLT, start, zero, "utf8.neg.start")?;
        let neg_count = builder.build_int_compare(IntPredicate::SLT, count, zero, "utf8.neg.count")?;
        let start_oob = builder.build_int_compare(IntPredicate::SGT, start, len, "utf8.start.oob")?;
        let count_oob = builder.build_int_compare(IntPredicate::SGT, count, rest, "utf8.count.oob")?;
        let bad_start = builder.build_or(neg_start, start_oob, "utf8.bad.start")?;
        let bad_count = builder.build_or(neg_count, count_oob, "utf8.bad.count")?;
        let bad = builder.build_or(bad_start, bad_count, "utf8.bad")?;
        self.branch_abort_if(llvm_fn, bad)?;

        let src = unsafe { builder.build_gep(i8_ty, base, &[start], "utf8.src")? };
        let count_usize = builder.build_int_cast(count, self.size_type(), "utf8.count.usize")?;
        let valid = builder
            .build_call(self.utf8_valid_fn(), &[src.into(), count_usize.into()], "utf8.ok")?
            .try_as_basic_value()
            .basic()
            .expect("bork_utf8_valid returns a byte")
            .into_int_value();
        let is_valid = builder.build_int_compare(IntPredicate::NE, valid, i8_ty.const_zero(), "utf8.valid")?;

        let (name, _) = self.option_parts(result_ty)?;
        let (_, some_tag) = self.enum_variant(&name, "Some").ok_or_else(|| not_yet_supported("the prelude `Option`", None))?;
        let (_, none_tag) = self.enum_variant(&name, "None").ok_or_else(|| not_yet_supported("the prelude `Option`", None))?;
        let copy = self.context.append_basic_block(llvm_fn, "utf8.copy");
        let absent = self.context.append_basic_block(llvm_fn, "utf8.none");
        let done = self.context.append_basic_block(llvm_fn, "utf8.done");
        builder.build_conditional_branch(is_valid, copy, absent)?;

        self.position_at_end(copy);
        let dst = self.arena_alloc(arena, count, 1)?;
        self.debug_allocation(dst, "", "String", false)?;
        builder.build_memcpy(dst, 1, src, 1, count).map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        let some = self.option_slot(arena, &name, some_tag, Some(self.descriptor(dst, count).as_basic_value_enum()))?;
        let copy_end = builder.get_insert_block().expect("positioned in a block");
        builder.build_unconditional_branch(done)?;

        self.position_at_end(absent);
        let none = self.option_slot(arena, &name, none_tag, None)?;
        let none_end = builder.get_insert_block().expect("positioned in a block");
        builder.build_unconditional_branch(done)?;

        self.position_at_end(done);
        let phi = builder.build_phi(some.get_type(), "utf8.result")?;
        phi.add_incoming(&[(&none, none_end), (&some, copy_end)]);
        Ok(phi.as_basic_value().into_struct_value())
    }

    /// A copy of a nullable whose payload is text, a plain array or a plain `Vec` (or a scalar):
    /// the slot and the payload bytes both move into `arena`.
    pub(super) fn copy_nullable_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, ty: TyId, value: StructValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let (name, payload_ty) = self.option_parts(ty)?;
        let (_, some_tag) = self.enum_variant(&name, "Some").ok_or_else(|| not_yet_supported("the prelude `Option`", None))?;
        let (_, none_tag) = self.enum_variant(&name, "None").ok_or_else(|| not_yet_supported("the prelude `Option`", None))?;
        let is_none = self.nullable_is_none(ty, value)?;
        let some_bb = self.context.append_basic_block(llvm_fn, "opt.copy.some");
        let none_bb = self.context.append_basic_block(llvm_fn, "opt.copy.none");
        let end_bb = self.context.append_basic_block(llvm_fn, "opt.copy.end");
        self.builder.build_conditional_branch(is_none, none_bb, some_bb)?;

        self.position_at_end(some_bb);
        let record = self.enum_layout(&name).expect("laid out with its enum").variants[some_tag as usize];
        let object = self.builder.build_extract_value(value, 1, "object.ptr")?.into_pointer_value();
        let slot = self.builder.build_struct_gep(record, object, 1, "opt.val.ptr")?;
        // A `()?` payload has no LLVM type and no bytes: `Some` is just the tag.
        let payload = match self.basic_type(payload_ty) {
            Some(payload_llvm) => {
                let payload = self.builder.build_load(payload_llvm, slot, "opt.val")?;
                let copied = self.copy_value_into(llvm_fn, arena, payload_ty, payload)?;
                self.root_object_arena(arena, payload_ty, copied)?;
                Some(copied)
            }
            None => None,
        };
        let some = self.option_slot(arena, &name, some_tag, payload)?;
        let some_end = self.builder.get_insert_block().expect("positioned");
        self.builder.build_unconditional_branch(end_bb)?;

        self.position_at_end(none_bb);
        let none = self.option_slot(arena, &name, none_tag, None)?;
        let none_end = self.builder.get_insert_block().expect("positioned");
        self.builder.build_unconditional_branch(end_bb)?;

        self.position_at_end(end_bb);
        let phi = self.builder.build_phi(some.get_type(), "opt.copy")?;
        phi.add_incoming(&[(&some, some_end), (&none, none_end)]);
        if self.debug.is_some() {
            let source = self.builder.build_extract_value(value, 1, "debug.source")?.into_pointer_value();
            let destination = self.builder.build_extract_value(phi.as_basic_value().into_struct_value(), 1, "debug.destination")?.into_pointer_value();
            self.debug_copy_origin(source, destination)?;
        }
        Ok(phi.as_basic_value().into_struct_value())
    }

    /// Whether the enum `ty` holds a `Ref` or a closure (see `mono::enum_holds_ref`).
    pub(super) fn enum_holds_ref(&self, ty: TyId) -> bool {
        crate::mono::enum_holds_ref(&self.tcx, &self.enums, ty)
    }

    /// An enum holding `Ref`s or closures copied into `arena` (#338): a fresh slot whose handles are cloned (the new slot
    /// owns and drops them) and whose other fields are copied like any value.
    fn copy_ref_enum_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, ty: TyId, value: StructValue<'ctx>) -> Result<StructValue<'ctx>, Diagnostic> {
        let name = crate::mono::enum_instance_name(&self.tcx, ty).ok_or_else(|| not_yet_supported("enum copy", None))?;
        let layout = self.enum_layout(&name).ok_or_else(|| not_yet_supported(&format!("enum `{name}`"), None))?;
        let e = self.enums.iter().find(|e| e.name == name).ok_or_else(|| not_yet_supported(&format!("enum `{name}`"), None))?;
        let object = self.builder.build_extract_value(value, 1, "enum.src")?.into_pointer_value();
        let new = self.arena_alloc(arena, self.word(layout.size), layout.align as usize)?;
        self.debug_allocation(new, "", &name, false)?;
        let i32_ty = self.context.i32_type();
        let tag = self.builder.build_load(i32_ty, self.builder.build_struct_gep(layout.variants[0], object, 0, "tag.src")?, "tag")?.into_int_value();
        self.builder.build_store(self.builder.build_struct_gep(layout.variants[0], new, 0, "tag.dst")?, tag)?;
        let done = self.context.append_basic_block(llvm_fn, "enum.copy.end");
        for (index, variant) in e.variants.iter().enumerate().filter(|(_, v)| v.fields.iter().any(|f| f.ty != self.unit_ty())) {
            let arm = self.context.append_basic_block(llvm_fn, &format!("enum.copy.{index}"));
            let after = self.context.append_basic_block(llvm_fn, &format!("enum.copy.after.{index}"));
            let hit = self.builder.build_int_compare(inkwell::IntPredicate::EQ, tag, i32_ty.const_int(index as u64, false), "is.variant")?;
            self.builder.build_conditional_branch(hit, arm, after)?;
            self.position_at_end(arm);
            for (i, field) in variant.fields.iter().enumerate().filter(|(_, f)| f.ty != self.unit_ty()) {
                let (from, to) = (self.builder.build_struct_gep(layout.variants[index], object, i as u32 + 1, "field.src")?, self.builder.build_struct_gep(layout.variants[index], new, i as u32 + 1, "field.dst")?);
                let llvm = self.basic_type(field.ty).ok_or_else(|| not_yet_supported("enum field without a value type", None))?;
                let loaded = self.builder.build_load(llvm, from, "field")?;
                if self.tcx.is_handle_clone(field.ty) {
                    let cloned = self.ref_clone(llvm_fn, loaded)?;
                    self.builder.build_store(to, self.ref_handle_type().const_zero())?;
                    self.ref_store(llvm_fn, to, arena, cloned)?;
                } else {
                    let copied = self.copy_value_into(llvm_fn, arena, field.ty, loaded)?;
                    self.root_object_arena(arena, field.ty, copied)?;
                    self.builder.build_store(to, copied)?;
                }
            }
            self.builder.build_unconditional_branch(done)?;
            self.position_at_end(after);
        }
        self.builder.build_unconditional_branch(done)?;
        self.position_at_end(done);
        if let Some(glue) = self.enum_drop_glue(&name) {
            self.builder.build_call(self.arena_register_drop_fn(), &[arena.into(), new.into(), glue.as_global_value().as_pointer_value().into()], "")?;
        }
        let handle = self.object_handle_type().const_named_struct(&[]);
        let handle = self.builder.build_insert_value(handle, arena, 0, "object.arena")?;
        let copied = self.builder.build_insert_value(handle, new, 1, "object.ptr")?.into_struct_value();
        if self.debug.is_some() {
            self.debug_copy_origin(object, new)?;
        }
        Ok(copied)
    }

    /// `value` of type `ty` copied into `arena`: text, arrays, `Vec`s, closures and nullables of
    /// those; anything else (scalars) is returned as is.
    pub(super) fn copy_value_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, ty: TyId, value: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.tcx.is_nullable(ty) {
            return Ok(self.copy_nullable_into(llvm_fn, arena, ty, value.into_struct_value())?.into());
        }
        if self.tcx.is_callable(ty) {
            return Ok(self.copy_closure_into(llvm_fn, arena, value.into_pointer_value())?.into());
        }
        if self.enum_holds_ref(ty) {
            return Ok(self.copy_ref_enum_into(llvm_fn, arena, ty, value.into_struct_value())?.into());
        }
        if self.tcx.is_text(ty) {
            return Ok(self.copy_string_into(arena, value.into_struct_value())?.into());
        }
        if self.tcx.is_array(ty) {
            if self.tcx.array_elem(ty).is_some_and(|e| self.tcx.is_text(e) || self.tcx.is_managed_ref(e)) {
                return Ok(self.copy_owning_array_into(llvm_fn, arena, value.into_struct_value(), ty)?.into());
            }
            return Ok(self.copy_plain_array_into(arena, value.into_struct_value(), ty)?.into());
        }
        if self.tcx.is_vec(ty) {
            return Ok(self.copy_vec_into(llvm_fn, arena, ty, value.into_struct_value())?.into());
        }
        Ok(value)
    }
}
