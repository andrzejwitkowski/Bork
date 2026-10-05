mod env;
mod expr;
mod lower;
mod stmt;
mod structs;

use std::collections::HashMap;

use crate::ast::{Function, Program};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{HirFunction, HirParam, HirProgram, HirStructDef, Ty};
use crate::span::Span;

use env::Env;
pub(crate) use env::FunSig;
pub(super) use lower::lower_type; // re-export for stmt / expr
use structs::build_struct_env;

pub fn check(program: &Program) -> (HirProgram, Vec<Ty>, Vec<Diagnostic>) {
    let mut diagnostics = Vec::new();
    reject_builtin_redefines(program, &mut diagnostics);

    let structs = build_struct_env(program, &mut diagnostics);
    let fun_sigs = collect_fun_sigs(program, &structs, &mut diagnostics);

    let mut decl_tys = Vec::new();
    let functions = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| {
            check_function(function, &fun_sigs, &structs, &mut diagnostics, &mut decl_tys)
        })
        .collect();

    let structs = program
        .structs
        .iter()
        .filter_map(|decl| structs.get(&decl.name).cloned())
        .collect();

    (HirProgram { structs, functions }, decl_tys, diagnostics)
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

fn collect_fun_sigs(
    program: &Program,
    structs: &HashMap<String, HirStructDef>,
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
                        .map(|param| lower_type(&param.ty, structs, diagnostics))
                        .collect(),
                    return_ty: lower_type(&function.return_type, structs, diagnostics),
                },
            )
        })
        .collect();
    fun_sigs.extend(crate::builtins::signatures());
    fun_sigs
}

fn check_function(
    function: &Function,
    fun_sigs: &HashMap<String, FunSig>,
    structs: &HashMap<String, HirStructDef>,
    diagnostics: &mut Vec<Diagnostic>,
    decl_tys: &mut Vec<Ty>,
) -> HirFunction {
    let mut env = Env::new(fun_sigs, structs);
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
    decl_tys.append(&mut env.decl_tys);
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

fn builtin_redefine_span(function: &Function) -> Option<Span> {
    function.params.first().map(|param| param.name.span)
}

#[cfg(test)]
mod tests;
