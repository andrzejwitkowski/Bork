use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, PointerValue, StructValue};

use crate::diag::Diagnostic;
use crate::ty::TyId;
use crate::hir::{HirExpr};
use crate::memory::{ArenaClass, LifetimeDomain};

use super::emit_fn::FnEmitter;
use super::not_yet_supported;

impl<'ctx> FnEmitter<'_, '_, '_, 'ctx> {
    pub(super) fn emit_ref_eq(
        &mut self,
        lhs: BasicValueEnum<'ctx>,
        rhs: BasicValueEnum<'ctx>,
        negate: bool,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let left = self.entry_alloca(self.cx.ref_handle_type(), "ref.eq.left")?;
        let right = self.entry_alloca(self.cx.ref_handle_type(), "ref.eq.right")?;
        self.cx.builder.build_store(left, lhs)?;
        self.cx.builder.build_store(right, rhs)?;
        let equal = self
            .cx
            .builder
            .build_call(
                self.cx.ref_equal_fn(),
                &[left.into(), right.into()],
                "ref.eq",
            )?
            .try_as_basic_value()
            .basic()
            .expect("Ref equality returns bool")
            .into_int_value();
        self.drop_ref_value(lhs)?;
        self.drop_ref_value(rhs)?;
        Ok(if negate {
            self.cx.builder.build_not(equal, "ref.ne")?
        } else {
            equal
        }
        .into())
    }

    pub(super) fn materialize_owned_value(
        &mut self,
        value: BasicValueEnum<'ctx>,
        field_ty: TyId,
        owner: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.cx.tcx.is_managed_ref(field_ty) {
            return self.rehome_ref(owner, value);
        }
        if self.cx.tcx.is_vec(field_ty) {
            let previous = self.alloc_sink;
            self.alloc_sink = Some(owner);
            let copied = self.copy_vec_into_arena(value.into_pointer_value(), field_ty);
            self.alloc_sink = previous;
            return copied.map(Into::into);
        }
        let inner = self.cx.tcx.base(field_ty);
        if !self.cx.tcx.is_string(inner) && !self.cx.tcx.is_array(inner) {
            return Ok(value);
        }

        if self.cx.tcx.is_nullable(field_ty) {
            let is_null = self.emit_nullable_is_null(field_ty, value)?;
            let mut split = self.open_presence("owned", is_null, Some(field_ty))?;
            let copied = self.materialize_owned_value(value, inner, owner)?;
            self.end_present_arm(&mut split, Some(copied))?;
            let absent = self.cx.const_null_value(field_ty)?;
            return self
                .end_absent_arm(split, Some(absent))?
                .ok_or_else(|| not_yet_supported("nullable owned value", None));
        }

        let previous = self.alloc_sink;
        self.alloc_sink = Some(owner);
        let result = if self.cx.tcx.is_array(field_ty) {
            self.copy_array_into_arena(value.into_struct_value(), field_ty)
                .map(Into::into)
        } else {
            self.copy_into_arena(value.into_struct_value())
                .map(Into::into)
        };
        self.alloc_sink = previous;
        result
    }

    pub(super) fn begin_object_construct(&mut self, expr: &HirExpr) -> Result<(), Diagnostic> {
        let span = expr
            .span
            .ok_or_else(|| not_yet_supported("record allocation without a source span", None))?;
        let allocation = self.cx.memory_plan.allocation_at(expr.id).ok_or_else(|| {
            not_yet_supported("record allocation without a memory plan", Some(span))
        })?;
        let previous = self.alloc_sink;
        let crate::hir::HirExprKind::ObjectConstruct { class_name, .. } = &expr.kind else {
            return Err(not_yet_supported("record allocation", Some(span)));
        };
        let arena = if allocation.class == ArenaClass::Dynamic {
            let owner = self.current_arena();
            let root_owner = match &allocation.lifetime_domain {
                LifetimeDomain::Program => self.result_sink.or(self.alloc_sink).unwrap_or(owner),
                LifetimeDomain::LexicalScope(_) => owner,
            };
            let (parent, scoped) = match allocation.lifetime_domain {
                LifetimeDomain::Program => {
                    let root = self
                        .cx
                        .builder
                        .build_call(self.cx.arena_root_fn(), &[], "arena.root")?
                        .try_as_basic_value()
                        .basic()
                        .expect("bork_arena_root returns a pointer")
                        .into_pointer_value();
                    (root, 0)
                }
                LifetimeDomain::LexicalScope(_) => (owner, 1),
            };
            let arena = self
                .cx
                .builder
                .build_call(
                    self.cx.arena_create_dynamic_fn(),
                    &[
                        parent.into(),
                        self.cx.context.i8_type().const_int(scoped, false).into(),
                    ],
                    "arena.dynamic",
                )?
                .try_as_basic_value()
                .basic()
                .expect("bork_arena_create_dynamic returns a pointer")
                .into_pointer_value();
            self.cx.builder.build_call(
                self.cx.arena_register_strong_root_fn(),
                &[root_owner.into(), arena.into()],
                "",
            )?;
            arena
        } else if let Some(sink) = self.result_sink.filter(|_| {
            self.cx.tcx.record_name(self.function.return_ty_id) == Some(class_name)
        }) {
            sink
        } else {
            self.sink_arena()
        };
        self.cx.debug_label_arena(arena)?;
        self.object_constructions.push((previous, arena));
        self.alloc_sink = Some(arena);
        Ok(())
    }

    pub(super) fn emit_object_construct(
        &mut self,
        class_name: &str,
        fields: &[HirExpr],
        values: &[BasicValueEnum<'ctx>],
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let (previous, arena) = self
            .object_constructions
            .pop()
            .ok_or_else(|| not_yet_supported("record construction context", span))?;
        let result = (|| {
            let record = self
                .cx
                .record_type(class_name)
                .ok_or_else(|| not_yet_supported(&format!("record `{class_name}`"), span))?;
            let size = record
                .size_of()
                .ok_or_else(|| not_yet_supported("unsized record", span))?;
            let align = record.get_alignment();
            let object = self
                .cx
                .builder
                .build_call(
                    self.cx.arena_alloc_fn(),
                    &[arena.into(), size.into(), align.into()],
                    "object",
                )?
                .try_as_basic_value()
                .basic()
                .expect("bork_arena_alloc returns a pointer")
                .into_pointer_value();

            for (index, (field, value)) in fields.iter().zip(values).enumerate() {
                let slot =
                    self.cx
                        .builder
                        .build_struct_gep(record, object, index as u32, "field")?;
                if self.cx.tcx.is_managed_ref(field.ty_id) {
                    self.cx
                        .builder
                        .build_store(slot, self.cx.ref_handle_type().const_zero())?;
                    self.store_ref(slot, arena, *value)?;
                    self.cx.builder.build_call(
                        self.cx.arena_register_ref_drop_fn(),
                        &[arena.into(), slot.into()],
                        "",
                    )?;
                    let field_name = self
                        .cx
                        .classes
                        .iter()
                        .find(|c| c.name == class_name)
                        .unwrap()
                        .fields[index]
                        .name
                        .clone();
                    self.cx.debug_label_slot(
                        arena,
                        slot,
                        &format!("{class_name}.{field_name}"),
                        field.ty_id,
                        self.cx.context.i64_type().const_zero(),
                        self.cx.context.i64_type().const_all_ones(),
                    )?;
                } else {
                    let value = self.materialize_owned_value(*value, field.ty_id, arena)?;
                    self.cx.builder.build_store(slot, value)?;
                    let field_name = &self
                        .cx
                        .classes
                        .iter()
                        .find(|c| c.name == class_name)
                        .unwrap()
                        .fields[index]
                        .name;
                    self.debug_label_ref_array(
                        value,
                        field.ty_id,
                        &format!("{class_name}.{field_name}"),
                    )?;
                }
            }

            let handle = self.cx.object_handle_type().const_named_struct(&[]);
            let handle = self
                .cx
                .builder
                .build_insert_value(handle, arena, 0, "object.arena")?;
            Ok(self
                .cx
                .builder
                .build_insert_value(handle, object, 1, "object.ptr")?
                .into_struct_value()
                .into())
        })();
        self.alloc_sink = previous;
        result
    }

    pub(super) fn emit_ref_create(
        &mut self,
        object: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let object = object.into_struct_value();
        let target = self.object_control(object)?;
        let ptr = self.object_pointer(object)?;
        let source = self.sink_arena();
        let output = self.entry_alloca(self.cx.ref_handle_type(), "ref.tmp")?;
        self.cx.builder.build_call(
            self.cx.ref_create_out_fn(),
            &[output.into(), source.into(), target.into(), ptr.into()],
            "",
        )?;
        Ok(self
            .cx
            .builder
            .build_load(self.cx.ref_handle_type(), output, "ref")?)
    }

    pub(super) fn emit_object_field(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        class_name: &str,
        field_index: usize,
        result_ty: TyId,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let object = self.object_pointer(receiver.into_struct_value())?;
        self.load_object_field(object, class_name, field_index, result_ty, span)
    }

    pub(super) fn emit_object_field_store(
        &mut self,
        receiver: BasicValueEnum<'ctx>,
        class_name: &str,
        field_index: usize,
        value: &HirExpr,
    ) -> Result<(), Diagnostic> {
        let receiver = receiver.into_struct_value();
        let owner = self.object_control(receiver)?;
        let object = self.object_pointer(receiver)?;
        let record = self
            .cx
            .record_type(class_name)
            .ok_or_else(|| not_yet_supported(&format!("record `{class_name}`"), value.span))?;
        let slot =
            self.cx
                .builder
                .build_struct_gep(record, object, field_index as u32, "field.place")?;
        let previous = self.alloc_sink;
        self.alloc_sink = Some(owner);
        let stored = self.value_from_walk(value, || {
            not_yet_supported("field assignment value", value.span)
        });
        self.alloc_sink = previous;
        let stored = stored?;
        if self.cx.tcx.is_managed_ref(value.ty_id) {
            self.store_ref(slot, owner, stored)?;
        } else {
            let stored = self.materialize_owned_value(stored, value.ty_id, owner)?;
            self.cx.builder.build_store(slot, stored)?;
            let field_name = &self
                .cx
                .classes
                .iter()
                .find(|c| c.name == class_name)
                .unwrap()
                .fields[field_index]
                .name;
            self.debug_label_ref_array(stored, value.ty_id, &format!("{class_name}.{field_name}"))?;
        }
        Ok(())
    }

    pub(super) fn load_object_field(
        &mut self,
        object: PointerValue<'ctx>,
        class_name: &str,
        field_index: usize,
        result_ty: TyId,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let record = self
            .cx
            .record_type(class_name)
            .ok_or_else(|| not_yet_supported(&format!("record `{class_name}`"), span))?;
        let slot = self
            .cx
            .builder
            .build_struct_gep(record, object, field_index as u32, "field")?;
        let ty = self
            .cx
            .basic_type(result_ty)
            .ok_or_else(|| not_yet_supported(&format!("field type `{}`", self.cx.tcx.display(result_ty)), span))?;
        let loaded = self.cx.builder.build_load(ty, slot, "field")?;
        if self.cx.tcx.is_managed_ref(result_ty) {
            return self.clone_ref(loaded);
        }
        Ok(loaded)
    }

    pub(super) fn object_control(
        &self,
        object: StructValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        Ok(self
            .cx
            .builder
            .build_extract_value(object, 0, "object.arena")?
            .into_pointer_value())
    }

    pub(super) fn move_ref(
        &mut self,
        slot: PointerValue<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let output = self.entry_alloca(self.cx.ref_handle_type(), "ref.move.tmp")?;
        self.cx
            .builder
            .build_call(self.cx.ref_move_out_fn(), &[output.into(), slot.into()], "")?;
        Ok(self
            .cx
            .builder
            .build_load(self.cx.ref_handle_type(), output, "ref.move")?)
    }

    pub(super) fn clone_ref(
        &mut self,
        reference: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let input = self.entry_alloca(self.cx.ref_handle_type(), "ref.copy.in")?;
        let output = self.entry_alloca(self.cx.ref_handle_type(), "ref.copy.out")?;
        self.cx.builder.build_store(input, reference)?;
        self.cx.builder.build_call(
            self.cx.ref_clone_out_fn(),
            &[output.into(), input.into()],
            "",
        )?;
        Ok(self
            .cx
            .builder
            .build_load(self.cx.ref_handle_type(), output, "ref.copy")?)
    }

    pub(super) fn store_ref(
        &mut self,
        slot: PointerValue<'ctx>,
        owner: PointerValue<'ctx>,
        reference: BasicValueEnum<'ctx>,
    ) -> Result<(), Diagnostic> {
        let incoming = self.entry_alloca(self.cx.ref_handle_type(), "ref.store.in")?;
        self.cx.builder.build_store(incoming, reference)?;
        self.cx.builder.build_call(
            self.cx.ref_store_take_fn(),
            &[slot.into(), owner.into(), incoming.into()],
            "",
        )?;
        Ok(())
    }

    pub(super) fn drop_ref_value(
        &mut self,
        reference: BasicValueEnum<'ctx>,
    ) -> Result<(), Diagnostic> {
        let input = self.entry_alloca(self.cx.ref_handle_type(), "ref.drop")?;
        self.cx.builder.build_store(input, reference)?;
        self.cx
            .builder
            .build_call(self.cx.ref_drop_value_fn(), &[input.into()], "")?;
        Ok(())
    }

    pub(super) fn register_ref_drop(
        &mut self,
        owner: PointerValue<'ctx>,
        slot: PointerValue<'ctx>,
    ) -> Result<(), Diagnostic> {
        self.cx.builder.build_call(
            self.cx.arena_register_ref_drop_fn(),
            &[owner.into(), slot.into()],
            "",
        )?;
        Ok(())
    }

    pub(super) fn store_fresh_ref(
        &mut self,
        slot: PointerValue<'ctx>,
        owner: PointerValue<'ctx>,
        reference: BasicValueEnum<'ctx>,
    ) -> Result<(), Diagnostic> {
        self.cx
            .builder
            .build_store(slot, self.cx.ref_handle_type().const_zero())?;
        self.store_ref(slot, owner, reference)?;
        self.register_ref_drop(owner, slot)
    }

    /// `strongCount(r)` / `weakCount(r)`. The argument is released before the counter is read so
    /// a temporary copy of `r` is not counted. Releasing the last reference can free the arena's
    /// control record, so a weak lease pins it for the read; its own +1 is removed from
    /// `weakCount`. A `None` handle has a null control, takes no lease and reads as 0.
    pub(super) fn emit_ref_count(
        &mut self,
        reference: BasicValueEnum<'ctx>,
        weak: bool,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let control = self
            .cx
            .builder
            .build_extract_value(reference.into_struct_value(), 0, "ref.control")?;
        let leased = self
            .cx
            .builder
            .build_call(
                self.cx.arena_count_lease_retain_fn(),
                &[control.into()],
                "ref.count.lease",
            )?
            .try_as_basic_value()
            .basic()
            .expect("lease retain returns a flag")
            .into_int_value();
        self.drop_ref_value(reference)?;
        let mut count = self
            .cx
            .builder
            .build_call(self.cx.arena_count_fn(weak), &[control.into()], "ref.count")?
            .try_as_basic_value()
            .basic()
            .expect("count returns usize")
            .into_int_value();
        if weak {
            let without_lease = self.cx.builder.build_int_sub(
                count,
                count.get_type().const_int(1, false),
                "ref.weak.unleased",
            )?;
            count = self
                .cx
                .builder
                .build_select(leased, without_lease, count, "ref.weak")?
                .into_int_value();
        }
        self.cx.builder.build_call(
            self.cx.arena_count_lease_release_fn(),
            &[control.into()],
            "",
        )?;
        Ok(self
            .cx
            .builder
            .build_int_truncate_or_bit_cast(count, self.cx.context.i32_type(), "ref.count.i32")?
            .into())
    }

    pub(super) fn rehome_ref(
        &mut self,
        owner: PointerValue<'ctx>,
        reference: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let input = self.entry_alloca(self.cx.ref_handle_type(), "ref.rehome.in")?;
        let output = self.entry_alloca(self.cx.ref_handle_type(), "ref.rehome.out")?;
        self.cx.builder.build_store(input, reference)?;
        self.cx.builder.build_call(
            self.cx.ref_rehome_out_fn(),
            &[output.into(), owner.into(), input.into()],
            "",
        )?;
        let rehomed =
            self.cx
                .builder
                .build_load(self.cx.ref_handle_type(), output, "ref.rehome")?;
        self.drop_ref_value(reference)?;
        Ok(rehomed)
    }

    pub(super) fn object_pointer(
        &self,
        object: StructValue<'ctx>,
    ) -> Result<PointerValue<'ctx>, Diagnostic> {
        Ok(self
            .cx
            .builder
            .build_extract_value(object, 1, "object.ptr")?
            .into_pointer_value())
    }
}
