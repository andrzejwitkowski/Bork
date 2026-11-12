//! The `Ref` runtime protocol, used by the MIR lowering (`Lower`),
//! `f` is the function being emitted (entry-block allocas).

use inkwell::types::BasicType;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue};
use inkwell::IntPredicate;

use super::context::Codegen;
use crate::diag::Diagnostic;

impl<'ctx> Codegen<'ctx> {
    /// Allocas go at the top of the entry block so each slot is allocated once per call.
    pub(super) fn entry_alloca_in(&self, f: FunctionValue<'ctx>, ty: impl BasicType<'ctx>, name: &str) -> Result<PointerValue<'ctx>, Diagnostic> {
        let entry = f.get_first_basic_block().expect("entry block exists");
        let builder = self.context.create_builder();
        match entry.get_first_instruction() {
            Some(first) => builder.position_before(&first),
            None => builder.position_at_end(entry),
        }
        Ok(builder.build_alloca(ty, name)?)
    }

    fn handle_in(&self, f: FunctionValue<'ctx>, v: BasicValueEnum<'ctx>, name: &str) -> Result<PointerValue<'ctx>, Diagnostic> {
        let slot = self.entry_alloca_in(f, self.ref_handle_type(), name)?;
        self.builder.build_store(slot, v)?;
        Ok(slot)
    }

    pub(super) fn ref_create(&self, f: FunctionValue<'ctx>, source: PointerValue<'ctx>, target: PointerValue<'ctx>, object: PointerValue<'ctx>) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let output = self.entry_alloca_in(f, self.ref_handle_type(), "ref.tmp")?;
        self.builder.build_call(self.ref_create_out_fn(), &[output.into(), source.into(), target.into(), object.into()], "")?;
        Ok(self.builder.build_load(self.ref_handle_type(), output, "ref")?)
    }

    pub(super) fn ref_move_out(&self, f: FunctionValue<'ctx>, slot: PointerValue<'ctx>) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let output = self.entry_alloca_in(f, self.ref_handle_type(), "ref.move.tmp")?;
        self.builder.build_call(self.ref_move_out_fn(), &[output.into(), slot.into()], "")?;
        Ok(self.builder.build_load(self.ref_handle_type(), output, "ref.move")?)
    }

    pub(super) fn ref_clone(&self, f: FunctionValue<'ctx>, reference: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let input = self.handle_in(f, reference, "ref.copy.in")?;
        let output = self.entry_alloca_in(f, self.ref_handle_type(), "ref.copy.out")?;
        self.builder.build_call(self.ref_clone_out_fn(), &[output.into(), input.into()], "")?;
        Ok(self.builder.build_load(self.ref_handle_type(), output, "ref.copy")?)
    }

    pub(super) fn ref_store(&self, f: FunctionValue<'ctx>, slot: PointerValue<'ctx>, owner: PointerValue<'ctx>, reference: BasicValueEnum<'ctx>) -> Result<(), Diagnostic> {
        let incoming = self.handle_in(f, reference, "ref.store.in")?;
        self.builder.build_call(self.ref_store_take_fn(), &[slot.into(), owner.into(), incoming.into()], "")?;
        Ok(())
    }

    pub(super) fn ref_drop(&self, f: FunctionValue<'ctx>, reference: BasicValueEnum<'ctx>) -> Result<(), Diagnostic> {
        let input = self.handle_in(f, reference, "ref.drop")?;
        self.builder.build_call(self.ref_drop_value_fn(), &[input.into()], "")?;
        Ok(())
    }

    pub(super) fn ref_register_drop(&self, owner: PointerValue<'ctx>, slot: PointerValue<'ctx>) -> Result<(), Diagnostic> {
        self.builder.build_call(self.arena_register_ref_drop_fn(), &[owner.into(), slot.into()], "")?;
        Ok(())
    }

    pub(super) fn ref_rehome(&self, f: FunctionValue<'ctx>, owner: PointerValue<'ctx>, reference: BasicValueEnum<'ctx>) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let input = self.handle_in(f, reference, "ref.rehome.in")?;
        let output = self.entry_alloca_in(f, self.ref_handle_type(), "ref.rehome.out")?;
        self.builder.build_call(self.ref_rehome_out_fn(), &[output.into(), owner.into(), input.into()], "")?;
        let rehomed = self.builder.build_load(self.ref_handle_type(), output, "ref.rehome")?;
        self.ref_drop(f, reference)?;
        Ok(rehomed)
    }

    /// `lhs == rhs`; both operands are released afterwards.
    pub(super) fn ref_eq(&self, f: FunctionValue<'ctx>, lhs: BasicValueEnum<'ctx>, rhs: BasicValueEnum<'ctx>, negate: bool) -> Result<IntValue<'ctx>, Diagnostic> {
        let left = self.handle_in(f, lhs, "ref.eq.left")?;
        let right = self.handle_in(f, rhs, "ref.eq.right")?;
        let equal = self
            .builder
            .build_call(self.ref_equal_fn(), &[left.into(), right.into()], "ref.eq")?
            .try_as_basic_value()
            .basic()
            .expect("Ref equality returns bool")
            .into_int_value();
        self.ref_drop(f, lhs)?;
        self.ref_drop(f, rhs)?;
        Ok(if negate { self.builder.build_not(equal, "ref.ne")? } else { equal })
    }

    /// `bork_ref_is_none`; does not release `reference`.
    pub(super) fn ref_is_none(&self, f: FunctionValue<'ctx>, reference: BasicValueEnum<'ctx>) -> Result<IntValue<'ctx>, Diagnostic> {
        let input = self.handle_in(f, reference, "ref.none.in")?;
        Ok(self
            .builder
            .build_call(self.ref_is_none_fn(), &[input.into()], "ref.none")?
            .try_as_basic_value()
            .basic()
            .expect("bork_ref_is_none returns bool")
            .into_int_value())
    }

    /// `strongCount(r)` / `weakCount(r)`. The argument is released before the counter is read so
    /// a temporary copy of `r` is not counted. Releasing the last reference can free the arena's
    /// control record, so a weak lease pins it for the read; its own +1 is removed from
    /// `weakCount`. A `None` handle has a null control, takes no lease and reads as 0.
    pub(super) fn ref_count(&self, f: FunctionValue<'ctx>, reference: BasicValueEnum<'ctx>, weak: bool) -> Result<BasicValueEnum<'ctx>, Diagnostic> {
        let control = self.builder.build_extract_value(reference.into_struct_value(), 0, "ref.control")?;
        let leased = self
            .builder
            .build_call(self.arena_count_lease_retain_fn(), &[control.into()], "ref.count.lease")?
            .try_as_basic_value()
            .basic()
            .expect("lease retain returns a flag")
            .into_int_value();
        self.ref_drop(f, reference)?;
        let mut count = self
            .builder
            .build_call(self.arena_count_fn(weak), &[control.into()], "ref.count")?
            .try_as_basic_value()
            .basic()
            .expect("count returns usize")
            .into_int_value();
        if weak {
            let without_lease = self.builder.build_int_sub(count, count.get_type().const_int(1, false), "ref.weak.unleased")?;
            count = self.builder.build_select(leased, without_lease, count, "ref.weak")?.into_int_value();
        }
        self.builder.build_call(self.arena_count_lease_release_fn(), &[control.into()], "")?;
        Ok(self.builder.build_int_truncate_or_bit_cast(count, self.context.i32_type(), "ref.count.i32")?.into())
    }

    /// `bork_ref_observe`: the observation, its object pointer and whether the target is gone.
    pub(super) fn ref_observe(&self, f: FunctionValue<'ctx>, reference: BasicValueEnum<'ctx>) -> Result<(StructValue<'ctx>, PointerValue<'ctx>, IntValue<'ctx>), Diagnostic> {
        let input = self.handle_in(f, reference, "ref.observe.in")?;
        let output = self.entry_alloca_in(f, self.ref_observation_type(), "ref.observe.out")?;
        self.builder.build_call(self.ref_observe_fn(), &[output.into(), input.into()], "")?;
        let observation = self.builder.build_load(self.ref_observation_type(), output, "ref.observe")?.into_struct_value();
        let object = self.builder.build_extract_value(observation, 0, "ref.object")?.into_pointer_value();
        let is_absent = self.builder.build_int_compare(IntPredicate::EQ, object, object.get_type().const_null(), "ref.absent")?;
        Ok((observation, object, is_absent))
    }
}
