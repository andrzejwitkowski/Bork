mod env;
mod expr;
mod lower;
mod stmt;

use std::collections::{HashMap, HashSet};

use crate::ast::{Function, Program};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::defs::{DefId, DefTable};
use crate::hir::{HirClass, HirClassField, HirFunction, HirParam, HirProgram};
use crate::span::{AstId, Span};
use crate::ty::{FnSig, FnSigs, TyCtx, TyId};

use env::{ClassFieldInfo, ClassInfo, ClassTable, Env};
use lower::lower_type;

/// Types used by sema, keyed by the AST node of a declaration name or field assignment.
pub type NodeTypes = HashMap<AstId, TyId>;

/// What a bare member name or a method-call callee resolved to.
#[derive(Debug, Clone, PartialEq)]
pub enum MemberRef {
    /// Bare `name` inside a method, read or assigned through `this`.
    Field { class: String, name: String, index: usize, ty: TyId },
    /// `recv.m(..)` / bare `m(..)` call, or a method declaration name.
    Method { class: String, name: String, signature: String },
    /// Global constant, at its declaration name and at each use.
    Constant { name: String, ty: TyId, value: String },
}

/// Member resolutions keyed by the AST node of a bare name, a `recv.m` callee, or a method name.
pub type MemberResolutions = HashMap<AstId, MemberRef>;

/// Everything typeck produces. `tcx` owns the ids in the HIR's `TyId` fields and outlives typeck.
pub struct Typeck {
    pub hir: HirProgram,
    pub node_tys: NodeTypes,
    pub members: MemberResolutions,
    pub diagnostics: Vec<Diagnostic>,
    pub tcx: TyCtx,
}

pub fn check(program: &Program) -> (HirProgram, NodeTypes, Vec<Diagnostic>) {
    let Typeck { hir, node_tys, diagnostics, .. } = check_with_members(program);
    (hir, node_tys, diagnostics)
}

pub fn check_with_members(program: &Program) -> Typeck {
    let mut diagnostics = Vec::new();
    let defs = DefTable::build(program);
    let mut tcx = TyCtx::new(&defs);
    let constants = crate::constants::collect_into(program, &defs, &mut tcx, &mut diagnostics);
    for index in 0..program.functions.len() {
        diagnostics.extend(defs.diagnostics(defs.fn_id(index), |_| true));
    }

    reject_generics(program, &mut diagnostics);
    let mut classes = build_classes(program, &defs, &mut tcx, &mut diagnostics);
    validate_owned_layouts(&classes, &tcx, &mut diagnostics);
    let mut fun_sigs = collect_fun_sigs(program, &defs, &mut tcx, &mut diagnostics);
    let mut members = MemberResolutions::new();
    for (name, constant) in &constants {
        members.insert(constant.id.numbered(), constant.member(name));
    }
    collect_method_sigs(program, &defs, &mut tcx, &mut classes, &mut fun_sigs, &mut members, &mut diagnostics);
    let mut node_tys = NodeTypes::new();
    let mut functions: Vec<HirFunction> = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| {
            check_function(
                function, None, &defs, &mut tcx, &fun_sigs, &classes, &constants, &mut diagnostics, &mut node_tys,
                &mut members,
            )
        })
        .collect();
    // Same order as sema roots: top-level functions, then methods class by class.
    for class in &program.classes {
        let owner = defs.lookup_class(&class.name).expect("every class is registered");
        for method in &class.methods {
            functions.push(check_function(
                method,
                Some(owner),
                &defs,
                &mut tcx,
                &fun_sigs,
                &classes,
                &constants,
                &mut diagnostics,
                &mut node_tys,
                &mut members,
            ));
        }
    }

    let hir_classes = program
        .classes
        .iter()
        .filter_map(|class| classes.get(&defs.lookup_class(&class.name)?))
        .map(|class| HirClass {
            name: class.name.clone(),
            fields: class
                .fields
                .iter()
                .map(|field| HirClassField {
                    name: field.name.clone(),
                    ty: field.ty,
                })
                .collect(),
        })
        .collect();
    Typeck {
        hir: HirProgram {
            classes: hir_classes,
            functions,
        },
        node_tys,
        members,
        diagnostics,
        tcx,
    }
}

fn build_classes(
    program: &Program,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> ClassTable {
    let mut classes = ClassTable::new();
    for (index, class) in program.classes.iter().enumerate() {
        use crate::defs::Clash;
        diagnostics.extend(defs.diagnostics(defs.class_id(index), |c| {
            matches!(c, Clash::ClassBuiltinType | Clash::ClassDuplicate)
        }));
        // A duplicate name keeps the first declaration.
        let id = defs.lookup_class(&class.name).expect("every class is registered");
        classes.entry(id).or_insert_with(|| ClassInfo {
            id,
            name: class.name.clone(),
            fields: Vec::new(),
            methods: HashMap::new(),
        });
    }
    let mut filled = HashSet::new();
    for class in &program.classes {
        let id = defs.lookup_class(&class.name).expect("every class is registered");
        if !filled.insert(id) {
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
                    ty: lower_type(&field.ty, defs, tcx, diagnostics, Some(field.name.span)),
                    span: field.name.span,
                }
            })
            .collect();
        if let Some(info) = classes.get_mut(&id) {
            info.fields = fields;
        }
    }
    classes
}

fn collect_fun_sigs(
    program: &Program,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) -> FnSigs {
    let mut fun_sigs = FnSigs::new();
    for function in program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
    {
        let params = function
            .params
            .iter()
            .map(|param| lower_type(&param.ty, defs, tcx, diagnostics, Some(param.name.span)))
            .collect();
        let ret = lower_type(&function.return_type, defs, tcx, diagnostics, Some(function.name_span));
        // Redeclared names share one id; the last declaration wins.
        let id = defs.lookup_fn(&function.name).expect("every function is registered");
        fun_sigs.insert(id, FnSig::new(params, ret));
    }
    for (name, sig) in crate::builtins::signatures(tcx) {
        let id = defs.lookup_intrinsic(name).expect("every builtin is registered");
        fun_sigs.insert(id, sig);
    }
    for index in 0..program.classes.len() {
        diagnostics.extend(defs.diagnostics(defs.class_id(index), |c| c == crate::defs::Clash::ClassFn));
    }
    fun_sigs
}

/// Registers each method with a leading `this: &C` parameter.
fn collect_method_sigs(
    program: &Program,
    defs: &DefTable,
    tcx: &mut TyCtx,
    classes: &mut ClassTable,
    fun_sigs: &mut FnSigs,
    members: &mut MemberResolutions,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (class_index, class) in program.classes.iter().enumerate() {
        for (method_index, method) in class.methods.iter().enumerate() {
            let method_id = defs.method_id(class_index, method_index);
            diagnostics.extend(defs.diagnostics(method_id, |_| true));
            let owner = defs.get(method_id).owner.expect("a method has an owner class");
            // A duplicate method resolves to the first declaration, which already has a signature.
            let key = defs.lookup_method(owner, &method.name).expect("every method is registered");
            if fun_sigs.contains_key(&key) {
                continue;
            }
            let params: Vec<TyId> = method
                .params
                .iter()
                .map(|param| lower_type(&param.ty, defs, tcx, diagnostics, Some(param.name.span)))
                .collect();
            let return_ty = lower_type(&method.return_type, defs, tcx, diagnostics, Some(method.name_span));
            let shown_params: Vec<String> = method
                .params
                .iter()
                .zip(&params)
                .map(|(param, ty)| format!("{}: {}", param.name.name, tcx.display(*ty)))
                .collect();
            let signature = format!(
                "fun {}.{}({}): {}",
                class.name,
                method.name,
                shown_params.join(", "),
                tcx.display(return_ty)
            );
            let class_ty = tcx.class(owner);
            let this_ty = tcx.ref_to(class_ty);
            fun_sigs.insert(key, FnSig::new(std::iter::once(this_ty).chain(params).collect(), return_ty));
            if let Some(info) = classes.get_mut(&owner) {
                info.methods.insert(key, signature.clone());
            }
            members.insert(
                method.id.numbered(),
                MemberRef::Method {
                    class: class.name.clone(),
                    name: method.name.clone(),
                    signature,
                },
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn check_function(
    function: &Function,
    owner: Option<DefId>,
    defs: &DefTable,
    tcx: &mut TyCtx,
    fun_sigs: &FnSigs,
    classes: &ClassTable,
    constants: &crate::constants::ConstantTable,
    diagnostics: &mut Vec<Diagnostic>,
    node_tys: &mut NodeTypes,
    members: &mut MemberResolutions,
) -> HirFunction {
    let def = match owner {
        Some(class) => defs.lookup_method(class, &function.name),
        None => defs.lookup_fn(&function.name),
    }
    .expect("every function and method is registered");
    let signature = fun_sigs
        .get(&def)
        .cloned()
        .expect("signature was collected for every function");
    let name = match owner {
        Some(class) => format!("{}.{}", defs.get(class).name, function.name),
        None => function.name.clone(),
    };
    let mut env = Env::new(defs, tcx, fun_sigs, classes, constants);
    env.self_class = owner;
    let mut param_tys = signature.params;
    let mut params = Vec::new();
    if owner.is_some() {
        let this_ty = param_tys.remove(0);
        env.bind("this".to_string(), crate::ast::BindingKind::Val, this_ty, true);
        params.push(HirParam {
            kind: crate::ast::BindingKind::Val,
            name: "this".to_string(),
            ty: this_ty,
        });
    }
    params.extend(bind_params(function, param_tys, &mut env));
    let return_ty = signature.ret;
    let body = stmt::check_block(&function.body, return_ty, &mut env, false);
    let unit = env.tcx.unit();
    if return_ty != unit && !stmt::block_always_returns(&body) {
        env.error(
            format!("function must return a value of type {} on all paths", env.show(return_ty)),
            None,
        );
    }
    // Defaulting happens once, here; after it no literal variable remains.
    env.infcx.default_literals(env.tcx);
    let mut body = body;
    let (infcx, tcx) = (&env.infcx, &mut *env.tcx);
    let mut settle = |ty: TyId| {
        let ty = infcx.resolve(tcx, ty).unwrap_or(ty);
        debug_assert!(!has_infer(tcx, ty), "unresolved inference variable reached HIR");
        ty
    };
    crate::hir::resolve_types(&mut body, &mut settle);
    env.node_tys.values_mut().for_each(|ty| *ty = settle(*ty));
    diagnostics.append(&mut env.diagnostics);
    node_tys.extend(std::mem::take(&mut env.node_tys));
    members.extend(std::mem::take(&mut env.members));
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
    param_tys: Vec<TyId>,
    env: &mut Env<'_>,
) -> Vec<HirParam> {
    function
        .params
        .iter()
        .zip(param_tys)
        .map(|(param, ty)| {
            let is_ref = env.tcx.is_ref(ty);
            if is_ref && param.kind == crate::ast::BindingKind::Var {
                env.error(
                    "reference parameter must be `val`",
                    Some(param.name.span),
                );
            }
            env.bind(param.name.name.clone(), param.kind, ty, is_ref);
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

fn validate_owned_layouts(classes: &ClassTable, tcx: &TyCtx, diagnostics: &mut Vec<Diagnostic>) {
    /// The class a field holds by value: itself, or as the element of an array.
    fn held_class(ty: TyId, tcx: &TyCtx) -> Option<DefId> {
        match tcx.data(tcx.base(ty)) {
            crate::ty::TyData::Class(def) => Some(*def),
            crate::ty::TyData::Array { elem, .. } => tcx.class_def(tcx.base(*elem)),
            _ => None,
        }
    }

    fn visits(
        origin: DefId,
        current: DefId,
        classes: &ClassTable,
        tcx: &TyCtx,
        seen: &mut HashSet<DefId>,
    ) -> bool {
        if !seen.insert(current) {
            return false;
        }
        let Some(class) = classes.get(&current) else {
            return false;
        };
        class.fields.iter().any(|field| {
            held_class(field.ty, tcx).is_some_and(|target| {
                target == origin || visits(origin, target, classes, tcx, &mut seen.clone())
            })
        })
    }

    for class in classes.values() {
        if visits(class.id, class.id, classes, tcx, &mut HashSet::new()) {
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

#[cfg(test)]
mod tests;

/// Whether `ty` still contains an inference variable.
fn has_infer(tcx: &TyCtx, ty: TyId) -> bool {
    struct Find(bool);
    impl crate::ty::TypeVisitor for Find {
        fn visit(&mut self, ctx: &TyCtx, id: TyId) {
            self.0 |= matches!(ctx.data(id), crate::ty::TyData::Infer(_));
            crate::ty::walk(self, ctx, id);
        }
    }
    let mut find = Find(false);
    crate::ty::TypeVisitor::visit(&mut find, tcx, ty);
    find.0
}

/// Generic declarations parse (#42) but are not checked yet.
fn reject_generics(program: &Program, diagnostics: &mut Vec<Diagnostic>) {
    let methods = program.classes.iter().flat_map(|class| class.methods.iter());
    let generics = program
        .functions
        .iter()
        .chain(methods)
        .flat_map(|function| &function.generics)
        .chain(program.classes.iter().flat_map(|class| &class.generics));
    for param in generics {
        diagnostics.push(Diagnostic {
            phase: Phase::Type,
            severity: Severity::Error,
            message: format!(
                "generic parameter `{}` is not supported yet; remove the `<..>` list",
                param.name.name
            ),
            span: Some(param.name.span),
        });
    }
}
