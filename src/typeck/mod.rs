mod env;
mod expr;
mod generics;
mod lower;
mod stmt;

use std::collections::{HashMap, HashSet};

use crate::ast::{Function, Program};
use crate::diag::{Diagnostic, Phase, Severity};
use crate::defs::{DefId, DefTable};
use crate::hir::{HirClass, HirClassField, HirFunction, HirParam, HirProgram};
use crate::span::{AstId, Span};
use crate::ty::{FnSig, FnSigs, TyCtx, TyId};

use env::{ClassFieldInfo, ClassInfo, ClassTable, EnumInfo, EnumTable, Env, PendingParam, VariantInfo};
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

    generics::validate(program, &defs, &mut diagnostics);
    let mut classes = build_classes(program, &defs, &mut tcx, &mut diagnostics);
    let enums = build_enums(program, &defs, &mut tcx, &mut diagnostics);
    validate_owned_layouts(&classes, &enums, &mut tcx, &mut diagnostics);
    let mut fun_sigs = collect_fun_sigs(program, &defs, &mut tcx, &mut diagnostics);
    let mut members = MemberResolutions::new();
    for (name, constant) in &constants {
        members.insert(constant.id.numbered(), constant.member(name));
    }
    collect_method_sigs(program, &defs, &mut tcx, &mut classes, &mut fun_sigs, &mut members, &mut diagnostics);
    let mut node_tys = NodeTypes::new();
    let mut checker = FunctionChecker {
        defs: &defs,
        fun_sigs: &fun_sigs,
        classes: &classes,
        enums: &enums,
        constants: &constants,
        tcx: &mut tcx,
        diagnostics: &mut diagnostics,
        node_tys: &mut node_tys,
        members: &mut members,
    };
    let mut functions: Vec<HirFunction> = program
        .functions
        .iter()
        .filter(|function| !crate::builtins::is_intrinsic(&function.name))
        .map(|function| check_function(&mut checker, function, None))
        .collect();
    // Same order as sema roots: top-level functions, then methods class by class.
    for class in &program.classes {
        let owner = defs.lookup_class(&class.name).expect("every class is registered");
        for method in &class.methods {
            functions.push(check_function(&mut checker, method, Some(owner)));
        }
    }

    let hir_classes = program
        .classes
        .iter()
        .filter_map(|class| classes.get(&defs.lookup_class(&class.name)?))
        .map(|class| HirClass {
            name: class.name.clone(),
            generics: class.generics.clone(),
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
    for (index, e) in program.enums.iter().enumerate() {
        diagnostics.extend(defs.diagnostics(defs.enum_id(index), |_| true));
        for variant in 0..e.variants.len() {
            diagnostics.extend(defs.diagnostics(defs.variant_id(index, variant), |_| true));
        }
    }
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
            generics: generics::names(&class.generics, &[]),
        });
    }
    let mut filled = HashSet::new();
    for class in &program.classes {
        let id = defs.lookup_class(&class.name).expect("every class is registered");
        if !filled.insert(id) {
            continue;
        }
        let mut field_names = HashSet::new();
        let scope = generics::scope(&class.generics, &[]);
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
                    ty: lower_type(&field.ty, defs, &scope, tcx, diagnostics, Some(field.name.span)),
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

/// Lowers every variant payload. A rejected or duplicate enum keeps the first declaration only.
fn build_enums(program: &Program, defs: &DefTable, tcx: &mut TyCtx, diagnostics: &mut Vec<Diagnostic>) -> EnumTable {
    let mut enums = EnumTable::new();
    for (index, e) in program.enums.iter().enumerate() {
        let id = defs.enum_id(index);
        if defs.lookup_enum(&e.name.name) != Some(id) {
            continue;
        }
        let scope = generics::scope(&e.generics, &[]);
        let variants = e
            .variants
            .iter()
            .map(|variant| {
                let mut field_names = HashSet::new();
                let fields = variant
                    .fields
                    .iter()
                    .map(|field| {
                        if !field_names.insert(field.name.name.clone()) {
                            diagnostics.push(type_error(
                                format!("duplicate field `{}` in variant `{}.{}`", field.name.name, e.name.name, variant.name.name),
                                Some(field.name.span),
                            ));
                        }
                        if field.ty.is_reference() {
                            diagnostics.push(type_error(
                                "borrowed references cannot be stored in enum payloads",
                                Some(field.name.span),
                            ));
                        }
                        ClassFieldInfo {
                            name: field.name.name.clone(),
                            ty: lower_type(&field.ty, defs, &scope, tcx, diagnostics, Some(field.name.span)),
                            span: field.name.span,
                        }
                    })
                    .collect();
                VariantInfo { name: variant.name.name.clone(), fields }
            })
            .collect();
        enums.insert(id, EnumInfo { id, name: e.name.name.clone(), variants, generics: generics::names(&e.generics, &[]) });
    }
    enums
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
        let scope = generics::scope(&[], &function.generics);
        let params = function
            .params
            .iter()
            .map(|param| lower_type(&param.ty, defs, &scope, tcx, diagnostics, Some(param.name.span)))
            .collect();
        let ret = lower_type(&function.return_type, defs, &scope, tcx, diagnostics, Some(function.name_span));
        // Redeclared names share one id; the last declaration wins.
        let id = defs.lookup_fn(&function.name).expect("every function is registered");
        let mut sig = FnSig::new(params, ret);
        sig.generics = scope.iter().map(|(_, idx)| *idx).collect();
        sig.generic_names = scope.iter().map(|(name, _)| name.clone()).collect();
        fun_sigs.insert(id, sig);
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
            let scope = generics::scope(&class.generics, &method.generics);
            let params: Vec<TyId> = method
                .params
                .iter()
                .map(|param| lower_type(&param.ty, defs, &scope, tcx, diagnostics, Some(param.name.span)))
                .collect();
            let return_ty = lower_type(&method.return_type, defs, &scope, tcx, diagnostics, Some(method.name_span));
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
            let identity = tcx.identity_args(class.generics.len());
            let class_ty = tcx.class(owner, identity);
            let this_ty = tcx.ref_to(class_ty);
            let mut sig = FnSig::new(std::iter::once(this_ty).chain(params).collect(), return_ty);
            sig.generics = scope.iter().map(|(_, idx)| *idx).collect();
            sig.generic_names = scope.iter().map(|(name, _)| name.clone()).collect();
            fun_sigs.insert(key, sig);
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

/// What checking any function body reads (the program's tables) and writes to (the type interner,
/// the diagnostics and the per-node results).
struct FunctionChecker<'a> {
    defs: &'a DefTable,
    fun_sigs: &'a FnSigs,
    classes: &'a ClassTable,
    enums: &'a EnumTable,
    constants: &'a crate::constants::ConstantTable,
    tcx: &'a mut TyCtx,
    diagnostics: &'a mut Vec<Diagnostic>,
    node_tys: &'a mut NodeTypes,
    members: &'a mut MemberResolutions,
}

fn check_function(checker: &mut FunctionChecker<'_>, function: &Function, owner: Option<DefId>) -> HirFunction {
    let FunctionChecker { defs, fun_sigs, classes, enums, constants, tcx, diagnostics, node_tys, members } = checker;
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
    let generics: Vec<String> = owner
        .and_then(|class| classes.get(&class))
        .map_or(&[][..], |class| &class.generics[..])
        .iter()
        .cloned()
        .chain(function.generics.iter().map(|param| param.name.name.clone()))
        .collect();
    tcx.set_param_names(generics.clone());
    let mut env = Env::new(defs, tcx, fun_sigs, classes, enums, constants);
    env.self_class = owner;
    // The owner class's parameters come first, as in the method signature.
    env.tparams = generics.iter().enumerate().map(|(i, name)| (name.clone(), crate::ty::ParamIdx(i as u32))).collect();
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
    report_uninferred(&mut env);
    let mut body = body;
    let (infcx, tcx, errors) = (&env.infcx, &mut *env.tcx, &mut env.diagnostics);
    let mut settle = |ty: TyId| match infcx.resolve(tcx, ty) {
        Ok(ty) => {
            debug_assert!(!has_infer(tcx, ty), "unresolved inference variable reached HIR");
            ty
        }
        Err(_) => {
            errors.push(type_error("a type argument would make a nullable of a nullable", None));
            tcx.error()
        }
    };
    crate::hir::resolve_types(&mut body, &mut settle);
    env.node_tys.values_mut().for_each(|ty| *ty = settle(*ty));
    env.tcx.set_param_names(Vec::new());
    diagnostics.append(&mut env.diagnostics);
    node_tys.extend(std::mem::take(&mut env.node_tys));
    members.extend(std::mem::take(&mut env.members));
    HirFunction {
        name,
        generics,
        name_span: function.name_span,
        params,
        return_ty,
        body,
    }
}

/// A type parameter instantiated at a call and fixed by nothing is an error, and is then poisoned
/// so its dependents report nothing further.
fn report_uninferred(env: &mut Env<'_>) {
    for pending in std::mem::take(&mut env.pending) {
        let ty = env.infcx.resolve_shallow(env.tcx, pending.var);
        if !matches!(env.tcx.data(ty), crate::ty::TyData::Infer(_)) {
            continue;
        }
        let PendingParam { span, callee, param, .. } = pending;
        // Only functions take explicit type arguments; constructors and methods are fixed by annotation.
        let hint = if env.fun_sig(&callee).is_some() {
            format!("specify it explicitly, e.g. `{callee}::<..>(..)`")
        } else {
            "annotate the binding with its type to fix it".to_string()
        };
        env.error(format!("cannot infer type parameter `{param}` of `{callee}`; {hint}"), Some(span));
        let unknown = env.tcx.error();
        let _ = env.infcx.unify(env.tcx, ty, unknown);
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
            check_ref_param_kind(param, is_ref, &mut env.diagnostics);
            env.bind(param.name.name.clone(), param.kind, ty, is_ref);
            HirParam {
                kind: param.kind,
                name: param.name.name.clone(),
                ty,
            }
        })
        .collect()
}

/// A reference parameter must be `val`.
fn check_ref_param_kind(param: &crate::ast::Param, is_ref: bool, diagnostics: &mut Vec<Diagnostic>) {
    if is_ref && param.kind == crate::ast::BindingKind::Var {
        diagnostics.push(type_error("reference parameter must be `val`", Some(param.name.span)));
    }
}

fn type_error(message: impl Into<String>, span: Option<Span>) -> Diagnostic {
    Diagnostic {
        phase: Phase::Type,
        severity: Severity::Error,
        message: message.into(),
        span,
    }
}

fn validate_owned_layouts(classes: &ClassTable, enums: &EnumTable, tcx: &mut TyCtx, diagnostics: &mut Vec<Diagnostic>) {
    /// Expanding generics (`class N<T> { x: N<N<T>> }`) never repeat a type, so the walk is capped.
    const MAX_DEPTH: usize = 32;

    /// The classes and enums a field type holds by value: itself, or as the element of an array.
    fn held(ty: TyId, tcx: &TyCtx) -> Option<TyId> {
        let ty = tcx.base(ty);
        match tcx.data(ty) {
            crate::ty::TyData::Class(..) | crate::ty::TyData::Enum(..) => Some(ty),
            crate::ty::TyData::Array { elem, .. } => Some(tcx.base(*elem)).filter(|e| tcx.class_def(*e).is_some() || tcx.enum_args(*e).is_some()),
            _ => None,
        }
    }

    /// The by-value path from `current` back to `origin`, if there is one. `done` holds the types
    /// already explored without reaching `origin`.
    fn find_cycle(
        origin: TyId,
        current: TyId,
        tables: (&ClassTable, &EnumTable),
        tcx: &mut TyCtx,
        done: &mut HashSet<TyId>,
        path: &mut Vec<TyId>,
    ) -> bool {
        if path.len() > MAX_DEPTH {
            return true;
        }
        if !done.insert(current) {
            return false;
        }
        let (args, field_tys): (Vec<TyId>, Vec<TyId>) = match tcx.data(current).clone() {
            crate::ty::TyData::Class(def, args) => match tables.0.get(&def) {
                Some(class) => (args, class.fields.iter().map(|field| field.ty).collect()),
                None => return false,
            },
            crate::ty::TyData::Enum(def, args) => match tables.1.get(&def) {
                Some(e) => (args, e.variants.iter().flat_map(|v| &v.fields).map(|field| field.ty).collect()),
                None => return false,
            },
            _ => return false,
        };
        for field_ty in field_tys {
            let Ok(field_ty) = crate::ty::subst(tcx, field_ty, &args) else {
                continue;
            };
            let Some(target) = held(field_ty, tcx) else {
                continue;
            };
            path.push(target);
            if target == origin || find_cycle(origin, target, tables, tcx, done, path) {
                return true;
            }
            path.pop();
        }
        false
    }

    /// A class or enum whose layout is checked.
    struct Owner<'a> {
        kind: &'static str,
        generics: &'a [String],
        is_enum: bool,
        id: DefId,
        span: Option<Span>,
    }

    // Classes then enums, each by name.
    let mut ordered: Vec<&ClassInfo> = classes.values().collect();
    ordered.sort_by(|a, b| a.name.cmp(&b.name));
    let mut ordered_enums: Vec<&EnumInfo> = enums.values().collect();
    ordered_enums.sort_by(|a, b| a.name.cmp(&b.name));
    let mut owners: Vec<Owner<'_>> = Vec::new();
    for class in ordered {
        owners.push(Owner { kind: "class", generics: &class.generics, is_enum: false, id: class.id, span: class.fields.first().map(|f| f.span) });
    }
    for e in ordered_enums {
        let span = e.variants.iter().flat_map(|v| &v.fields).next().map(|f| f.span);
        owners.push(Owner { kind: "enum", generics: &e.generics, is_enum: true, id: e.id, span });
    }
    for Owner { kind, generics, is_enum, id, span } in owners {
        // The definition itself, its parameters standing for unknown leaf types.
        let identity = tcx.identity_args(generics.len());
        let origin = if is_enum { tcx.enum_ty(id, identity) } else { tcx.class(id, identity) };
        let mut path = vec![origin];
        if find_cycle(origin, origin, (classes, enums), tcx, &mut HashSet::new(), &mut path) {
            tcx.set_param_names(generics.to_vec());
            let shown: Vec<String> = path.iter().take(MAX_DEPTH).map(|&ty| tcx.display(ty).to_string()).collect();
            diagnostics.push(type_error(
                format!(
                    "{kind} `{}` has a recursive by-value layout ({}); use `Ref<..>` for the recursive field",
                    tcx.display(origin),
                    shown.join(" -> ")
                ),
                span,
            ));
            tcx.set_param_names(Vec::new());
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
