//! Monomorphization (#47): turns the generic HIR into concrete instances.
//!
//! A worklist starts at `main`. Every call to a user function becomes an instance keyed by
//! `(function, type arguments)`; the arguments are recovered by matching the generic signature
//! against the callee's instantiated type, which typeck already resolved. Each instance is a clone of
//! the generic `HirFunction` with its types substituted, called by a mangled name. Classes used with
//! type arguments are emitted as instances too. Escape and memory planning run on the result, so
//! they see concrete types only. Emitting LLVM for the result is a later step (TS-P3-07).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::{self, HirClass, HirClassField, HirExpr, HirExprKind, HirFunction, HirProgram};
use crate::memory::MemoryPlan;
use crate::sema::ArenaReport;
use crate::span::Span;
use crate::ty::{subst, TyCtx, TyData, TyId};

/// Deepest type argument an instance may have. A polymorphic recursion such as
/// `f<T>` calling `f<Vec<T>>` grows without bound, so it hits this instead of looping.
const MAX_TYPE_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Mangled name, e.g. `id<i32>`; a non-generic function keeps its own name.
    pub name: String,
    /// The generic function this is an instance of.
    pub origin: String,
    pub args: Vec<TyId>,
}

#[derive(Debug)]
pub struct Mono {
    /// Concrete program: no `Param` in any type, no generic class, ids renumbered.
    pub hir: HirProgram,
    /// One root per function of `hir`, cloned from the generic function's root.
    pub report: ArenaReport,
    pub memory_plan: MemoryPlan,
    pub instances: Vec<Instance>,
}

fn error(message: String, span: Option<Span>) -> Diagnostic {
    Diagnostic { phase: Phase::Type, severity: Severity::Error, message, span }
}

fn mangle(base: &str, tcx: &TyCtx, args: &[TyId]) -> String {
    if args.is_empty() {
        return base.to_string();
    }
    let args: Vec<String> = args.iter().map(|&arg| tcx.display(arg).to_string().replace(' ', "")).collect();
    format!("{base}<{}>", args.join(","))
}

/// Nesting depth of `ty`. Types are interned DAGs (`Pair<T, T>` shares a child), so it is memoised.
fn depth(tcx: &TyCtx, ty: TyId, memo: &mut HashMap<TyId, usize>) -> usize {
    if let Some(&known) = memo.get(&ty) {
        return known;
    }
    let deepest = tcx.data(ty).children().into_iter().map(|child| depth(tcx, child, memo)).max().unwrap_or(0);
    memo.insert(ty, deepest + 1);
    deepest + 1
}

fn too_deep(tcx: &TyCtx, args: &[TyId]) -> Option<TyId> {
    let mut memo = HashMap::new();
    args.iter().copied().find(|&arg| depth(tcx, arg, &mut memo) > MAX_TYPE_DEPTH)
}

/// One-way match of the generic `pattern` against the concrete `actual`, binding `Param(i)` to
/// `out[i]`. False when the shapes differ or a parameter would get two different types.
fn bind(tcx: &TyCtx, pattern: TyId, actual: TyId, out: &mut [Option<TyId>]) -> bool {
    if !tcx.has_param(pattern) {
        return pattern == actual;
    }
    match (tcx.data(pattern), tcx.data(actual)) {
        (TyData::Param(idx), _) => match out.get_mut(idx.0 as usize) {
            Some(slot) => *slot.get_or_insert(actual) == actual,
            None => false,
        },
        (TyData::Class(d1, a1), TyData::Class(d2, a2)) => {
            d1 == d2 && a1.len() == a2.len() && a1.iter().zip(a2).all(|(&p, &a)| bind(tcx, p, a, out))
        }
        (TyData::Array { elem: e1, len: l1 }, TyData::Array { elem: e2, len: l2 }) => {
            l1 == l2 && bind(tcx, *e1, *e2, out)
        }
        (TyData::Func { params: p1, ret: r1 }, TyData::Func { params: p2, ret: r2 }) => {
            p1.len() == p2.len() && p1.iter().zip(p2).all(|(&p, &a)| bind(tcx, p, a, out)) && bind(tcx, *r1, *r2, out)
        }
        (TyData::Ref(p), TyData::Ref(a))
        | (TyData::ManagedRef(p), TyData::ManagedRef(a))
        | (TyData::Vec(p), TyData::Vec(a))
        | (TyData::Range(p), TyData::Range(a))
        | (TyData::Nullable(p), TyData::Nullable(a)) => bind(tcx, *p, *a, out),
        _ => false,
    }
}

/// Every `Class` type with arguments inside `ty`, outermost first.
fn class_instances(tcx: &TyCtx, ty: TyId, seen: &mut HashSet<TyId>, out: &mut Vec<TyId>) {
    if let TyData::Class(_, args) = tcx.data(ty) {
        if !args.is_empty() && seen.insert(ty) {
            out.push(ty);
        }
    }
    for child in tcx.data(ty).children() {
        class_instances(tcx, child, seen, out);
    }
}

struct Collector<'a> {
    hir: &'a HirProgram,
    by_name: HashMap<&'a str, usize>,
    /// `(index in hir.functions, type arguments)` -> index in `instances`.
    keys: HashMap<(usize, Vec<TyId>), usize>,
    instances: Vec<Instance>,
    origins: Vec<usize>,
    queue: VecDeque<usize>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Collector<'a> {
    /// The instance of function `origin` with `args`, queued the first time it is seen.
    fn intern(&mut self, tcx: &TyCtx, origin: usize, args: Vec<TyId>, span: Option<Span>) -> Option<String> {
        let function = &self.hir.functions[origin];
        if let Some(ty) = too_deep(tcx, &args) {
            let message = format!(
                "polymorphic recursion: `{}` is instantiated with ever-growing types (`{}`)",
                function.name,
                tcx.display(ty)
            );
            self.diagnostics.push(error(message, span));
            return None;
        }
        let key = (origin, args);
        if let Some(&index) = self.keys.get(&key) {
            return Some(self.instances[index].name.clone());
        }
        let name = mangle(&function.name, tcx, &key.1);
        self.instances.push(Instance { name: name.clone(), origin: function.name.clone(), args: key.1.clone() });
        self.origins.push(origin);
        self.queue.push_back(self.instances.len() - 1);
        self.keys.insert(key, self.instances.len() - 1);
        Some(name)
    }

    /// Type arguments of a call to `callee` seen at type `callee_ty`, `None` once reported.
    fn call_args(&mut self, tcx: &TyCtx, callee: &HirFunction, callee_ty: TyId, span: Option<Span>) -> Option<Vec<TyId>> {
        if callee.generics.is_empty() {
            return Some(Vec::new());
        }
        let mut out = vec![None; callee.generics.len()];
        let matched = match tcx.data(callee_ty) {
            TyData::Func { params, ret } => {
                params.len() == callee.params.len()
                    && callee.params.iter().zip(params).all(|(p, &actual)| bind(tcx, p.ty, actual, &mut out))
                    && bind(tcx, callee.return_ty, *ret, &mut out)
            }
            _ => false,
        };
        let missing = out.iter().position(Option::is_none);
        if !matched || missing.is_some() {
            let why = match missing {
                Some(index) if matched => format!(
                    "type parameter `{}` of `{}` is not determined by its signature, which monomorphization does not support yet",
                    callee.generics[index], callee.name
                ),
                _ => format!("cannot determine the type arguments of this call to `{}`", callee.name),
            };
            self.diagnostics.push(error(why, span));
            return None;
        }
        Some(out.into_iter().flatten().collect())
    }

    /// Renames the call target and the class names of one expression to their instances.
    fn visit(&mut self, tcx: &TyCtx, e: &mut HirExpr) {
        match &mut e.kind {
            HirExprKind::Call { callee, .. } => {
                let (callee_ty, span) = (callee.ty, callee.span);
                let HirExprKind::Ident { name, .. } = &mut callee.kind else { return };
                let Some(&origin) = self.by_name.get(name.as_str()) else { return };
                let hir = self.hir;
                let Some(args) = self.call_args(tcx, &hir.functions[origin], callee_ty, span) else { return };
                if let Some(instance) = self.intern(tcx, origin, args, span) {
                    *name = instance;
                }
            }
            HirExprKind::ObjectConstruct { class_name, .. } => rename_class(tcx, e.ty, class_name),
            HirExprKind::ObjectField { receiver, class_name, .. } => rename_class(tcx, receiver.ty, class_name),
            _ => {}
        }
    }
}

fn rename_class(tcx: &TyCtx, ty: TyId, class_name: &mut String) {
    if let Some((def, args)) = tcx.class_args(ty) {
        *class_name = mangle(tcx.class_name(def), tcx, args);
    }
}

/// Specialises the program reachable from `main`. `report` is the sema output for `hir`
/// (one root per function, same order), and planning diagnostics come back as errors.
pub fn monomorphize(hir: &HirProgram, tcx: &mut TyCtx, report: &ArenaReport) -> Result<Mono, Vec<Diagnostic>> {
    if report.roots.len() != hir.functions.len() {
        let message = format!("HIR has {} functions, report has {}", hir.functions.len(), report.roots.len());
        return Err(vec![error(message, None)]);
    }
    let mut by_name = HashMap::new();
    for (index, function) in hir.functions.iter().enumerate() {
        by_name.entry(function.name.as_str()).or_insert(index);
    }
    let mut collector = Collector {
        hir,
        by_name,
        keys: HashMap::new(),
        instances: Vec::new(),
        origins: Vec::new(),
        queue: VecDeque::new(),
        diagnostics: Vec::new(),
    };
    if let Some(&main) = collector.by_name.get("main") {
        collector.intern(tcx, main, Vec::new(), None);
    }

    let mut functions = Vec::new();
    while let Some(index) = collector.queue.pop_front() {
        let instance = collector.instances[index].clone();
        let mut function = hir.functions[collector.origins[index]].clone();
        function.name = instance.name;
        function.generics = Vec::new();
        if !instance.args.is_empty() {
            let mut failed = false;
            let mut sub = |ty: TyId| match subst(tcx, ty, &instance.args) {
                Ok(ty) => ty,
                Err(_) => {
                    failed = true;
                    tcx.error()
                }
            };
            function.params.iter_mut().for_each(|param| param.ty = sub(param.ty));
            function.return_ty = sub(function.return_ty);
            hir::resolve_types(&mut function.body, &mut sub);
            if failed {
                let message = format!("type arguments of `{}` would make a nullable of a nullable", function.name);
                collector.diagnostics.push(error(message, Some(function.name_span)));
            }
        }
        hir::rewrite(&mut function.body, &mut |ty| ty, &mut |e| collector.visit(tcx, e));
        functions.push(function);
    }

    // Classes used with type arguments get an instance too; plain classes are kept as they are.
    let (mut seen, mut pending) = (HashSet::new(), Vec::new());
    for function in &mut functions {
        function.params.iter().for_each(|param| class_instances(tcx, param.ty, &mut seen, &mut pending));
        class_instances(tcx, function.return_ty, &mut seen, &mut pending);
        hir::resolve_types(&mut function.body, &mut |ty| {
            class_instances(tcx, ty, &mut seen, &mut pending);
            ty
        });
    }
    let mut classes: Vec<HirClass> = hir.classes.iter().filter(|class| class.generics.is_empty()).cloned().collect();
    for field in classes.iter().flat_map(|class| &class.fields) {
        class_instances(tcx, field.ty, &mut seen, &mut pending);
    }
    while let Some(ty) = pending.pop() {
        let TyData::Class(def, args) = tcx.data(ty).clone() else { continue };
        if let Some(arg) = too_deep(tcx, &args) {
            let message = format!("class `{}` is instantiated with ever-growing types (`{}`)", tcx.class_name(def), tcx.display(arg));
            collector.diagnostics.push(error(message, None));
            continue;
        }
        let Some(generic) = hir.classes.iter().find(|class| class.name == tcx.class_name(def)) else {
            let message = format!("internal: no definition of class `{}`", tcx.class_name(def));
            collector.diagnostics.push(error(message, None));
            continue;
        };
        let mut fields = Vec::new();
        for field in &generic.fields {
            let field_ty = subst(tcx, field.ty, &args).unwrap_or_else(|_| {
                let message = format!("type arguments of `{}` would make a nullable of a nullable", tcx.display(ty));
                collector.diagnostics.push(error(message, None));
                tcx.error()
            });
            class_instances(tcx, field_ty, &mut seen, &mut pending);
            fields.push(HirClassField { name: field.name.clone(), ty: field_ty });
        }
        classes.push(HirClass { name: mangle(tcx.class_name(def), tcx, &args), generics: Vec::new(), fields });
    }

    if !collector.diagnostics.is_empty() {
        return Err(collector.diagnostics);
    }

    let mut report = ArenaReport {
        roots: collector
            .origins
            .iter()
            .zip(&collector.instances)
            .map(|(&origin, instance)| {
                let mut root = report.roots[origin].clone();
                root.label = format!("fun {}", instance.name);
                root
            })
            .collect(),
    };
    let mut program = HirProgram { classes, functions };
    debug_assert!(no_params_left(&mut program, tcx), "a type parameter survived monomorphization");
    hir::number_nodes(&mut program);

    let mut diagnostics = Vec::new();
    for function in &program.functions {
        crate::escape::check_function(function, tcx, &mut diagnostics);
    }
    let (memory_plan, mut plan_diagnostics) = crate::memory::plan(&program, tcx, &report);
    diagnostics.append(&mut plan_diagnostics);
    if diagnostics.is_empty() {
        if let Err(ice) = crate::region_walk::stamp_codegen_push(&program, tcx, &mut report) {
            diagnostics.push(ice);
        }
    }
    if !diagnostics.is_empty() {
        return Err(diagnostics);
    }
    Ok(Mono { hir: program, report, memory_plan, instances: collector.instances })
}

/// Whether no type of `program` mentions a type parameter.
fn no_params_left(program: &mut HirProgram, tcx: &TyCtx) -> bool {
    let mut clean = true;
    let mut check = |ty: TyId| {
        clean &= !tcx.has_param(ty);
        ty
    };
    for class in &program.classes {
        class.fields.iter().for_each(|field| {
            check(field.ty);
        });
    }
    for function in &mut program.functions {
        function.params.iter().for_each(|param| {
            check(param.ty);
        });
        check(function.return_ty);
        hir::resolve_types(&mut function.body, &mut check);
    }
    clean
}

#[cfg(test)]
mod tests;
