use inkwell::types::BasicMetadataTypeEnum;
use inkwell::values::{BasicMetadataValueEnum, PointerValue};
use inkwell::AddressSpace;

use crate::hir::{HirFunction, Ty};

use super::context::Codegen;

/// `main` is a C function. Other functions take the caller's lexical arena,
/// and an owning return also takes the caller's result sink.
#[derive(Clone, Copy)]
pub(super) enum CallAbi {
    Main,
    Callee,
    OwningReturn,
}

impl CallAbi {
    pub(super) fn of(function: &HirFunction) -> Self {
        if function.name == "main" {
            Self::Main
        } else if owns_result(&function.return_ty) {
            Self::OwningReturn
        } else {
            Self::Callee
        }
    }

    pub(super) fn hidden_len(self) -> usize {
        match self {
            Self::Main => 0,
            Self::Callee => 1,
            Self::OwningReturn => 2,
        }
    }

    pub(super) fn has_result_sink(self) -> bool {
        matches!(self, Self::OwningReturn)
    }

    pub(super) fn hidden_types<'ctx>(
        self,
        cx: &Codegen<'ctx>,
    ) -> Vec<BasicMetadataTypeEnum<'ctx>> {
        let ptr = cx.context.ptr_type(AddressSpace::default());
        self.hidden(ptr.into(), ptr.into())
    }

    pub(super) fn hidden_values<'ctx>(
        self,
        parent: PointerValue<'ctx>,
        sink: PointerValue<'ctx>,
    ) -> Vec<BasicMetadataValueEnum<'ctx>> {
        self.hidden(parent.into(), sink.into())
    }

    fn hidden<T>(self, parent: T, sink: T) -> Vec<T> {
        match self {
            Self::Main => Vec::new(),
            Self::Callee => vec![parent],
            Self::OwningReturn => vec![parent, sink],
        }
    }
}

fn owns_result(ty: &Ty) -> bool {
    ty.is_managed_ref() || ty.uses_arena_storage()
}
