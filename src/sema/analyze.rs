//! Region analysis entrypoints.

use super::env::{Analyzer, Ty};
use super::region::RegionParam;
use super::report::{ArenaNode, ArenaReport, SemaError};
use super::walk::open_ordinary;
use crate::ast::{BindingKind, Function, Program, Type};

/// Analyze `program` for arena hierarchy and Copy/Move ownership.
pub fn analyze(program: &Program) -> (ArenaReport, Vec<SemaError>) {
    let (_, node_tys, members, _) = crate::typeck::check_with_members(program);
    analyze_with_members(program, node_tys, members)
}

/// Same as [`analyze`], but reuses `node_tys` from an earlier `typeck::check` (avoids duplicate work).
pub fn analyze_with_node_tys(
    program: &Program,
    node_tys: crate::typeck::NodeTypes,
) -> (ArenaReport, Vec<SemaError>) {
    analyze_with_members(program, node_tys, Default::default())
}

/// Same as [`analyze_with_node_tys`], plus typeck's member resolutions for methods.
pub fn analyze_with_members(
    program: &Program,
    node_tys: crate::typeck::NodeTypes,
    members: crate::typeck::MemberResolutions,
) -> (ArenaReport, Vec<SemaError>) {
    let mut az = Analyzer::new();
    az.node_tys = node_tys;
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
    // Same order as typeck's HIR functions: top-level, then methods class by class.
    let mut roots: Vec<ArenaNode> = program
        .functions
        .iter()
        .map(|f| analyze_function(&mut az, f, None))
        .collect();
    for class in &program.classes {
        for method in &class.methods {
            roots.push(analyze_function(&mut az, method, Some(&class.name)));
        }
    }
    (ArenaReport { roots }, az.errors)
}

fn this_type(class: &str) -> Type {
    Type::Ref {
        inner: Box::new(Type::Named {
            name: class.to_string(),
            nullable: false,
        }),
        nullable: false,
    }
}

fn analyze_function(az: &mut Analyzer, func: &Function, owner: Option<&str>) -> ArenaNode {
    let this = owner.map(|class| RegionParam {
        name: "this".to_string(),
        ty: Ty::Known(this_type(class)),
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
    let label = match owner {
        Some(class) => format!("fun {class}.{}", func.name),
        None => format!("fun {}", func.name),
    };
    open_ordinary(az, &label, &func.body, &params, None)
}
