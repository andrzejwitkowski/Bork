//! Struct declaration checking and cycle detection.

use std::collections::{HashMap, HashSet};

use crate::ast::{Program, StructDecl, Type};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirStructDef, HirStructField, Ty, TyKind};
use crate::span::Span;

pub(super) fn build_struct_env(
    program: &Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> HashMap<String, HirStructDef> {
    reject_bad_struct_names(program, diagnostics);

    let known: HashSet<String> = program.structs.iter().map(|s| s.name.clone()).collect();
    let mut defs = HashMap::new();
    for decl in &program.structs {
        defs.insert(decl.name.clone(), lower_struct_decl(decl, &known, diagnostics));
    }
    reject_struct_cycles(&defs, diagnostics);
    defs
}

fn reject_bad_struct_names(program: &Program, diagnostics: &mut Vec<Diagnostic>) {
    let mut names = HashSet::new();
    for decl in &program.structs {
        if !names.insert(decl.name.clone()) {
            push_err(diagnostics, format!("duplicate struct `{}`", decl.name), None);
        }
        if program.functions.iter().any(|f| f.name == decl.name)
            || crate::builtins::is_intrinsic(&decl.name)
        {
            push_err(
                diagnostics,
                format!("struct `{}` conflicts with a function name", decl.name),
                None,
            );
        }
    }
}

fn lower_struct_decl(
    decl: &StructDecl,
    known: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> HirStructDef {
    let mut seen_fields = HashSet::new();
    let mut fields = Vec::new();
    for field in &decl.fields {
        if !seen_fields.insert(field.name.name.clone()) {
            push_err(
                diagnostics,
                format!(
                    "duplicate field `{}` in struct `{}`",
                    field.name.name, decl.name
                ),
                Some(field.name.span),
            );
        }
        let ty = lower_struct_field_type(&field.ty, known, diagnostics, Some(field.name.span));
        validate_struct_field(&field.name.name, &ty, field.name.span, diagnostics);
        fields.push(HirStructField {
            kind: field.kind,
            name: field.name.name.clone(),
            ty,
        });
    }
    HirStructDef {
        name: decl.name.clone(),
        fields,
    }
}

fn validate_struct_field(
    name: &str,
    ty: &Ty,
    span: Span,
    diagnostics: &mut Vec<Diagnostic>,
) {
    if !ty.is_unknown() && !ty.is_struct_field_allowed() {
        push_err(
            diagnostics,
            format!(
                "struct field `{name}` has unsupported type `{ty}` (only primitives, structs, and their `?` forms)"
            ),
            Some(span),
        );
    }
}

fn lower_struct_field_type(
    ty: &Type,
    known: &HashSet<String>,
    diagnostics: &mut Vec<Diagnostic>,
    span: Option<Span>,
) -> Ty {
    match ty {
        Type::Primitive { name, nullable } => Ty::new(
            crate::hir::Prim::from_name(name).map_or(TyKind::Unknown, TyKind::Prim),
            *nullable,
        ),
        Type::Named { name, nullable } if name == "String" => {
            push_err(
                diagnostics,
                "struct fields of type `String` are not supported yet",
                span,
            );
            Ty::string(*nullable)
        }
        Type::Named { name, nullable } if known.contains(name) => {
            Ty::new(TyKind::Struct(name.clone()), *nullable)
        }
        Type::Named { name, .. } => {
            push_err(diagnostics, format!("unknown named type `{name}`"), span);
            Ty::unknown()
        }
        Type::Array { .. } => {
            push_err(
                diagnostics,
                "struct fields of array type are not supported yet",
                span,
            );
            Ty::unknown()
        }
        Type::Ref { .. } => {
            push_err(
                diagnostics,
                "struct fields of reference type are not supported",
                span,
            );
            Ty::unknown()
        }
        Type::Func { .. } => {
            push_err(
                diagnostics,
                "struct fields of function type are not supported",
                span,
            );
            Ty::unknown()
        }
    }
}

fn reject_struct_cycles(defs: &HashMap<String, HirStructDef>, diagnostics: &mut Vec<Diagnostic>) {
    let mut visiting = HashSet::new();
    let mut done = HashSet::new();
    for name in defs.keys() {
        if detect_cycle(name, defs, &mut visiting, &mut done) {
            push_err(
                diagnostics,
                format!("struct `{name}` is part of a cyclic definition"),
                None,
            );
        }
    }
}

fn detect_cycle(
    name: &str,
    defs: &HashMap<String, HirStructDef>,
    visiting: &mut HashSet<String>,
    done: &mut HashSet<String>,
) -> bool {
    if done.contains(name) {
        return false;
    }
    if !visiting.insert(name.to_string()) {
        return true;
    }
    let cyclic = defs.get(name).is_some_and(|def| {
        def.fields.iter().any(|f| {
            f.ty.struct_name()
                .is_some_and(|dep| detect_cycle(dep, defs, visiting, done))
        })
    });
    visiting.remove(name);
    done.insert(name.to_string());
    cyclic
}

fn push_err(diagnostics: &mut Vec<Diagnostic>, message: impl Into<String>, span: Option<Span>) {
    diagnostics.push(Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: message.into(),
        span,
    });
}
