//! Source metadata and labels for the optional runtime debug registry.
pub(super) mod checks;
#[cfg(test)]
mod diag_tests;
mod hooks;
mod scopes;
mod types;

use super::context::Codegen;
use crate::{hir::HirFunction, span::Span};
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
    active_source: Cell<Option<&'static str>>,
    last_stamped: RefCell<
        HashMap<inkwell::basic_block::BasicBlock<'ctx>, inkwell::values::InstructionValue<'ctx>>,
    >,
    function_spans: RefCell<HashMap<String, (Span, Option<&'static str>)>>,
    locations:
        RefCell<HashMap<(Option<&'static str>, usize, usize), inkwell::values::PointerValue<'ctx>>>,
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
            active_source: Cell::new(None),
            last_stamped: RefCell::new(HashMap::new()),
            function_spans: RefCell::new(HashMap::new()),
            locations: RefCell::new(HashMap::new()),
        }
    }

    fn line_column(&self, span: Span) -> (u32, u32) {
        let (_, source) = self.source_parts();
        let prefix = &source.as_bytes()[..span.start.min(source.len())];
        let line = prefix.iter().filter(|&&b| b == b'\n').count() + 1;
        let col = prefix
            .iter()
            .rposition(|&b| b == b'\n')
            .map_or(prefix.len() + 1, |p| prefix.len() - p);
        (line as u32, col as u32)
    }

    fn source_parts(&self) -> (&str, &str) {
        self.active_source
            .get()
            .map_or((&*self.path, &*self.source), |path| {
                (path, crate::prelude::source_text(path))
            })
    }

    fn source_file(&self) -> DIFile<'ctx> {
        match self.active_source.get() {
            None => self.file,
            Some(path) => {
                let path = Path::new(path);
                self.builder.create_file(
                    &path.file_name().unwrap().to_string_lossy(),
                    &path.parent().unwrap().to_string_lossy(),
                )
            }
        }
    }

    pub fn function(
        &self,
        cx: &Codegen<'ctx>,
        function: &HirFunction,
        llvm: inkwell::values::FunctionValue<'ctx>,
    ) -> DIScope<'ctx> {
        self.active_source.set(function.source);
        let file = self.source_file();
        let (line, _) = self.line_column(function.name_span);
        let params: Vec<_> = function.params.iter().map(|p| self.ty(cx, p.ty)).collect();
        let result = (function.return_ty != cx.unit_ty()).then(|| self.ty(cx, function.return_ty));
        let sig = self
            .builder
            .create_subroutine_type(file, result, &params, DIFlags::PUBLIC);
        let sub = self.builder.create_function(
            file.as_debug_info_scope(),
            &function.name,
            None,
            file,
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
        self.function_spans.borrow_mut().insert(
            llvm.get_name().to_string_lossy().into_owned(),
            (function.name_span, function.source),
        );
        sub.as_debug_info_scope()
    }
}
