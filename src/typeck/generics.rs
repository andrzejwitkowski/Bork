//! Type-parameter scopes and validation of generic declarations (#43).

use crate::ast::{Program, TypeParam};
use crate::defs::DefTable;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::ty::ParamIdx;

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
    for e in &program.enums {
        check_list(&[], &e.generics, defs, diagnostics);
    }
    for t in &program.traits {
        check_list(&[], &t.generics, defs, diagnostics);
        for method in &t.methods {
            check_list(&t.generics, &method.generics, defs, diagnostics);
        }
    }
    for i in &program.impls {
        check_list(&[], &i.generics, defs, diagnostics);
        for method in &i.methods {
            check_list(&i.generics, &method.generics, defs, diagnostics);
        }
    }
    for class in &program.classes {
        check_list(&[], &class.generics, defs, diagnostics);
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
        } else if defs.type_name_taken(name) {
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
