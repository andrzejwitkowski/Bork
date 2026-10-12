//! Source metadata and labels for the optional runtime debug registry.
mod hooks;
mod scopes;
mod types;

use super::context::Codegen;
use crate::{
    hir::{HirFunction, Ty},
    span::Span,
};
use inkwell::{
    debug_info::{
        AsDIScope, DIFile, DIFlags, DIFlagsConstants, DIScope, DIType, DWARFEmissionKind,
        DWARFSourceLanguage, DebugInfoBuilder,
    },
    module::FlagBehavior,
};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::Path,
};

pub(super) struct DebugInfo<'ctx> {
    pub builder: DebugInfoBuilder<'ctx>,
    file: DIFile<'ctx>,
    source: String,
    path: String,
    types: RefCell<HashMap<String, DIType<'ctx>>>,
    pub position: Cell<Span>,
}

impl<'ctx> DebugInfo<'ctx> {
    pub fn new(cx: &Codegen<'ctx>, source: &str, path: &Path) -> Self {
        cx.module.add_basic_value_flag(
            "Debug Info Version",
            FlagBehavior::Warning,
            cx.context.i32_type().const_int(3, false),
        );
        cx.module.add_basic_value_flag(
            "Dwarf Version",
            FlagBehavior::Warning,
            cx.context.i32_type().const_int(4, false),
        );
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir().unwrap_or_default().join(path)
        };
        let (builder, unit) = cx.module.create_debug_info_builder(
            true,
            DWARFSourceLanguage::C,
            &absolute.file_name().unwrap_or_default().to_string_lossy(),
            &absolute
                .parent()
                .unwrap_or(Path::new("."))
                .to_string_lossy(),
            "Bork",
            false,
            "-g",
            0,
            "",
            DWARFEmissionKind::Full,
            0,
            false,
            false,
            "",
            "",
        );
        Self {
            builder,
            file: unit.get_file(),
            source: source.into(),
            path: absolute.to_string_lossy().into_owned(),
            types: RefCell::new(HashMap::new()),
            position: Cell::new(Span::default()),
        }
    }

    fn line_column(&self, span: Span) -> (u32, u32) {
        let prefix = &self.source.as_bytes()[..span.start.min(self.source.len())];
        let line = prefix.iter().filter(|&&b| b == b'\n').count() + 1;
        let col = prefix
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(prefix.len() + 1, |p| prefix.len() - p);
        (line as u32, col as u32)
    }

    pub fn function(
        &self,
        cx: &Codegen<'ctx>,
        function: &HirFunction,
        llvm: inkwell::values::FunctionValue<'ctx>,
    ) -> DIScope<'ctx> {
        let (line, _) = self.line_column(function.name_span);
        let params: Vec<_> = function.params.iter().map(|p| self.ty(cx, &p.ty)).collect();
        let result = (function.return_ty != Ty::unit()).then(|| self.ty(cx, &function.return_ty));
        let sig = self
            .builder
            .create_subroutine_type(self.file, result, &params, DIFlags::PUBLIC);
        let sub = self.builder.create_function(
            self.file.as_debug_info_scope(),
            &function.name,
            None,
            self.file,
            line,
            sig,
            function.name != "main",
            true,
            line,
            DIFlags::PUBLIC,
            false,
        );
        llvm.set_subprogram(sub);
        self.position.set(function.name_span);
        sub.as_debug_info_scope()
    }
}
