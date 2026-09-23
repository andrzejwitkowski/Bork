mod env;
mod expr;
mod stmt;

use std::collections::HashMap;

use crate::ast::Program;
use crate::diag::Diagnostic;
use crate::hir::{HirFunction, HirParam, HirProgram, RegionId, Ty};

use env::{Env, FunSig};

pub fn check(program: &Program) -> (Option<HirProgram>, Vec<Diagnostic>) {
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
                        .map(|param| Ty::from_ast(&param.ty))
                        .collect(),
                    return_ty: Ty::from_ast(&function.return_type),
                },
            )
        })
        .collect();

    let mut diagnostics = Vec::new();
    let functions = program
        .functions
        .iter()
        .enumerate()
        .map(|(idx, function)| {
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
                region: idx as RegionId,
            }
        })
        .collect();

    if diagnostics.is_empty() {
        (Some(HirProgram { functions }), diagnostics)
    } else {
        (None, diagnostics)
    }
}

#[cfg(test)]
mod tests;
