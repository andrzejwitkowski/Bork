//! Push-flag stamping: marks which function roots need `arena_push` after typeck.

use super::{walk_program, RegionSite, RegionVisitor, WalkError};
use crate::hir::{HirBlock, HirConditionalBinding, HirProgram};
use crate::sema::ArenaNode;

pub(super) struct StampVisitor;

impl RegionVisitor for StampVisitor {
    fn touch_codegen_push(&self) -> bool {
        true
    }

    fn on_function_root_mut(&mut self, root: &mut ArenaNode) -> Result<(), WalkError> {
        root.codegen_push = true;
        Ok(())
    }

    fn enter_region(
        &mut self,
        _site: RegionSite,
        _child: &ArenaNode,
        _body: &HirBlock,
    ) -> Result<(), WalkError> {
        Ok(())
    }

    fn loop_latch(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        Ok(())
    }

    fn exit_region(&mut self, _site: RegionSite) -> Result<(), WalkError> {
        Ok(())
    }

    fn skip_closure(&mut self, _child: &ArenaNode) -> Result<(), WalkError> {
        Ok(())
    }

    fn bind_presence_guard(&mut self, _binding: &HirConditionalBinding) -> Result<(), WalkError> {
        Ok(())
    }
}

pub fn stamp_codegen_push(program: &HirProgram, report: &mut crate::sema::ArenaReport) {
    let mut visitor = StampVisitor;
    walk_program(program, &mut report.roots, &mut visitor).expect("stamp walk");
}
