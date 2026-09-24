use inkwell::builder::BuilderError;
use inkwell::values::PointerValue;

use crate::codegen::regions::RegionSink;
use crate::sema::ArenaNode;

use super::context::Codegen;

/// Lowers region events to `bork_arena_*` runtime calls at the builder's position.
///
/// `handles` mirrors the open regions of the function being emitted so an early `return`
/// can pop every live arena. Resets and pops at unreachable insertion points are skipped.
pub struct ArenaCalls<'a, 'ctx> {
    cx: &'a Codegen<'ctx>,
    handles: Vec<PointerValue<'ctx>>,
    error: Option<BuilderError>,
}

impl<'a, 'ctx> ArenaCalls<'a, 'ctx> {
    pub fn new(cx: &'a Codegen<'ctx>) -> Self {
        Self {
            cx,
            handles: Vec::new(),
            error: None,
        }
    }

    /// Builder failure from the last push/pop; `RegionSink` methods cannot return it.
    pub fn take_error(&mut self) -> Result<(), BuilderError> {
        self.error.take().map_or(Ok(()), Err)
    }

    /// Handle of the innermost open arena, where new allocations go.
    pub fn current(&self) -> Option<PointerValue<'ctx>> {
        self.handles.last().copied()
    }

    pub fn root(&self) -> Option<PointerValue<'ctx>> {
        self.handles.first().copied()
    }

    /// Pops every open arena, innermost first, ahead of a `return`.
    pub fn unwind(&self) -> Result<(), BuilderError> {
        for handle in self.handles.iter().rev() {
            self.build_pop(*handle)?;
        }
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
    fn push(&mut self, _node: &ArenaNode) {
        let call = self
            .cx
            .builder
            .build_call(self.cx.arena_push_fn(), &[], "arena");
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

    fn reset(&mut self, _node: &ArenaNode) {
        let Some(&handle) = self.handles.last() else {
            return;
        };
        if !self.cx.insertion_is_dead() {
            let result = self
                .cx
                .builder
                .build_call(self.cx.arena_reset_fn(), &[handle.into()], "")
                .map(|_| ());
            self.record(result);
        }
    }

    fn pop(&mut self, _node: &ArenaNode) {
        let Some(handle) = self.handles.pop() else {
            return;
        };
        if !self.cx.insertion_is_dead() {
            let result = self.build_pop(handle);
            self.record(result);
        }
    }
}
