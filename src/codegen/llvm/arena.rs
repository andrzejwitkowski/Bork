use inkwell::basic_block::BasicBlock;
use inkwell::builder::BuilderError;
use inkwell::values::{PhiValue, PointerValue};
use inkwell::AddressSpace;

use super::context::Codegen;
use crate::codegen::regions::RegionSink;

struct LoopGeneration<'ctx> {
    index: usize,
    phi: PhiValue<'ctx>,
}

/// Lowers region events to `bork_arena_*` runtime calls at the builder's position.
///
/// `handles` mirrors the open regions of the function being emitted so an early `return`
/// can pop every live arena. Resets and pops at unreachable insertion points are skipped.
pub struct ArenaCalls<'a, 'ctx> {
    cx: &'a Codegen<'ctx>,
    handles: Vec<PointerValue<'ctx>>,
    loop_gens: Vec<LoopGeneration<'ctx>>,
    caller_parent: Option<PointerValue<'ctx>>,
    error: Option<BuilderError>,
}

impl<'a, 'ctx> ArenaCalls<'a, 'ctx> {
    pub fn new(cx: &'a Codegen<'ctx>) -> Self {
        Self {
            cx,
            handles: Vec::new(),
            loop_gens: Vec::new(),
            caller_parent: None,
            error: None,
        }
    }

    pub fn set_caller_parent(&mut self, parent: PointerValue<'ctx>) {
        self.caller_parent = Some(parent);
    }

    pub fn clear_caller_parent(&mut self) {
        self.caller_parent = None;
    }

    pub fn unwind_to_depth(&mut self, depth: usize) -> Result<(), BuilderError> {
        let extra: Vec<_> = self.handles.iter().skip(depth).rev().copied().collect();
        for handle in extra {
            if !self.cx.insertion_is_dead() {
                self.build_pop(handle)?;
            }
        }
        Ok(())
    }

    pub fn handle_count(&self) -> usize {
        self.handles.len()
    }

    /// Phi so the exit `pop` does not use the latch-only reset value.
    pub fn begin_loop_generation(
        &mut self,
        preheader: BasicBlock<'ctx>,
    ) -> Result<(), BuilderError> {
        let index = self
            .handles
            .len()
            .checked_sub(1)
            .expect("loop generation requires an open arena");
        let initial = self.handles[index];
        let ptr = self.cx.context.ptr_type(AddressSpace::default());
        let phi = self.cx.builder.build_phi(ptr, "arena.gen")?;
        phi.add_incoming(&[(&initial, preheader)]);
        self.handles[index] = phi.as_basic_value().into_pointer_value();
        self.loop_gens.push(LoopGeneration { index, phi });
        Ok(())
    }

    /// Builder failure from the last push/pop; `RegionSink` methods cannot return it.
    pub fn take_error(&mut self) -> Result<(), BuilderError> {
        self.error.take().map_or(Ok(()), Err)
    }

    /// Handle of the innermost open arena, where new allocations go.
    pub fn current(&self) -> Option<PointerValue<'ctx>> {
        self.handles.last().copied()
    }

    /// Pops every open arena, innermost first, ahead of a `return`.
    pub fn unwind(&mut self) -> Result<(), BuilderError> {
        for handle in self.handles.iter().rev() {
            self.build_pop(*handle)?;
        }
        // Other exits (fall-through, another `return`) still need these handles for their pops.
        Ok(())
    }

    fn build_pop(&self, handle: PointerValue<'ctx>) -> Result<(), BuilderError> {
        self.cx
            .builder
            .build_call(self.cx.arena_pop_fn(), &[handle.into()], "")
            .map(|_| ())
    }

    fn record(&mut self, result: Result<(), BuilderError>) {
        if let Err(err) = result {
            self.error.get_or_insert(err);
        }
    }
}

impl RegionSink for ArenaCalls<'_, '_> {
    fn push(&mut self, _arena: usize) {
        let parent = self.current().or(self.caller_parent);
        let call = match parent {
            Some(parent) => self.cx.builder.build_call(
                self.cx.arena_push_child_fn(),
                &[parent.into()],
                "arena",
            ),
            None => self
                .cx
                .builder
                .build_call(self.cx.arena_push_fn(), &[], "arena"),
        };
        match call {
            Ok(call) => {
                let handle = call
                    .try_as_basic_value()
                    .basic()
                    .expect("bork_arena_push returns a pointer")
                    .into_pointer_value();
                self.handles.push(handle);
            }
            Err(err) => self.record(Err(err)),
        }
    }

    fn reset(&mut self, _arena: usize) {
        let Some(index) = self.handles.len().checked_sub(1) else {
            return;
        };
        let handle = self.handles[index];
        if !self.cx.insertion_is_dead() {
            let call = self
                .cx
                .builder
                .build_call(self.cx.arena_reset_fn(), &[handle.into()], "arena.reset");
            let result = call.map(|call| {
                let next = call
                    .try_as_basic_value()
                    .basic()
                    .expect("bork_arena_reset returns a pointer")
                    .into_pointer_value();
                if let Some(generation) = self
                    .loop_gens
                    .iter()
                    .rev()
                    .find(|generation| generation.index == index)
                {
                    let latch = self
                        .cx
                        .builder
                        .get_insert_block()
                        .expect("arena reset has an insertion block");
                    generation.phi.add_incoming(&[(&next, latch)]);
                } else {
                    self.handles[index] = next;
                }
            });
            self.record(result)
        }
    }

    fn pop(&mut self, _arena: usize) {
        let Some(index) = self.handles.len().checked_sub(1) else {
            return;
        };
        self.loop_gens
            .retain(|generation| generation.index != index);
        let Some(handle) = self.handles.pop() else {
            return;
        };
        if !self.cx.insertion_is_dead() {
            let result = self.build_pop(handle);
            self.record(result);
        }
    }
}
