mod env;
mod expr;
mod stmt;

use std::collections::HashMap;

use crate::ast::{Function, Program, Type};
use crate::span::Span;
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirFunction, HirParam, HirProgram, Ty, TyKind};

use env::Env;
pub(crate) use env::FunSig;

pub fn check(program: &Program) -> (HirProgram, Vec<Ty>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
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
                        .map(|param| lower_type(&param.ty, &mut diagnostics))
                        .collect(),
                    return_ty: lower_type(&function.return_type, &mut diagnostics),
                },
            )
        })
        .collect();
    fun_sigs.extend(crate::builtins::signatures());

    let mut decl_tys = Vec::new();
    let functions = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| {
            let mut env = Env::new(&fun_sigs);
            let signature = env
                .fun_sigs
                .get(&function.name)
                .cloned()
                .expect("signature was collected for every function");
            let params = function
                .params
                .iter()
                .zip(signature.params)
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
                .collect();
            let return_ty = signature.return_ty;
            let body = stmt::check_block(&function.body, &return_ty, &mut env, false);
            if return_ty != Ty::unit() && !stmt::block_always_returns(&body) {
                env.error(
                    format!("function must return a value of type {return_ty} on all paths"),
                    None,
                );
            }
            diagnostics.append(&mut env.diagnostics);
            decl_tys.append(&mut env.decl_tys);
            HirFunction {
                name: function.name.clone(),
                params,
                return_ty,
                body,
            }
        })
        .collect();

    (HirProgram { functions }, decl_tys, diagnostics)
}

pub(super) fn lower_type(ty: &Type, diagnostics: &mut Vec<Diagnostic>) -> Ty {
    match ty {
        Type::Array { nullable, .. } if *nullable => {
            diagnostics.push(Diagnostic {
                phase: Phase::Type,
                severity: Severity::Error,
                message: "nullable array types `[T]?` are not supported".into(),
                span: None,
            });
            Ty::unknown()
        }
        Type::Array { elem, len, .. } => Ty::new(
            TyKind::Array {
                elem: Box::new(lower_type(elem, diagnostics)),
                len: *len,
            },
            false,
        ),
        Type::Named { name, .. } if name != "String" => {
            diagnostics.push(Diagnostic {
                phase: Phase::Type,
                severity: Severity::Error,
                message: format!("unknown named type `{name}`"),
                span: None,
            });
            Ty::unknown()
        }
        Type::Func {
            params,
            ret,
            nullable,
        } => Ty::new(
            TyKind::Func {
                params: params
                    .iter()
                    .map(|param| lower_type(param, diagnostics))
                    .collect(),
                ret: Box::new(lower_type(ret, diagnostics)),
            },
            *nullable,
        ),
        Type::Ref { inner, nullable } => {
            if *nullable {
                diagnostics.push(Diagnostic {
                    phase: Phase::Type,
                    severity: Severity::Error,
                    message: "nullable reference types `&T?` are not supported".into(),
                    span: None,
                });
            }
            let inner_ty = lower_type(inner, diagnostics);
            if !inner_ty.is_unknown() && inner_ty.is_copy() {
                diagnostics.push(Diagnostic {
                    phase: Phase::Type,
                    severity: Severity::Error,
                    message: format!("cannot borrow Copy type `{inner_ty}`"),
                    span: None,
                });
            }
            Ty::new(TyKind::Ref(Box::new(inner_ty)), false)
        }
        _ => Ty::from_ast(ty),
    }
}

fn builtin_redefine_span(function: &Function) -> Option<Span> {
    function.params.first().map(|param| param.name.span)
}

#[cfg(test)]
mod tests;
