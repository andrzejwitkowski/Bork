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

/// Types used by sema, keyed by declaration names and field assignment spans.
pub type SpanTypes = HashMap<Span, Ty>;

/// What a bare member name or a method-call callee resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberRef {
    /// Bare `name` inside a method, read or assigned through `this`.
    Field { class: String, name: String, index: usize, ty: Ty },
    /// `recv.m(..)` / bare `m(..)` call, or a method declaration name.
    Method { class: String, name: String, signature: String },
    /// Global constant, at its declaration name and at each use.
    Constant { name: String, ty: Ty, value: String },
}

/// Member resolutions keyed by the span of a bare name, a `recv.m` callee, or a method name.
pub type MemberResolutions = HashMap<Span, MemberRef>;

pub fn check(program: &Program) -> (HirProgram, SpanTypes, Vec<Diagnostic>) {
    let (hir, span_tys, _, diagnostics) = check_with_members(program);
    (hir, span_tys, diagnostics)
}

pub fn check_with_members(
    program: &Program,
) -> (HirProgram, SpanTypes, MemberResolutions, Vec<Diagnostic>) {
    let (constants, mut diagnostics) = crate::constants::collect(program);
    reject_builtin_redefines(program, &mut diagnostics);

    let mut classes = build_classes(program, &mut diagnostics);
    validate_owned_layouts(&classes, &mut diagnostics);
    let mut fun_sigs = collect_fun_sigs(program, &classes, &mut diagnostics);
    let mut members = MemberResolutions::new();
    for (name, constant) in &constants {
        members.insert(constant.span, constant.member(name));
    }
    collect_method_sigs(program, &mut classes, &mut fun_sigs, &mut members, &mut diagnostics);
    let mut span_tys = SpanTypes::new();
    let mut functions: Vec<HirFunction> = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| {
            check_function(function, None, &fun_sigs, &classes, &constants, &mut diagnostics, &mut span_tys, &mut members)
        })
        .collect();
    // Same order as sema roots: top-level functions, then methods class by class.
    for class in &program.classes {
        for method in &class.methods {
            functions.push(check_function(
                method,
                Some(&class.name),
                &fun_sigs,
                &classes,
                &constants,
                &mut diagnostics,
                &mut span_tys,
                &mut members,
            ));
        }
    }

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
        span_tys,
        members,
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
                methods: HashMap::new(),
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

/// Registers each method as `C.m` with a leading `this: &C` parameter.
fn collect_method_sigs(
    program: &Program,
    classes: &mut ClassTable,
    fun_sigs: &mut HashMap<String, FunSig>,
    members: &mut MemberResolutions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for class in &program.classes {
        for method in &class.methods {
            if class.fields.iter().any(|field| field.name.name == method.name) {
                diagnostics.push(type_error(
                    format!("class `{}` already has a field named `{}`", class.name, method.name),
                    Some(method.name_span),
                ));
            }
            let qualified = format!("{}.{}", class.name, method.name);
            if fun_sigs.contains_key(&qualified) {
                diagnostics.push(type_error(
                    format!("duplicate method `{}` in class `{}`", method.name, class.name),
                    Some(method.name_span),
                ));
                continue;
            }
            let params: Vec<Ty> = method
                .params
                .iter()
                .map(|param| lower_type(&param.ty, classes, diagnostics))
                .collect();
            let return_ty = lower_type(&method.return_type, classes, diagnostics);
            let shown_params: Vec<String> = method
                .params
                .iter()
                .zip(&params)
                .map(|(param, ty)| format!("{}: {ty}", param.name.name))
                .collect();
            let signature = format!("fun {qualified}({}): {return_ty}", shown_params.join(", "));
            let this_ty = Ty::new(
                TyKind::Ref(Box::new(Ty::new(TyKind::Named(class.name.clone()), false))),
                false,
            );
            fun_sigs.insert(
                qualified,
                FunSig {
                    params: std::iter::once(this_ty).chain(params).collect(),
                    return_ty,
                },
            );
            if let Some(info) = classes.get_mut(&class.name) {
                info.methods.insert(method.name.clone(), signature.clone());
            }
            members.insert(
                method.name_span,
                MemberRef::Method {
                    class: class.name.clone(),
                    name: method.name.clone(),
                    signature,
                },
            );
        }
    }
}

fn check_function(
    function: &Function,
    owner: Option<&str>,
    fun_sigs: &HashMap<String, FunSig>,
    classes: &ClassTable,
    constants: &crate::constants::ConstantTable,
    diagnostics: &mut Vec<Diagnostic>,
    span_tys: &mut SpanTypes,
    members: &mut MemberResolutions,
) -> HirFunction {
    let mut env = Env::new(fun_sigs, classes, constants);
    env.self_class = owner.map(str::to_string);
    let name = match owner {
        Some(class) => format!("{class}.{}", function.name),
        None => function.name.clone(),
    };
    let signature = env
        .fun_sigs
        .get(&name)
        .cloned()
        .expect("signature was collected for every function");
    let mut param_tys = signature.params;
    let mut params = Vec::new();
    if owner.is_some() {
        let this_ty = param_tys.remove(0);
        env.bind("this".to_string(), crate::ast::BindingKind::Val, this_ty.clone(), true);
        params.push(HirParam {
            kind: crate::ast::BindingKind::Val,
            name: "this".to_string(),
            ty: this_ty,
        });
    }
    params.extend(bind_params(function, param_tys, &mut env));
    let return_ty = signature.return_ty;
    let body = stmt::check_block(&function.body, &return_ty, &mut env, false);
    if return_ty != Ty::unit() && !stmt::block_always_returns(&body) {
        env.error(
            format!("function must return a value of type {return_ty} on all paths"),
            None,
        );
    }
    diagnostics.append(&mut env.diagnostics);
    span_tys.extend(env.span_tys);
    members.extend(env.members);
    HirFunction {
        name,
        name_span: function.name_span,
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
