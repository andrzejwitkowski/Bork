mod env;
mod expr;
mod lower;
mod stmt;

use std::collections::{HashMap, HashSet};

use crate::ast::{Function, Program};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirClass, HirClassField, HirFunction, HirParam, HirProgram, Ty, TyKind};
use crate::span::Span;

use env::{ClassFieldInfo, ClassInfo, ClassTable, Env};
pub(crate) use env::FunSig;
use lower::lower_type;

pub type DeclTypes = HashMap<Span, Ty>;

pub fn check(program: &Program) -> (HirProgram, DeclTypes, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    reject_builtin_redefines(program, &mut diagnostics);

    let classes = build_classes(program, &mut diagnostics);
    validate_owned_layouts(&classes, &mut diagnostics);
    let fun_sigs = collect_fun_sigs(program, &classes, &mut diagnostics);

    let mut decl_tys = DeclTypes::new();
    let functions = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| {
            check_function(
                function,
                &fun_sigs,
                &classes,
                &mut diagnostics,
                &mut decl_tys,
            )
        })
        .collect();

    let hir_classes = program
        .classes
        .iter()
        .filter_map(|class| classes.get(&class.name))
        .map(|class| HirClass {
            name: class.name.clone(),
            fields: class
                .fields
                .iter()
                .map(|field| HirClassField {
                    name: field.name.clone(),
                    ty: field.ty.clone(),
                })
                .collect(),
        })
        .collect();
    (
        HirProgram {
            classes: hir_classes,
            functions,
        },
        decl_tys,
        diagnostics,
    )
}

fn reject_builtin_redefines(program: &Program, diagnostics: &mut Vec<Diagnostic>) {
    for function in &program.functions {
        if crate::builtins::is_intrinsic(&function.name) {
            diagnostics.push(Diagnostic {
                phase: Phase::Type,
                severity: Severity::Error,
                message: format!("cannot redefine builtin function `{}`", function.name),
                span: builtin_redefine_span(function),
            });
        }
    }
}

fn build_classes(
    program: &Program,
    diagnostics: &mut Vec<Diagnostic>,
) -> ClassTable {
    let mut classes = ClassTable::new();
    for class in &program.classes {
        if class.name == "String"
            || !matches!(
                crate::ast::Type::from_ident(&class.name, false),
                crate::ast::Type::Named { .. }
            )
        {
            diagnostics.push(type_error(
                format!("class `{}` conflicts with a builtin type name", class.name),
                class.fields.first().map(|field| field.name.span),
            ));
        }
        if classes.contains_key(&class.name) {
            diagnostics.push(type_error(
                format!("duplicate class `{}`", class.name),
                class.fields.first().map(|field| field.name.span),
            ));
            continue;
        }
        classes.insert(
            class.name.clone(),
            ClassInfo {
                name: class.name.clone(),
                fields: Vec::new(),
            },
        );
    }
    let mut filled = HashSet::new();
    for class in &program.classes {
        if !filled.insert(class.name.as_str()) {
            continue;
        }
        let mut field_names = HashSet::new();
        let fields = class
            .fields
            .iter()
            .map(|field| {
                if !field_names.insert(field.name.name.clone()) {
                    diagnostics.push(type_error(
                        format!(
                            "duplicate field `{}` in class `{}`",
                            field.name.name, class.name
                        ),
                        Some(field.name.span),
                    ));
                }
                // shortcut: borrow fields are rejected, allow them once aggregate lifetimes are tracked.
                if field.ty.is_reference() {
                    diagnostics.push(type_error(
                        "borrowed references cannot be stored in class fields",
                        Some(field.name.span),
                    ));
                }
                ClassFieldInfo {
                    name: field.name.name.clone(),
                    ty: lower_type(&field.ty, &classes, diagnostics),
                    span: field.name.span,
                }
            })
            .collect();
        if let Some(info) = classes.get_mut(&class.name) {
            info.fields = fields;
        }
    }
    classes
}

fn collect_fun_sigs(
    program: &Program,
    classes: &ClassTable,
    diagnostics: &mut Vec<Diagnostic>,
) -> HashMap<String, FunSig> {
    let mut fun_sigs: HashMap<_, _> = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| {
            (
                function.name.clone(),
                FunSig {
                    params: function
                        .params
                        .iter()
                        .map(|param| lower_type(&param.ty, classes, diagnostics))
                        .collect(),
                    return_ty: lower_type(&function.return_type, classes, diagnostics),
                },
            )
        })
        .collect();
    fun_sigs.extend(crate::builtins::signatures());
    for class in &program.classes {
        if fun_sigs.contains_key(&class.name) {
            diagnostics.push(type_error(
                format!("class `{}` conflicts with another type or function", class.name),
                class.fields.first().map(|field| field.name.span),
            ));
        }
    }
    fun_sigs
}

fn check_function(
    function: &Function,
    fun_sigs: &HashMap<String, FunSig>,
    classes: &ClassTable,
    diagnostics: &mut Vec<Diagnostic>,
    decl_tys: &mut DeclTypes,
) -> HirFunction {
    let mut env = Env::new(fun_sigs, classes);
    let signature = env
        .fun_sigs
        .get(&function.name)
        .cloned()
        .expect("signature was collected for every function");
    let params = bind_params(function, signature.params, &mut env);
    let return_ty = signature.return_ty;
    let body = stmt::check_block(&function.body, &return_ty, &mut env, false);
    if return_ty != Ty::unit() && !stmt::block_always_returns(&body) {
        env.error(
            format!("function must return a value of type {return_ty} on all paths"),
            None,
        );
    }
    diagnostics.append(&mut env.diagnostics);
    decl_tys.extend(env.decl_tys);
    HirFunction {
        name: function.name.clone(),
        params,
        return_ty,
        body,
    }
}

fn bind_params(
    function: &Function,
    param_tys: Vec<Ty>,
    env: &mut Env<'_>,
) -> Vec<HirParam> {
    function
        .params
        .iter()
        .zip(param_tys)
        .map(|(param, ty)| {
            if ty.is_ref() && param.kind == crate::ast::BindingKind::Var {
                env.error(
                    "reference parameter must be `val`",
                    Some(param.name.span),
                );
            }
            env.bind(
                param.name.name.clone(),
                param.kind,
                ty.clone(),
                ty.is_ref(),
            );
            HirParam {
                kind: param.kind,
                name: param.name.name.clone(),
                ty,
            }
        })
        .collect()
}

fn type_error(message: impl Into<String>, span: Option<Span>) -> Diagnostic {
    Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: message.into(),
        span,
    }
}

fn validate_owned_layouts(classes: &ClassTable, diagnostics: &mut Vec<Diagnostic>) {
    fn visits(
        origin: &str,
        current: &str,
        classes: &ClassTable,
        seen: &mut HashSet<String>,
    ) -> bool {
        if !seen.insert(current.to_string()) {
            return false;
        }
        let Some(class) = classes.get(current) else {
            return false;
        };
        class.fields.iter().any(|field| {
            let target = match &field.ty.kind {
                TyKind::Named(name) if name != "String" => Some(name.as_str()),
                TyKind::Array { elem, .. } => match &elem.kind {
                    TyKind::Named(name) if name != "String" => Some(name.as_str()),
                    _ => None,
                },
                _ => None,
            };
            target.is_some_and(|target| {
                target == origin || visits(origin, target, classes, &mut seen.clone())
            })
        })
    }

    for class in classes.values() {
        if visits(&class.name, &class.name, classes, &mut HashSet::new()) {
            diagnostics.push(type_error(
                format!(
                    "class `{}` has a recursive by-value layout; use `Ref<{}>`",
                    class.name, class.name
                ),
                class.fields.first().map(|field| field.span),
            ));
        }
    }
}

fn builtin_redefine_span(function: &Function) -> Option<Span> {
    function.params.first().map(|param| param.name.span)
}

#[cfg(test)]
mod tests;
