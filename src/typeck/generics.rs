//! Type-parameter scopes and validation of generic declarations (#43).

use crate::ast::{BindingKind, Function, Program, Type, TypeParam};
use crate::defs::DefTable;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirFunction, HirParam};
use crate::ty::{FnSig, ParamIdx, TyCtx, TyId};

/// Parameters visible in a signature: the owner class's, then the item's own, numbered in order.
pub(super) fn scope(outer: &[TypeParam], own: &[TypeParam]) -> Vec<(String, ParamIdx)> {
    outer
        .iter()
        .chain(own)
        .enumerate()
        .map(|(index, param)| (param.name.name.clone(), ParamIdx(index as u32)))
        .collect()
}

pub(super) fn names(outer: &[TypeParam], own: &[TypeParam]) -> Vec<String> {
    outer.iter().chain(own).map(|param| param.name.name.clone()).collect()
}

pub(super) fn validate(program: &Program, defs: &DefTable, diagnostics: &mut Vec<Diagnostic>) {
    for function in &program.functions {
        check_list(&[], &function.generics, defs, diagnostics);
    }
    for class in &program.classes {
        check_list(&[], &class.generics, defs, diagnostics);
        if let Some(first) = class.generics.first() {
            // Keeps `Param` field types away from sema and codegen until #44.
            diagnostics.push(Diagnostic {
                phase: Phase::Type,
                severity: Severity::Error,
                message: format!("generic class `{}` is declaration-only; its use is not supported yet", class.name),
                span: Some(first.name.span),
            });
        }
        for method in &class.methods {
            check_list(&class.generics, &method.generics, defs, diagnostics);
        }
    }
}

fn check_list(outer: &[TypeParam], own: &[TypeParam], defs: &DefTable, diagnostics: &mut Vec<Diagnostic>) {
    for (index, param) in own.iter().enumerate() {
        let name = &param.name.name;
        let message = if own[..index].iter().any(|earlier| earlier.name.name == *name) {
            format!("duplicate type parameter `{name}`")
        } else if outer.iter().any(|class_param| class_param.name.name == *name) {
            format!("type parameter `{name}` shadows the class type parameter `{name}`")
        } else if name == "String"
            || defs.lookup_class(name).is_some()
            || !matches!(Type::from_ident(name, false), Type::Named { .. })
        {
            format!("type parameter `{name}` conflicts with the type `{name}`")
        } else {
            continue;
        };
        diagnostics.push(Diagnostic {
            phase: Phase::Type,
            severity: Severity::Error,
            message,
            span: Some(param.name.span),
        });
    }
}

/// HIR for a generic function or method without its body: bodies see `Param` types only once
/// #44 can check them, so the item reports a diagnostic and keeps just the declaration.
pub(super) fn declaration_only(
    function: &Function,
    is_method: bool,
    name: String,
    generics: Vec<String>,
    signature: FnSig<TyId>,
    tcx: &TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> HirFunction {
    diagnostics.push(Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: format!("the body of generic `{name}` is not checked yet; generic items are declaration-only"),
        span: Some(function.name_span),
    });
    let mut tys = signature.params.into_iter();
    let mut params = Vec::new();
    if is_method {
        let ty = tys.next().expect("methods have `this`");
        params.push(HirParam { kind: BindingKind::Val, name: "this".to_string(), ty });
    }
    for (param, ty) in function.params.iter().zip(tys) {
        super::check_ref_param_kind(param, tcx.is_ref(ty), diagnostics);
        params.push(HirParam { kind: param.kind, name: param.name.name.clone(), ty });
    }
    HirFunction {
        name,
        generics,
        name_span: function.name_span,
        params,
        return_ty: signature.ret,
        body: Default::default(),
    }
}
