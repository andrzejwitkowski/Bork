//! Lowering for `T?`, `None`, `Some`, `?:`, and `!!`. `T?` is the prelude's `Option<T>` (#54), so a
//! nullable value is an enum slot (`{ i32 tag, payload }`, see `enum_layout_of`) and these are the
//! tag tests, constructions and payload reads over it. `Ref<T>` keeps its own null handle.

use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};
use inkwell::IntPredicate;

use crate::ast::BinOp;
use crate::diag::Diagnostic;
use crate::hir::{HirExpr, HirExprKind, Prim};
use crate::ty::{TyData, TyId};

use super::emit_fn::FnEmitter;
use super::not_yet_supported;

impl<'s, 'report, 'a, 'ctx> FnEmitter<'s, 'report, 'a, 'ctx> {
    /// The instance name of the `Option` that `ty` is, and its payload type.
    fn option_parts(&self, ty: TyId) -> Result<(String, TyId), Diagnostic> {
        let name = crate::mono::enum_instance_name(&self.cx.tcx, ty)
            .ok_or_else(|| not_yet_supported(&format!("nullable type `{}`", self.cx.tcx.display(ty)), None))?;
        let payload = self.cx.tcx.enum_args(ty).and_then(|(_, args)| args.first().copied());
        payload
            .map(|payload| (name, payload))
            .ok_or_else(|| not_yet_supported(&format!("nullable type `{}`", self.cx.tcx.display(ty)), None))
    }

    /// The object pointer of the nullable value `value`.
    fn option_object(&mut self, value: BasicValueEnum<'ctx>) -> Result<PointerValue<'ctx>, Diagnostic> {
        self.object_pointer(value.into_struct_value())
    }

    pub(super) fn emit_nullable_is_null(
        &mut self,
        ty: TyId,
        value: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let (name, _) = self.option_parts(ty)?;
        let (_, none_tag) = self
            .cx
            .enum_variant(&name, "None")
            .ok_or_else(|| not_yet_supported("the prelude `Option`", None))?;
        let record = self.cx.enum_layout(&name).expect("laid out with its enum").variants[none_tag as usize];
        let object = self.option_object(value)?;
        let i32_ty = self.cx.context.i32_type();
        let tag_ptr = self.cx.builder.build_struct_gep(record, object, 0, "opt.tag.ptr")?;
        let tag = self.cx.builder.build_load(i32_ty, tag_ptr, "opt.tag")?.into_int_value();
        let wanted = i32_ty.const_int(u64::from(none_tag), false);
        Ok(self.cx.builder.build_int_compare(IntPredicate::EQ, tag, wanted, "opt.is_null")?)
    }

    /// `Some(inner)` in the current sink arena.
    pub(super) fn emit_nullable_some(
        &mut self,
        nullable_ty: TyId,
        inner: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let (name, payload) = self.option_parts(nullable_ty)?;
        let arena = self.sink_arena();
        self.build_enum_slot(arena, &name, "Some", &[payload], &[inner], None)
    }

    /// `None` in the current sink arena.
    pub(super) fn emit_nullable_none(&mut self, nullable_ty: TyId) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let (name, _) = self.option_parts(nullable_ty)?;
        let arena = self.sink_arena();
        self.build_enum_slot(arena, &name, "None", &[], &[], None)
    }

    /// A copy of a nullable value in the current sink arena, for `move` and `promote`: the slot
    /// and its payload (a `String`'s bytes, say) both move there.
    pub(super) fn emit_nullable_copy(
        &mut self,
        ty: TyId,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let is_null = self.emit_nullable_is_null(ty, value)?;
        let some_bb = self.cx.context.append_basic_block(self.llvm_fn, "opt.copy.some");
        let none_bb = self.cx.context.append_basic_block(self.llvm_fn, "opt.copy.none");
        let end_bb = self.cx.context.append_basic_block(self.llvm_fn, "opt.copy.end");
        self.cx.builder.build_conditional_branch(is_null, none_bb, some_bb)?;
        self.cx.builder.position_at_end(some_bb);
        let payload = self.emit_nullable_unwrap(ty, value)?;
        let some = self.emit_nullable_some(ty, payload)?;
        let some_end = self.cx.builder.get_insert_block().expect("positioned");
        self.cx.builder.build_unconditional_branch(end_bb)?;
        self.cx.builder.position_at_end(none_bb);
        let none = self.emit_nullable_none(ty)?;
        let none_end = self.cx.builder.get_insert_block().expect("positioned");
        self.cx.builder.build_unconditional_branch(end_bb)?;
        self.cx.builder.position_at_end(end_bb);
        let phi = self.cx.builder.build_phi(some.get_type(), "opt.copy")?;
        phi.add_incoming(&[(&some, some_end), (&none, none_end)]);
        if self.cx.debug.is_some() {
            let source = self.object_pointer(value.into_struct_value())?;
            let destination = self.object_pointer(phi.as_basic_value().into_struct_value())?;
            self.cx.debug_copy_origin(source, destination)?;
        }
        Ok(phi.as_basic_value())
    }

    /// The absent value of `ty`: a null handle for a `Ref<T>`, a `None` slot for a `T?`.
    pub(super) fn emit_none(&mut self, ty: TyId) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.cx.tcx.is_managed_ref(ty) {
            return Ok(self.cx.ref_handle_type().const_zero().into());
        }
        self.emit_nullable_none(ty)
    }

    /// The payload of a nullable value known to be `Some`.
    pub(super) fn emit_nullable_unwrap(
        &mut self,
        nullable_ty: TyId,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let (name, payload) = self.option_parts(nullable_ty)?;
        let (_, some_tag) = self
            .cx
            .enum_variant(&name, "Some")
            .ok_or_else(|| not_yet_supported("the prelude `Option`", None))?;
        let record = self.cx.enum_layout(&name).expect("laid out with its enum").variants[some_tag as usize];
        let object = self.option_object(value)?;
        let ty = self
            .cx
            .basic_type(payload)
            .ok_or_else(|| not_yet_supported(&format!("nullable payload `{}`", self.cx.tcx.display(payload)), None))?;
        let slot = self.cx.builder.build_struct_gep(record, object, 1, "opt.val.ptr")?;
        Ok(self.cx.builder.build_load(ty, slot, "opt.val")?)
    }

    pub(super) fn emit_nullable_expect_non_null(
        &mut self,
        nullable_ty: TyId,
        value: BasicValueEnum<'ctx>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let cx = self.cx;
        let is_null = self.emit_nullable_is_null(nullable_ty, value)?;
        let ok_bb = cx.context.append_basic_block(self.llvm_fn, "nn.ok");
        let abort_bb = cx.context.append_basic_block(self.llvm_fn, "nn.abort");
        cx.builder
            .build_conditional_branch(is_null, abort_bb, ok_bb)?;
        cx.builder.position_at_end(abort_bb);
        cx.builder.build_call(cx.abort_function(), &[], "abort")?;
        cx.builder.build_unreachable()?;
        cx.builder.position_at_end(ok_bb);
        self.emit_nullable_unwrap(nullable_ty, value)
    }

    pub(super) fn emit_elvis(
        &mut self,
        lhs: &HirExpr,
        rhs: &HirExpr,
        lhs_val: BasicValueEnum<'ctx>,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        if self.cx.tcx.is_managed_ref(lhs.ty) {
            return self.emit_ref_elvis(lhs_val, rhs, expr);
        }
        let nullable_ty = lhs.ty;
        let result_ty = expr.ty;
        let is_null = self.emit_nullable_is_null(nullable_ty, lhs_val)?;
        let rhs_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "elvis.rhs");
        let lhs_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "elvis.lhs");
        let merge_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "elvis.end");
        self.cx
            .builder
            .build_conditional_branch(is_null, rhs_bb, lhs_bb)?;
        self.cx.builder.position_at_end(rhs_bb);
        let rhs_val = self.emit_skipped_as(rhs, result_ty)?;
        let rhs_end = self.cx.builder.get_insert_block().expect("rhs block");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(lhs_bb);
        let lhs_unwrapped = self.emit_nullable_unwrap(nullable_ty, lhs_val)?;
        let lhs_end = self.cx.builder.get_insert_block().expect("lhs block");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(merge_bb);
        let llvm_ty = self
            .cx
            .basic_type(result_ty)
            .ok_or_else(|| not_yet_supported(&format!("type `{}`", self.cx.tcx.display(result_ty)), expr.span))?;
        let phi = self
            .cx
            .builder
            .build_phi(llvm_ty.as_basic_type_enum(), "elvis")?;
        phi.add_incoming(&[(&rhs_val, rhs_end), (&lhs_unwrapped, lhs_end)]);
        Ok(phi.as_basic_value())
    }

    pub(super) fn emit_nullable_eq(
        &mut self,
        op: &BinOp,
        lhs: &HirExpr,
        rhs: &HirExpr,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        expr: &HirExpr,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let eq = *op == BinOp::Eq;
        let cx = self.cx;
        let i1 = cx.context.bool_type();

        let lhs_null = matches!(lhs.kind, HirExprKind::None);
        let rhs_null = matches!(rhs.kind, HirExprKind::None);

        let result = if lhs_null && rhs_null {
            i1.const_int(eq as u64, false)
        } else if lhs_null {
            let is_null = self.emit_nullable_is_null(rhs.ty, rhs_val)?;
            self.eq_or_ne(eq, is_null)?
        } else if rhs_null {
            let is_null = self.emit_nullable_is_null(lhs.ty, lhs_val)?;
            self.eq_or_ne(eq, is_null)?
        } else if self.cx.tcx.is_nullable(lhs.ty) || self.cx.tcx.is_nullable(rhs.ty) {
            let ty = if self.cx.tcx.is_nullable(lhs.ty) {
                lhs.ty
            } else {
                rhs.ty
            };
            let l_null = self.emit_nullable_is_null(ty, lhs_val)?;
            let r_null = self.emit_nullable_is_null(ty, rhs_val)?;
            let both_null = cx.builder.build_and(l_null, r_null, "both_null")?;
            let both_some = cx.builder.build_and(
                cx.builder.build_not(l_null, "l.some")?,
                cx.builder.build_not(r_null, "r.some")?,
                "both_some",
            )?;
            let payload_eq = self.emit_nullable_payload_eq(ty, lhs_val, rhs_val, expr.span)?;
            let value_eq = cx.builder.build_and(both_some, payload_eq, "val.eq")?;
            self.eq_or_ne(eq, cx.builder.build_or(both_null, value_eq, "nullable.eq")?)?
        } else {
            return Err(not_yet_supported("nullable equality mismatch", expr.span));
        };

        Ok(result.into())
    }

    fn eq_or_ne(&self, eq: bool, is_eq: IntValue<'ctx>) -> Result<IntValue<'ctx>, Diagnostic> {
        if eq {
            Ok(is_eq)
        } else {
            Ok(self.cx.builder.build_not(is_eq, "nullable.ne")?)
        }
    }

    pub(super) fn emit_buffer_desc_eq(
        &self,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let cx = self.cx;
        let l_desc = lhs_val.into_struct_value();
        let r_desc = rhs_val.into_struct_value();
        let l_ptr = cx
            .builder
            .build_extract_value(l_desc, 0, "l.ptr")?
            .into_pointer_value();
        let r_ptr = cx
            .builder
            .build_extract_value(r_desc, 0, "r.ptr")?
            .into_pointer_value();
        let l_len = cx.builder.build_extract_value(l_desc, 1, "l.len")?;
        let r_len = cx.builder.build_extract_value(r_desc, 1, "r.len")?;
        let ptr_eq = cx
            .builder
            .build_int_compare(IntPredicate::EQ, l_ptr, r_ptr, "ptr.eq")?;
        let len_eq = cx.builder.build_int_compare(
            IntPredicate::EQ,
            l_len.into_int_value(),
            r_len.into_int_value(),
            "len.eq",
        )?;
        Ok(cx.builder.build_and(ptr_eq, len_eq, "desc.eq")?)
    }

    /// Whether the payloads of two nullable values are equal. Only read where both are `Some`
    /// (the caller masks the result), and compared as a scalar or as a buffer descriptor.
    fn emit_nullable_payload_eq(
        &mut self,
        ty: TyId,
        lhs_val: BasicValueEnum<'ctx>,
        rhs_val: BasicValueEnum<'ctx>,
        span: Option<crate::span::Span>,
    ) -> Result<IntValue<'ctx>, Diagnostic> {
        let l_val = self.emit_nullable_unwrap(ty, lhs_val)?;
        let r_val = self.emit_nullable_unwrap(ty, rhs_val)?;
        let inner = self.cx.tcx.base(ty);
        // Each slot owns its own copy of the bytes, so strings compare by content.
        if self.cx.tcx.is_string(inner) {
            return Ok(self.emit_string_eq(l_val, r_val, false)?.into_int_value());
        }
        if self.cx.tcx.is_array(inner) {
            return self.emit_buffer_desc_eq(l_val, r_val);
        }
        if !matches!(self.cx.tcx.data(inner), TyData::Prim(_)) {
            return Err(not_yet_supported(&format!("nullable equality for `{}`", self.cx.tcx.display(ty)), span));
        }
        if matches!(self.cx.tcx.data(inner), TyData::Prim(Prim::F32 | Prim::F64)) {
            let l = self.value_as_float(l_val, inner, span)?;
            let r = self.value_as_float(r_val, inner, span)?;
            return Ok(self.cx.builder.build_float_compare(
                inkwell::FloatPredicate::OEQ,
                l,
                r,
                "feq",
            )?);
        }
        let l = self.value_as_int(l_val, inner, span)?;
        let r = self.value_as_int(r_val, inner, span)?;
        Ok(self
            .cx
            .builder
            .build_int_compare(IntPredicate::EQ, l, r, "ieq")?)
    }

    pub(super) fn emit_safe_nullable_length(
        &mut self,
        receiver_ty: TyId,
        receiver_val: BasicValueEnum<'ctx>,
        result_ty: TyId,
        span: Option<crate::span::Span>,
    ) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let is_null = self.emit_nullable_is_null(receiver_ty, receiver_val)?;
        let null_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "safe.len.null");
        let some_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "safe.len.some");
        let merge_bb = self
            .cx
            .context
            .append_basic_block(self.llvm_fn, "safe.len.end");
        self.cx
            .builder
            .build_conditional_branch(is_null, null_bb, some_bb)?;
        self.cx.builder.position_at_end(null_bb);
        let none_val = self.emit_nullable_none(result_ty)?;
        let none_end = self.cx.builder.get_insert_block().expect("none block");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(some_bb);
        let payload = self.emit_nullable_unwrap(receiver_ty, receiver_val)?;
        let len = self.emit_buffer_length_value(payload.into_struct_value())?;
        let some_val = self.emit_nullable_some(result_ty, len.into())?;
        let some_end = self.cx.builder.get_insert_block().expect("some block");
        self.cx.builder.build_unconditional_branch(merge_bb)?;
        self.cx.builder.position_at_end(merge_bb);
        let llvm_ty = self
            .cx
            .basic_type(result_ty)
            .ok_or_else(|| not_yet_supported(&format!("type `{}`", self.cx.tcx.display(result_ty)), span))?;
        let phi = self
            .cx
            .builder
            .build_phi(llvm_ty.as_basic_type_enum(), "safe.len")?;
        phi.add_incoming(&[(&none_val, none_end), (&some_val, some_end)]);
        Ok(phi.as_basic_value())
    }
}
