//! Region analysis entrypoints.

use super::env::{Analyzer, Ty};
use super::region::RegionParam;
use super::report::{ArenaNode, ArenaReport, SemaError};
use super::walk::open_ordinary;
use crate::ast::{BindingKind, Function, Program, Type};

/// Analyze `program` for arena hierarchy and Copy/Move ownership.
pub fn analyze(program: &Program) -> (ArenaReport, Vec<SemaError>) {
    let crate::typeck::Typeck { node_tys, members, option_if_vals, tcx, .. } = crate::typeck::check_with_members(program);
    analyze_with_members(program, node_tys, members, option_if_vals, &tcx)
}

/// Same as [`analyze`], but reuses `node_tys` from an earlier `typeck::check` (avoids duplicate work).
pub fn analyze_with_node_tys(
    program: &Program,
    node_tys: crate::typeck::NodeTypes,
    tcx: &crate::ty::TyCtx,
) -> (ArenaReport, Vec<SemaError>) {
    analyze_with_members(program, node_tys, Default::default(), Default::default(), tcx)
}

/// Same as [`analyze_with_node_tys`], plus typeck's member resolutions for methods.
pub fn analyze_with_members(
    program: &Program,
    node_tys: crate::typeck::NodeTypes,
    members: crate::typeck::MemberResolutions,
    option_if_vals: std::collections::HashSet<crate::span::AstId>,
    tcx: &crate::ty::TyCtx,
) -> (ArenaReport, Vec<SemaError>) {
    let mut az = Analyzer::new();
    az.option_if_vals = option_if_vals;
    az.node_tys = node_tys
        .into_iter()
        .map(|(node, ty)| (node, super::walk::ty_from_id(tcx, ty)))
        .collect();
    az.members = members;
    az.classes = program
        .classes
        .iter()
        .map(|class| (class.name.clone(), class.clone()))
        .collect();
    // MVP: keyed by bare function name (no local shadowing of callees yet).
    for f in &program.functions {
        az.fun_sigs.insert(
            f.name.clone(),
            f.params.iter().map(|p| p.kind).collect(),
        );
        az.fun_param_tys.insert(
            f.name.clone(),
            f.params.iter().map(|p| p.ty.clone()).collect(),
        );
    }
    for class in &program.classes {
        for method in &class.methods {
            let qualified = format!("{}.{}", class.name, method.name);
            az.fun_sigs.insert(
                qualified.clone(),
                std::iter::once(BindingKind::Val)
                    .chain(method.params.iter().map(|p| p.kind))
                    .collect(),
            );
            az.fun_param_tys.insert(
                qualified,
                std::iter::once(this_type(&class.name))
                    .chain(method.params.iter().map(|p| p.ty.clone()))
                    .collect(),
            );
        }
    }
    // A call through a trait (`Show.show`) takes its modes from the trait's declaration. Its `this`
    // is the implementing type, which arena analysis never inspects, so any borrow type will do.
    for t in &program.traits {
        for method in &t.methods {
            let qualified = format!("{}.{}", t.name.name, method.name.name);
            az.fun_sigs.insert(
                qualified.clone(),
                std::iter::once(BindingKind::Val).chain(method.params.iter().map(|p| p.kind)).collect(),
            );
            az.fun_param_tys.insert(
                qualified,
                std::iter::once(this_type(&t.name.name)).chain(method.params.iter().map(|p| p.ty.clone())).collect(),
            );
        }
    }
    // Same order as typeck's HIR functions: top-level, then methods class by class, then impl methods.
    let mut roots: Vec<ArenaNode> = program
        .functions
        .iter()
        .map(|f| analyze_function(&mut az, f, None))
        .collect();
    for class in &program.classes {
        for method in &class.methods {
            roots.push(analyze_function(&mut az, method, Some((&class.name, this_type(&class.name)))));
        }
    }
    for (index, i) in program.impls.iter().enumerate() {
        let this = Type::Ref { inner: Box::new(i.self_ty.clone()), nullable: false };
        let label = format!("{}@{index}", i.trait_name.name);
        for method in &i.methods {
            roots.push(analyze_function(&mut az, method, Some((&label, this.clone()))));
        }
    }
    (ArenaReport { roots }, az.errors)
}

fn this_type(class: &str) -> Type {
    Type::Ref {
        inner: Box::new(Type::Named {
            name: class.to_string(),
                args: Vec::new(),
            nullable: false,
        }),
        nullable: false,
    }
}

/// `owner` is the label prefix (`Class`, or `Trait@index` for an impl) and the type of `this`.
fn analyze_function(az: &mut Analyzer, func: &Function, owner: Option<(&str, Type)>) -> ArenaNode {
    let label_owner = owner.as_ref().map(|(label, _)| *label);
    let this = owner.map(|(_, this_ty)| RegionParam {
        name: "this".to_string(),
        ty: Ty::Known(this_ty),
        kind: BindingKind::Val,
        span: None,
        borrow_from: super::region::REF_PARAM_BORROW_FROM,
    });
    let params: Vec<RegionParam> = this
        .into_iter()
        .chain(func.params.iter().map(|p| RegionParam {
            name: p.name.name.clone(),
            ty: Ty::Known(p.ty.clone()),
            kind: p.kind,
            span: Some(p.name.span),
            borrow_from: super::region::REF_PARAM_BORROW_FROM,
        }))
        .collect();
    let label = match label_owner {
        Some(class) => format!("fun {class}.{}", func.name),
        None => format!("fun {}", func.name),
    };
    open_ordinary(az, &label, &func.body, &params, None)
}
