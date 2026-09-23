mod env;
mod expr;
mod stmt;

use std::collections::HashMap;

use crate::ast::{Program, Type};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirFunction, HirParam, HirProgram, Ty, TyKind};

use env::{Env, FunSig};

pub fn check(program: &Program) -> (Option<HirProgram>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    let fun_sigs: HashMap<_, _> = program
        .functions
        .iter()
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

    let functions = program
        .functions
        .iter()
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
                    env.bind(param.name.name.clone(), param.kind, ty.clone());
                    HirParam {
                        kind: param.kind,
                        name: param.name.name.clone(),
                        ty,
                    }
                })
                .collect();
            let return_ty = signature.return_ty;
            let body = stmt::check_block(&function.body, &return_ty, &mut env, false);
            diagnostics.append(&mut env.diagnostics);
            HirFunction {
                name: function.name.clone(),
                params,
                return_ty,
                body,
            }
        })
        .collect();

    if diagnostics.is_empty() {
        (Some(HirProgram { functions }), diagnostics)
    } else {
        (None, diagnostics)
    }
}

pub(super) fn lower_type(ty: &Type, diagnostics: &mut Vec<Diagnostic>) -> Ty {
    match ty {
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
        _ => Ty::from_ast(ty),
    }
}

#[cfg(test)]
mod tests;
