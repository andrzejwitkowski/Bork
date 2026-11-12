//! Arena-parameterised value helpers shared by the HIR emitter and the MIR lowering (`lower.rs`):
//! text, buffer descriptors, copies into another arena and `T?` slots. None of them needs a
//! `FnEmitter`; ops that branch take the `llvm_fn` to add blocks to.

use inkwell::values::{BasicValue, BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

use crate::diag::Diagnostic;
use crate::ty::TyId;

use super::codegen_error;
use super::context::Codegen;
use super::not_yet_supported;

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

    /// A copy of a `Vec` of plain elements in `arena`: a new header whose buffer holds `len` elements.
    pub(super) fn copy_plain_vec_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, source: PointerValue<'ctx>, elem_ty: TyId) -> Result<PointerValue<'ctx>, Diagnostic> {
        let len = self.vec_load_word(source, super::vec::LEN)?;
        let header = self.vec_alloc(llvm_fn, arena, elem_ty, Some(len))?;
        let (_, stride, align) = self.elem_storage(elem_ty)?;
        let from = self.vec_load_ptr(source, super::vec::BUF)?;
        let to = self.vec_load_ptr(header, super::vec::BUF)?;
        let bytes = self.builder.build_int_mul(len, self.word(stride as u64), "vec.copy.bytes")?;
        // `vec_alloc` leaves `to` null for an empty source; a zero-byte copy touches nothing.
        self.builder.build_memcpy(to, align as u32, from, align as u32, bytes).map_err(|err| codegen_error(format!("LLVM builder error: {err}"), None))?;
        self.vec_store(header, super::vec::LEN, len)?;
        self.debug_copy_origin(source, header)?;
        self.debug_copy_origin(from, to)?;
        Ok(header)
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
        let payload_llvm = self.basic_type(payload_ty).ok_or_else(|| not_yet_supported("nullable payload", None))?;
        let payload = self.builder.build_load(payload_llvm, slot, "opt.val")?;
        let payload = self.copy_value_into(llvm_fn, arena, payload_ty, payload)?;
        let some = self.option_slot(arena, &name, some_tag, Some(payload))?;
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

    /// `value` of type `ty` copied into `arena`: text, plain arrays, plain `Vec`s and nullables of
    /// those; anything else (scalars) is returned as is.
    pub(super) fn copy_value_into(&self, llvm_fn: FunctionValue<'ctx>, arena: PointerValue<'ctx>, ty: TyId, value: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.tcx.is_nullable(ty) {
            return Ok(self.copy_nullable_into(llvm_fn, arena, ty, value.into_struct_value())?.into());
        }
        if self.tcx.is_text(ty) {
            return Ok(self.copy_string_into(arena, value.into_struct_value())?.into());
        }
        if self.tcx.is_array(ty) {
            return Ok(self.copy_plain_array_into(arena, value.into_struct_value(), ty)?.into());
        }
        if let Some(elem) = self.tcx.vec_elem(ty) {
            return Ok(self.copy_plain_vec_into(llvm_fn, arena, value.into_pointer_value(), elem)?.into());
        }
        Ok(value)
    }
}
