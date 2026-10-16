//! `Vec<T>` codegen.
//!
//! A vector value is a pointer to a header `{ buf, len, cap, arena }` allocated once in the owner
//! arena; the header never moves, only `buf` does (`bork_arena_grow`). The header keeps the owner
//! arena so `push` can grow without knowing where the vector lives.

use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::{AddressSpace, IntPredicate};

use crate::diag::Diagnostic;
use crate::hir::{Ty, VecOp};

use super::emit_fn::FnEmitter;
use super::not_yet_supported;

/// Capacity of the first buffer; later growth doubles.
const FIRST_CAPACITY: u64 = 4;
const HEADER_SIZE: u64 = 32;
const HEADER_ALIGN: usize = 8;
const BUF: u32 = 0;
const LEN: u32 = 1;
const CAP: u32 = 2;
const ARENA: u32 = 3;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    fn vec_header_type(&self) -> StructType<'ctx> {
        let ptr = self.cx.context.ptr_type(AddressSpace::default());
        let word = self.cx.context.i64_type();
        self.cx
            .context
            .struct_type(&[ptr.into(), word.into(), word.into(), ptr.into()], false)
    }

    fn vec_field(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        Ok(self
            .cx
            .builder
            .build_struct_gep(self.vec_header_type(), header, field, "vec.field")?)
    }

    fn vec_load_word(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let field = self.vec_field(header, field)?;
        Ok(self
            .cx
            .builder
            .build_load(self.cx.context.i64_type(), field, "vec.word")?
            .into_int_value())
    }

    fn vec_load_ptr(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let field = self.vec_field(header, field)?;
        Ok(self
            .cx
            .builder
            .build_load(
                self.cx.context.ptr_type(AddressSpace::default()),
                field,
                "vec.ptr",
            )?
            .into_pointer_value())
    }

    fn vec_store(
        &self,
        header: PointerValue<'ctx>,
        field: u32,
        value: impl inkwell::values::BasicValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        let field = self.vec_field(header, field)?;
        self.cx.builder.build_store(field, value)?;
        Ok(())
    }

    fn word(&self, value: u64) -> IntValue<'ctx> {
        self.cx.context.i64_type().const_int(value, false)
    }

    fn vec_slot(
        &self,
        buf: PointerValue<'ctx>,
        index: IntValue<'ctx>,
        stride: usize,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let offset = self
            .cx
            .builder
            .build_int_mul(index, self.word(stride as u64), "vec.off")?;
        // SAFETY: callers bounds-check `index` against `len` or `cap`.
        Ok(unsafe {
            self.cx
                .builder
                .build_gep(self.cx.context.i8_type(), buf, &[offset], "vec.slot")?
        })
    }

    /// Run `body(i)` for `i` in `0..count` (`count` is an `i64`).
    fn vec_counted_loop(
        &mut self,
        count: IntValue<'ctx>,
        mut body: impl FnMut(&mut Self, IntValue<'ctx>) -> Result<(), Diagnostic>,
    ) -> Result<(), Diagnostic> {
        let context = self.cx.context;
        let entry = self.cx.builder.get_insert_block().expect("positioned");
        let header = context.append_basic_block(self.llvm_fn, "vec.loop.hdr");
        let body_block = context.append_basic_block(self.llvm_fn, "vec.loop.body");
        let done = context.append_basic_block(self.llvm_fn, "vec.loop.done");
        self.cx.builder.build_unconditional_branch(header)?;
        self.cx.builder.position_at_end(header);
        let index = self.cx.builder.build_phi(context.i64_type(), "vec.i")?;
        index.add_incoming(&[(&self.word(0), entry)]);
        let i = index.as_basic_value().into_int_value();
        let more = self
            .cx
            .builder
            .build_int_compare(IntPredicate::ULT, i, count, "vec.more")?;
        self.cx
            .builder
            .build_conditional_branch(more, body_block, done)?;
        self.cx.builder.position_at_end(body_block);
        body(self, i)?;
        let next = self.cx.builder.build_int_add(i, self.word(1), "vec.next")?;
        let tail = self.cx.builder.get_insert_block().expect("positioned");
        self.cx.builder.build_unconditional_branch(header)?;
        index.add_incoming(&[(&next, tail)]);
        self.cx.builder.position_at_end(done);
        Ok(())
    }

    /// Allocate a header (and a `cap`-element buffer when `cap > 0`) in `arena`.
    fn vec_alloc(
        &mut self,
        arena: PointerValue<'ctx>,
        elem_ty: &Ty,
        cap: Option<IntValue<'ctx>>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let header = self.arena_alloc(arena, self.word(HEADER_SIZE), HEADER_ALIGN)?;
        let null = self.cx.context.ptr_type(AddressSpace::default()).const_null();
        self.vec_store(header, BUF, null)?;
        self.vec_store(header, LEN, self.word(0))?;
        self.vec_store(header, CAP, self.word(0))?;
        self.vec_store(header, ARENA, arena)?;
        if let Some(cap) = cap {
            let (_, stride, align) = self.elem_storage(elem_ty)?;
            let positive = self.cx.builder.build_int_compare(
                IntPredicate::SGT,
                cap,
                self.word(0),
                "vec.cap.pos",
            )?;
            let alloc = self.cx.context.append_basic_block(self.llvm_fn, "vec.cap.alloc");
            let done = self.cx.context.append_basic_block(self.llvm_fn, "vec.cap.done");
            self.cx
                .builder
                .build_conditional_branch(positive, alloc, done)?;
            self.cx.builder.position_at_end(alloc);
            let bytes = self
                .cx
                .builder
                .build_int_mul(cap, self.word(stride as u64), "vec.cap.bytes")?;
            let buf = self.arena_alloc(arena, bytes, align)?;
            self.vec_store(header, BUF, buf)?;
            self.vec_store(header, CAP, cap)?;
            self.cx.builder.build_unconditional_branch(done)?;
            self.cx.builder.position_at_end(done);
        }
        if elem_ty.is_managed_ref() {
            // One drop per vector: the buffer moves on growth, so slot addresses cannot be registered.
            let drop_fn = self
                .cx
                .vec_drop_refs_fn()
                .as_global_value()
                .as_pointer_value();
            self.cx.builder.build_call(
                self.cx.arena_register_drop_fn(),
                &[arena.into(), header.into(), drop_fn.into()],
                "",
            )?;
        }
        Ok(header)
    }

    /// `Vec<T>()` / `Vec<T>(cap)`; `cap` is the `i32` value of the optional argument.
    pub(super) fn emit_vec_new(
        &mut self,
        elem_ty: &Ty,
        cap: Option<BasicValueEnum<'ctx>>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let arena = self.sink_arena();
        let cap = match cap {
            Some(cap) => Some(self.cx.builder.build_int_s_extend(
                cap.into_int_value(),
                self.cx.context.i64_type(),
                "vec.cap",
            )?),
            None => None,
        };
        Ok(self.vec_alloc(arena, elem_ty, cap)?.into())
    }

    /// Make room for one more element, doubling the buffer when `len == cap`.
    fn vec_reserve_one(
        &mut self,
        header: PointerValue<'ctx>,
        elem_ty: &Ty,
    ) -> Result<(), Diagnostic> {
        let (_, stride, align) = self.elem_storage(elem_ty)?;
        let context = self.cx.context;
        let len = self.vec_load_word(header, LEN)?;
        let cap = self.vec_load_word(header, CAP)?;
        let full = self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, len, cap, "vec.full")?;
        let grow = context.append_basic_block(self.llvm_fn, "vec.grow");
        let done = context.append_basic_block(self.llvm_fn, "vec.room");
        self.cx.builder.build_conditional_branch(full, grow, done)?;

        self.cx.builder.position_at_end(grow);
        let doubled = self
            .cx
            .builder
            .build_int_mul(cap, self.word(2), "vec.cap.x2")?;
        let empty = self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, cap, self.word(0), "vec.empty")?;
        let new_cap = self
            .cx
            .builder
            .build_select(empty, self.word(FIRST_CAPACITY), doubled, "vec.cap.new")?
            .into_int_value();
        let new_bytes = self
            .cx
            .builder
            .build_int_mul(new_cap, self.word(stride as u64), "vec.bytes.new")?;
        let old_bytes = self
            .cx
            .builder
            .build_int_mul(cap, self.word(stride as u64), "vec.bytes.old")?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let arena = self.vec_load_ptr(header, ARENA)?;
        let fresh = context.append_basic_block(self.llvm_fn, "vec.grow.alloc");
        let moved = context.append_basic_block(self.llvm_fn, "vec.grow.grow");
        let joined = context.append_basic_block(self.llvm_fn, "vec.grow.join");
        self.cx.builder.build_conditional_branch(empty, fresh, moved)?;

        self.cx.builder.position_at_end(fresh);
        let first = self.arena_alloc(arena, new_bytes, align)?;
        self.cx.builder.build_unconditional_branch(joined)?;

        self.cx.builder.position_at_end(moved);
        let grown = self
            .cx
            .builder
            .build_call(
                self.cx.arena_grow_fn(),
                &[
                    arena.into(),
                    buf.into(),
                    old_bytes.into(),
                    new_bytes.into(),
                    self.word(align as u64).into(),
                ],
                "vec.grown",
            )?
            .try_as_basic_value()
            .basic()
            .expect("bork_arena_grow returns a pointer")
            .into_pointer_value();
        self.cx.builder.build_unconditional_branch(joined)?;

        self.cx.builder.position_at_end(joined);
        let phi = self.cx.builder.build_phi(
            context.ptr_type(AddressSpace::default()),
            "vec.buf.new",
        )?;
        phi.add_incoming(&[(&first, fresh), (&grown, moved)]);
        self.vec_store(header, BUF, phi.as_basic_value())?;
        self.vec_store(header, CAP, new_cap)?;
        self.cx.builder.build_unconditional_branch(done)?;
        self.cx.builder.position_at_end(done);
        Ok(())
    }

    /// Store `value` into an uninitialised (`fresh`) or live slot of a vector owned by `arena`.
    fn vec_store_elem(
        &mut self,
        slot: PointerValue<'ctx>,
        arena: PointerValue<'ctx>,
        elem_ty: &Ty,
        value: BasicValueEnum<'ctx>,
        fresh: bool,
    ) -> Result<(), Diagnostic> {
        if elem_ty.is_managed_ref() {
            if fresh {
                self.cx
                    .builder
                    .build_store(slot, self.cx.ref_handle_type().const_zero())?;
            }
            return self.store_ref(slot, arena, value);
        }
        // Strings are copied into the vector's arena; Copy values pass through.
        let stored = self.materialize_owned_value(value, elem_ty, arena)?;
        self.cx.builder.build_store(slot, stored)?;
        Ok(())
    }

    fn vec_check_index(
        &mut self,
        header: PointerValue<'ctx>,
        index: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let idx = self.cx.builder.build_int_s_extend(
            index.into_int_value(),
            self.cx.context.i64_type(),
            "vec.idx",
        )?;
        let len = self.vec_load_word(header, LEN)?;
        self.guard_index(idx, len, span)?;
        Ok(idx)
    }

    /// `v[i]` / `v.get(i)`: Copy values load by value, `Ref` elements are cloned.
    pub(super) fn emit_vec_get(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        index: BasicValueEnum<'ctx>,
        elem_ty: &Ty,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let header = receiver.into_pointer_value();
        let idx = self.vec_check_index(header, index, span)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let slot = self.vec_slot(buf, idx, stride)?;
        let loaded = self.cx.builder.build_load(llvm_elem, slot, "vec.get")?;
        if elem_ty.is_managed_ref() {
            self.clone_ref(loaded)
        } else {
            Ok(loaded)
        }
    }

    /// `v[i] = value` where `slot_ptr` is the local holding the header pointer.
    pub(super) fn emit_vec_index_store(
        &mut self,
        slot_ptr: PointerValue<'ctx>,
        vec_ty: &Ty,
        index: &crate::hir::HirExpr,
        value: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let elem_ty = vec_ty
            .vec_elem()
            .ok_or_else(|| not_yet_supported("index assignment target", span))?;
        let header = self
            .cx
            .builder
            .build_load(
                self.cx.context.ptr_type(AddressSpace::default()),
                slot_ptr,
                "vec",
            )?
            .into_pointer_value();
        let index = self.emit_int(index, &Ty::i32())?;
        self.vec_set(header, index.into(), elem_ty, value, span)
    }

    fn vec_set(
        &mut self,
        header: PointerValue<'ctx>,
        index: BasicValueEnum<'ctx>,
        elem_ty: &Ty,
        value: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<(), Diagnostic> {
        let idx = self.vec_check_index(header, index, span)?;
        let (_, stride, _) = self.elem_storage(elem_ty)?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let slot = self.vec_slot(buf, idx, stride)?;
        let arena = self.vec_load_ptr(header, ARENA)?;
        self.vec_store_elem(slot, arena, elem_ty, value, false)
    }

    /// Built-in method on a vector. `ops[0]` is the borrowed header pointer.
    pub(super) fn emit_vec_op(
        &mut self,
        op: VecOp,
        elem_ty: &Ty,
        result_ty: &Ty,
        ops: &[BasicValueEnum<'ctx>],
        span: Option<crate::span::Span>,
    ) -> Result<Option<BasicValueEnum<'ctx>>, Diagnostic> {
        let header = ops[0].into_pointer_value();
        match op {
            VecOp::Len => {
                let len = self.vec_load_word(header, LEN)?;
                Ok(Some(
                    self.cx
                        .builder
                        .build_int_truncate(len, self.cx.context.i32_type(), "vec.len")?
                        .into(),
                ))
            }
            VecOp::Get => self.emit_vec_get(ops[0], ops[1], elem_ty, span).map(Some),
            VecOp::Set => {
                self.vec_set(header, ops[1], elem_ty, ops[2], span)?;
                Ok(None)
            }
            VecOp::Push => {
                self.vec_reserve_one(header, elem_ty)?;
                let (_, stride, _) = self.elem_storage(elem_ty)?;
                let len = self.vec_load_word(header, LEN)?;
                let buf = self.vec_load_ptr(header, BUF)?;
                let slot = self.vec_slot(buf, len, stride)?;
                let arena = self.vec_load_ptr(header, ARENA)?;
                self.vec_store_elem(slot, arena, elem_ty, ops[1], true)?;
                let next = self.cx.builder.build_int_add(len, self.word(1), "vec.len.new")?;
                self.vec_store(header, LEN, next)?;
                Ok(None)
            }
            VecOp::Clear => {
                if elem_ty.is_managed_ref() {
                    let len = self.vec_load_word(header, LEN)?;
                    let buf = self.vec_load_ptr(header, BUF)?;
                    let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
                    self.vec_counted_loop(len, |this, i| {
                        let slot = this.vec_slot(buf, i, stride)?;
                        let handle = this.cx.builder.build_load(llvm_elem, slot, "vec.drop")?;
                        this.drop_ref_value(handle)
                    })?;
                }
                self.vec_store(header, LEN, self.word(0))?;
                Ok(None)
            }
            VecOp::Pop => self.emit_vec_pop(header, elem_ty, result_ty).map(Some),
        }
    }

    fn emit_vec_pop(
        &mut self,
        header: PointerValue<'ctx>,
        elem_ty: &Ty,
        result_ty: &Ty,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let context = self.cx.context;
        let len = self.vec_load_word(header, LEN)?;
        let empty = self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, len, self.word(0), "vec.pop.empty")?;
        let present = context.append_basic_block(self.llvm_fn, "vec.pop.some");
        let absent = context.append_basic_block(self.llvm_fn, "vec.pop.none");
        let merge = context.append_basic_block(self.llvm_fn, "vec.pop.done");
        self.cx
            .builder
            .build_conditional_branch(empty, absent, present)?;

        self.cx.builder.position_at_end(present);
        let last = self.cx.builder.build_int_sub(len, self.word(1), "vec.last")?;
        self.vec_store(header, LEN, last)?;
        let (llvm_elem, stride, _) = self.elem_storage(elem_ty)?;
        let buf = self.vec_load_ptr(header, BUF)?;
        let slot = self.vec_slot(buf, last, stride)?;
        let popped = if elem_ty.is_managed_ref() {
            // The slot is past `len` now, so the handle moves out and the slot becomes `None`.
            self.move_ref(slot)?
        } else {
            let loaded = self.cx.builder.build_load(llvm_elem, slot, "vec.pop")?;
            self.emit_nullable_some(result_ty, loaded)?
        };
        let present_end = self.cx.builder.get_insert_block().expect("positioned");
        self.cx.builder.build_unconditional_branch(merge)?;

        self.cx.builder.position_at_end(absent);
        let none = self.cx.const_null_value(result_ty)?;
        self.cx.builder.build_unconditional_branch(merge)?;

        self.cx.builder.position_at_end(merge);
        let phi = self.cx.builder.build_phi(popped.get_type(), "vec.pop.value")?;
        phi.add_incoming(&[(&popped, present_end), (&none, absent)]);
        Ok(phi.as_basic_value())
    }

    /// `move` / `promote` / owned result: a new vector in the sink arena holding copies of the
    /// elements. Refs are cloned and rehomed (their edge kinds are recomputed for the new owner);
    /// the source stays valid, like `copy_array_into_arena`.
    pub(super) fn copy_vec_into_arena(
        &mut self,
        source: PointerValue<'ctx>,
        vec_ty: &Ty,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        let elem_ty = vec_ty
            .vec_elem()
            .ok_or_else(|| not_yet_supported("vector copy type", None))?
            .clone();
        let arena = self.sink_arena();
        let len = self.vec_load_word(source, LEN)?;
        let header = self.vec_alloc(arena, &elem_ty, Some(len))?;
        let (llvm_elem, stride, _) = self.elem_storage(&elem_ty)?;
        let from = self.vec_load_ptr(source, BUF)?;
        let to = self.vec_load_ptr(header, BUF)?;
        // `vec_alloc` left `to` null for an empty source; the loop below never runs then.
        self.vec_counted_loop(len, |this, i| {
            let src_slot = this.vec_slot(from, i, stride)?;
            let dst_slot = this.vec_slot(to, i, stride)?;
            let loaded = this.cx.builder.build_load(llvm_elem, src_slot, "vec.copy")?;
            let value = if elem_ty.is_managed_ref() {
                this.clone_ref(loaded)?
            } else {
                loaded
            };
            this.vec_store_elem(dst_slot, arena, &elem_ty, value, true)
        })?;
        self.vec_store(header, LEN, len)?;
        Ok(header)
    }
}
