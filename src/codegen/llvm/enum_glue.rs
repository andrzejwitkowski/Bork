//! Drop glue for enum values: one internal function per enum with a `Ref` payload.

use inkwell::module::Linkage;
use inkwell::values::FunctionValue;
use inkwell::{AddressSpace, IntPredicate};

use super::context::Codegen;
use crate::hir::HirEnum;

impl<'ctx> Codegen<'ctx> {
    /// Drop glue for enum `e`: switches on the tag and releases the `Ref` payloads of that variant
    /// only, so a slot never releases a payload another variant wrote. ``None` without `Ref` payloads.
    pub(super) fn emit_enum_drop(&self, e: &HirEnum) -> Result<Option<FunctionValue<'ctx>>, String> {
        let has_ref = |v: &crate::hir::HirVariant| v.fields.iter().any(|f| self.tcx.is_managed_ref(f.ty));
        if !e.variants.iter().any(has_ref) {
            return Ok(None);
        }
        let layout = &self.enum_layouts[&e.name];
        let ptr = self.context.ptr_type(AddressSpace::default());
        let function = self.module.add_function(
            &format!("bork.drop.{}", e.name),
            self.context.void_type().fn_type(&[ptr.into()], false),
            Some(Linkage::Internal),
        );
        let emitted = (|| -> Result<(), inkwell::builder::BuilderError> {
            // A separate builder: construction code is mid-function on `self.builder`.
            let builder = self.context.create_builder();
            let entry = self.context.append_basic_block(function, "entry");
            let done = self.context.append_basic_block(function, "done");
            builder.position_at_end(entry);
            let slot = function.get_first_param().expect("one parameter").into_pointer_value();
            let i32_ty = self.context.i32_type();
            let tag_ptr = builder.build_struct_gep(layout.variants[0], slot, 0, "tag.ptr")?;
            let tag = builder.build_load(i32_ty, tag_ptr, "tag")?.into_int_value();
            let mut next = entry;
            for (index, variant) in e.variants.iter().enumerate().filter(|(_, v)| has_ref(v)) {
                let arm = self.context.append_basic_block(function, &format!("variant.{index}"));
                let after = self.context.append_basic_block(function, &format!("after.{index}"));
                builder.position_at_end(next);
                let hit = builder
                    .build_int_compare(IntPredicate::EQ, tag, i32_ty.const_int(index as u64, false), "is.variant")
                    ?;
                builder.build_conditional_branch(hit, arm, after)?;
                builder.position_at_end(arm);
                for (field_index, _) in variant.fields.iter().enumerate().filter(|(_, f)| self.tcx.is_managed_ref(f.ty)) {
                    let field_ptr = builder
                        .build_struct_gep(layout.variants[index], slot, field_index as u32 + 1, "payload")
                        ?;
                    builder.build_call(self.arena_handle_fn("bork_ref_drop"), &[field_ptr.into()], "")?;
                }
                builder.build_unconditional_branch(done)?;
                next = after;
            }
            builder.position_at_end(next);
            builder.build_unconditional_branch(done)?;
            builder.position_at_end(done);
            builder.build_return(None)?;
            Ok(())
        })();
        emitted.map_err(|err| format!("enum `{}` drop glue: {err}", e.name))?;
        Ok(Some(function))
    }
}
