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
use crate::hir::{self, HirClass, HirClassField, HirEnum, HirExpr, HirExprKind, HirFunction, HirProgram, HirVariant};
use crate::memory::MemoryPlan;
use crate::sema::ArenaReport;
use crate::span::Span;
use crate::ty::{subst, TyCtx, TyData, TyId};

/// Deepest type argument an instance may have. A polymorphic recursion such as
/// `f<T>` calling `f<Vec<T>>` grows without bound, so it hits this instead of looping.
const MAX_TYPE_DEPTH: usize = 32;

/// Largest type argument, counted as printed (a shared child counts each time it is used). Depth
/// alone is not enough: `f<T>` calling `f<Pair<T, T>>` doubles the printed size per level, so a type
/// of depth 32 would print as billions of names.
const MAX_TYPE_SIZE: usize = 4096;

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

/// Printed size of `ty`, saturating; memoised like [`depth`].
fn size(tcx: &TyCtx, ty: TyId, memo: &mut HashMap<TyId, usize>) -> usize {
    if let Some(&known) = memo.get(&ty) {
        return known;
    }
    let total = tcx.data(ty).children().into_iter().fold(1usize, |acc, child| acc.saturating_add(size(tcx, child, memo)));
    memo.insert(ty, total);
    total
}

/// Whether an argument is too deep or too big to instantiate with (and to mangle).
fn too_big(tcx: &TyCtx, args: &[TyId]) -> bool {
    let (mut depths, mut sizes) = (HashMap::new(), HashMap::new());
    args.iter().any(|&arg| depth(tcx, arg, &mut depths) > MAX_TYPE_DEPTH || size(tcx, arg, &mut sizes) > MAX_TYPE_SIZE)
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
        (TyData::Class(d1, a1), TyData::Class(d2, a2)) | (TyData::Enum(d1, a1), TyData::Enum(d2, a2)) => {
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
        | (TyData::Range(p), TyData::Range(a)) => bind(tcx, *p, *a, out),
        _ => false,
    }
}

/// Every `Class` or `Enum` type with arguments inside `ty`, outermost first.
fn class_instances(tcx: &TyCtx, ty: TyId, seen: &mut HashSet<TyId>, out: &mut Vec<TyId>) {
    if let TyData::Class(_, args) | TyData::Enum(_, args) = tcx.data(ty) {
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
    /// Where each instance was first called from, for diagnostics about its types.
    spans: Vec<Option<Span>>,
    queue: VecDeque<usize>,
    diagnostics: Vec<Diagnostic>,
}

impl<'a> Collector<'a> {
    /// The instance of function `origin` with `args`, queued for specialisation the first time it is seen.
    fn request_instance(&mut self, tcx: &TyCtx, origin: usize, args: Vec<TyId>, span: Option<Span>) -> Option<String> {
        let function = &self.hir.functions[origin];
        if too_big(tcx, &args) {
            let message = format!("polymorphic recursion: `{}` is instantiated with ever-growing types", function.name);
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
        self.spans.push(span);
        self.queue.push_back(self.instances.len() - 1);
        self.keys.insert(key, self.instances.len() - 1);
        Some(name)
    }

    /// Type arguments of a call to `callee` seen at type `callee_ty`, `None` once reported.
    fn infer_type_args(&mut self, tcx: &TyCtx, callee: &HirFunction, callee_ty: TyId, span: Option<Span>) -> Option<Vec<TyId>> {
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

    /// Points a call, construction or field access at the instance it now refers to.
    fn retarget_expr(&mut self, tcx: &TyCtx, e: &mut HirExpr) {
        match &mut e.kind {
            HirExprKind::Call { callee, .. } => {
                let (callee_ty, span) = (callee.ty, callee.span);
                let HirExprKind::Ident { name, .. } = &mut callee.kind else { return };
                let Some(&origin) = self.by_name.get(name.as_str()) else { return };
                let hir = self.hir;
                let Some(args) = self.infer_type_args(tcx, &hir.functions[origin], callee_ty, span) else { return };
                if let Some(instance) = self.request_instance(tcx, origin, args, span) {
                    *name = instance;
                }
            }
            HirExprKind::ObjectConstruct { class_name, .. } => retarget_class(tcx, e.ty, class_name),
            HirExprKind::EnumConstruct { enum_name, .. } => {
                if let Some(name) = enum_instance_name(tcx, e.ty) {
                    *enum_name = name;
                }
            }
            HirExprKind::ObjectField { receiver, class_name, .. } => retarget_class(tcx, receiver.ty, class_name),
            _ => {}
        }
    }
}

fn retarget_class(tcx: &TyCtx, ty: TyId, class_name: &mut String) {
    if let Some((def, args)) = tcx.class_args(ty).filter(|(_, args)| !too_big(tcx, args)) {
        *class_name = mangle(tcx.class_name(def), tcx, args);
    }
}

/// `TyCtx::record_name` for an instance: the name mono gives the `HirClass` of the object `ty` is,
/// or a borrow of (`Pair<i32,String>`, or just `Point` for a plain class). It reaches exactly as far
/// as `record_name`, so a managed `Ref<Foo>` has none. Codegen looks layouts up by it.
pub(crate) fn class_instance_name(tcx: &TyCtx, ty: TyId) -> Option<String> {
    tcx.record_def(ty)?;
    let (def, args) = tcx.class_args(ty)?;
    Some(mangle(tcx.class_name(def), tcx, args))
}

/// The name mono gives the `HirEnum` of the enum value `ty` (`Option<i32>`, or just `Shape`).
pub(crate) fn enum_instance_name(tcx: &TyCtx, ty: TyId) -> Option<String> {
    let (def, args) = tcx.enum_args(ty)?;
    Some(mangle(tcx.class_name(def), tcx, args))
}

/// Specialises the program reachable from `main`. `report` is the sema output for `hir`
/// (one root per function, same order), and planning diagnostics come back as errors.
pub fn monomorphize(hir: &HirProgram, tcx: &mut TyCtx, report: &ArenaReport) -> Result<Mono, Vec<Diagnostic>> {
    monomorphize_from(hir, tcx, report, false)
}

/// [`monomorphize`] with every non-generic function as a root, not only `main`: codegen unit tests
/// emit helpers that `main` never calls.
#[cfg(test)]
pub(crate) fn monomorphize_every_function(hir: &HirProgram, tcx: &mut TyCtx, report: &ArenaReport) -> Result<Mono, Vec<Diagnostic>> {
    monomorphize_from(hir, tcx, report, true)
}

fn monomorphize_from(hir: &HirProgram, tcx: &mut TyCtx, report: &ArenaReport, every_function: bool) -> Result<Mono, Vec<Diagnostic>> {
    ensure_one_root_per_function(hir, report)?;

    let (collector, functions, classes, enums) = collect_from(hir, tcx, every_function)?;
    let mut report = collector.report_per_instance(report);
    let mut program = HirProgram { classes, enums, functions };
    debug_assert!(no_params_left(&mut program, tcx), "a type parameter survived monomorphization");
    hir::number_nodes(&mut program);
    let memory_plan = plan_instances(&program, tcx, &mut report)?;
    Ok(Mono { hir: program, report, memory_plan, instances: collector.instances })
}

/// The user-facing half of monomorphization: collects every instance reachable from `main` and the
/// class instances they use, and reports what cannot be specialised (an instance that makes an
/// ill-formed type, a parameter no signature determines, polymorphic recursion, `T??`).
/// `bork check` runs just this, so it agrees with `bork build` without the planning passes.
pub fn check_instances(hir: &HirProgram, tcx: &mut TyCtx) -> Vec<Diagnostic> {
    collect_instances(hir, tcx).err().unwrap_or_default()
}

type Collected<'a> = (Collector<'a>, Vec<HirFunction>, Vec<HirClass>, Vec<HirEnum>);

fn collect_instances<'a>(hir: &'a HirProgram, tcx: &mut TyCtx) -> Result<Collected<'a>, Vec<Diagnostic>> {
    collect_from(hir, tcx, false)
}

fn collect_from<'a>(hir: &'a HirProgram, tcx: &mut TyCtx, every_function: bool) -> Result<Collected<'a>, Vec<Diagnostic>> {
    let mut collector = Collector::new(hir);
    if every_function {
        for (index, _) in hir.functions.iter().enumerate().filter(|(_, f)| f.generics.is_empty()) {
            collector.request_instance(tcx, index, Vec::new(), None);
        }
    } else if let Some(&main) = collector.by_name.get("main") {
        collector.request_instance(tcx, main, Vec::new(), None);
    }
    let mut functions = collector.specialize_reachable_functions(tcx);
    let (classes, enums) = collector.instantiate_types(tcx, &mut functions);
    if collector.diagnostics.is_empty() {
        Ok((collector, functions, classes, enums))
    } else {
        Err(collector.diagnostics)
    }
}

/// The sema report was built from the generic program, so it must line up with its functions.
fn ensure_one_root_per_function(hir: &HirProgram, report: &ArenaReport) -> Result<(), Vec<Diagnostic>> {
    if report.roots.len() == hir.functions.len() {
        return Ok(());
    }
    let message = format!("HIR has {} functions, report has {}", hir.functions.len(), report.roots.len());
    Err(vec![error(message, None)])
}

impl<'a> Collector<'a> {
    fn new(hir: &'a HirProgram) -> Self {
        let mut by_name = HashMap::new();
        for (index, function) in hir.functions.iter().enumerate() {
            by_name.entry(function.name.as_str()).or_insert(index);
        }
        Collector {
            hir,
            by_name,
            keys: HashMap::new(),
            instances: Vec::new(),
            origins: Vec::new(),
            spans: Vec::new(),
            queue: VecDeque::new(),
            diagnostics: Vec::new(),
        }
    }

    /// Drains the worklist: every requested instance becomes a concrete function, and specialising
    /// it may request more. Returns the functions in instance order.
    fn specialize_reachable_functions(&mut self, tcx: &mut TyCtx) -> Vec<HirFunction> {
        let mut functions = Vec::new();
        while let Some(index) = self.queue.pop_front() {
            functions.push(self.specialize(tcx, index));
        }
        functions
    }

    /// A clone of the generic function behind instance `index`, with its type arguments
    /// substituted and its calls and class names pointing at other instances.
    fn specialize(&mut self, tcx: &mut TyCtx, index: usize) -> HirFunction {
        let instance = self.instances[index].clone();
        let mut function = self.hir.functions[self.origins[index]].clone();
        function.name = instance.name;
        function.generics = Vec::new();
        if !instance.args.is_empty() {
            self.substitute_type_arguments(tcx, &mut function, &instance.args, self.spans[index]);
        }
        hir::rewrite(&mut function.body, &mut |ty| ty, &mut |e| self.retarget_expr(tcx, e));
        function
    }

    /// Replaces `Param(i)` by `args[i]` in the signature and every type of the body.
    /// The instantiated types are checked too: `lower_type` lets a parameter through
    /// (`Ref<T>`, `Vec<T>`), so `T = i32` would otherwise reach codegen as `Ref<i32>`.
    fn substitute_type_arguments(
        &mut self,
        tcx: &mut TyCtx,
        function: &mut HirFunction,
        args: &[TyId],
        call_span: Option<Span>,
    ) {
        let (mut nested_nullable, mut ill_formed) = (false, Vec::<String>::new());
        let mut checked = HashSet::new();
        let mut substitute = |ty: TyId| {
            let ty = subst(tcx, ty, args).unwrap_or_else(|_| {
                nested_nullable = true;
                tcx.error()
            });
            if checked.insert(ty) {
                ill_formed.extend(tcx.ill_formed(ty).filter(|rule| !ill_formed.contains(rule)));
            }
            ty
        };
        function.params.iter_mut().for_each(|param| param.ty = substitute(param.ty));
        function.return_ty = substitute(function.return_ty);
        hir::resolve_types(&mut function.body, &mut substitute);
        if nested_nullable {
            let message = format!("type arguments of `{}` would make a nullable of a nullable", function.name);
            self.diagnostics.push(error(message, Some(function.name_span)));
        }
        for rule in ill_formed {
            let span = call_span.or(Some(function.name_span));
            self.diagnostics.push(error(format!("`{}`: {rule}", function.name), span));
        }
    }

    /// Plain classes and enums are kept; every one applied to type arguments in a signature, a body
    /// or a field becomes an instance, transitively through the fields of instances.
    fn instantiate_types(&mut self, tcx: &mut TyCtx, functions: &mut [HirFunction]) -> (Vec<HirClass>, Vec<HirEnum>) {
        let (mut seen, mut pending) = (HashSet::new(), Vec::new());
        for function in functions {
            function.params.iter().for_each(|param| class_instances(tcx, param.ty, &mut seen, &mut pending));
            class_instances(tcx, function.return_ty, &mut seen, &mut pending);
            hir::resolve_types(&mut function.body, &mut |ty| {
                class_instances(tcx, ty, &mut seen, &mut pending);
                ty
            });
        }
        let mut classes: Vec<HirClass> =
            self.hir.classes.iter().filter(|class| class.generics.is_empty()).cloned().collect();
        for field in classes.iter().flat_map(|class| &class.fields) {
            class_instances(tcx, field.ty, &mut seen, &mut pending);
        }
        let mut enums: Vec<HirEnum> = self.hir.enums.iter().filter(|e| e.generics.is_empty()).cloned().collect();
        let payloads = enums.iter().flat_map(|e| &e.variants).flat_map(|v| &v.fields);
        for field in payloads {
            class_instances(tcx, field.ty, &mut seen, &mut pending);
        }
        while let Some(ty) = pending.pop() {
            if matches!(tcx.data(ty), TyData::Enum(..)) {
                enums.extend(self.instantiate_enum(tcx, ty, &mut seen, &mut pending));
            } else {
                classes.extend(self.instantiate_class(tcx, ty, &mut seen, &mut pending));
            }
        }
        (classes, enums)
    }

    /// The concrete enum for `ty` (an enum applied to arguments); payloads may use further
    /// instances, which are pushed on `pending`.
    fn instantiate_enum(
        &mut self,
        tcx: &mut TyCtx,
        ty: TyId,
        seen: &mut HashSet<TyId>,
        pending: &mut Vec<TyId>,
    ) -> Option<HirEnum> {
        let TyData::Enum(def, args) = tcx.data(ty).clone() else { return None };
        if too_big(tcx, &args) {
            let message = format!("enum `{}` is instantiated with ever-growing types", tcx.class_name(def));
            self.diagnostics.push(error(message, None));
            return None;
        }
        let Some(generic) = self.hir.enums.iter().find(|e| e.name == tcx.class_name(def)) else {
            let message = format!("internal: no definition of enum `{}`", tcx.class_name(def));
            self.diagnostics.push(error(message, None));
            return None;
        };
        let variants = generic
            .variants
            .iter()
            .map(|variant| HirVariant {
                name: variant.name.clone(),
                fields: self.instantiate_fields(tcx, ty, &variant.fields, &args, "payload", seen, pending),
            })
            .collect();
        Some(HirEnum { name: mangle(tcx.class_name(def), tcx, &args), generics: Vec::new(), variants })
    }

    /// The concrete class for `ty` (a class applied to arguments); its fields may use further
    /// instances, which are pushed on `pending`.
    fn instantiate_class(
        &mut self,
        tcx: &mut TyCtx,
        ty: TyId,
        seen: &mut HashSet<TyId>,
        pending: &mut Vec<TyId>,
    ) -> Option<HirClass> {
        let TyData::Class(def, args) = tcx.data(ty).clone() else { return None };
        if too_big(tcx, &args) {
            let message = format!("class `{}` is instantiated with ever-growing types", tcx.class_name(def));
            self.diagnostics.push(error(message, None));
            return None;
        }
        let Some(generic) = self.hir.classes.iter().find(|class| class.name == tcx.class_name(def)) else {
            let message = format!("internal: no definition of class `{}`", tcx.class_name(def));
            self.diagnostics.push(error(message, None));
            return None;
        };
        let fields = self.instantiate_fields(tcx, ty, &generic.fields, &args, "field", seen, pending);
        Some(HirClass { name: mangle(tcx.class_name(def), tcx, &args), generics: Vec::new(), fields })
    }

    /// `fields` of the generic class or enum behind `ty`, with `args` substituted. Each type is
    /// checked, and the instances it uses are pushed on `pending`.
    fn instantiate_fields(
        &mut self,
        tcx: &mut TyCtx,
        ty: TyId,
        fields: &[HirClassField],
        args: &[TyId],
        what: &str,
        seen: &mut HashSet<TyId>,
        pending: &mut Vec<TyId>,
    ) -> Vec<HirClassField> {
        let mut out = Vec::new();
        for field in fields {
            let field_ty = subst(tcx, field.ty, args).unwrap_or_else(|_| {
                let message = format!("type arguments of `{}` would make a nullable of a nullable", tcx.display(ty));
                self.diagnostics.push(error(message, None));
                tcx.error()
            });
            if let Some(rule) = tcx.ill_formed(field_ty) {
                let message = format!("{what} `{}` of `{}`: {rule}", field.name, tcx.display(ty));
                self.diagnostics.push(error(message, None));
            }
            class_instances(tcx, field_ty, seen, pending);
            out.push(HirClassField { name: field.name.clone(), ty: field_ty, span: field.span, source: field.source });
        }
        out
    }

    /// One report root per instance: the generic function's root, cloned and relabelled.
    fn report_per_instance(&self, generic: &ArenaReport) -> ArenaReport {
        let roots = self.origins.iter().zip(&self.instances).map(|(&origin, instance)| {
            let mut root = generic.roots[origin].clone();
            root.label = format!("fun {}", instance.name);
            root
        });
        ArenaReport { roots: roots.collect() }
    }
}

/// Ownership and memory planning, now on concrete types: escape, the memory plan and the codegen
/// push stamps. A failure here is not a user error (the generic body was already accepted).
fn plan_instances(program: &HirProgram, tcx: &TyCtx, report: &mut ArenaReport) -> Result<MemoryPlan, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for function in &program.functions {
        crate::escape::check_function(function, tcx, &mut diagnostics);
    }
    let (memory_plan, mut plan_diagnostics) = crate::memory::plan(program, tcx, report);
    diagnostics.append(&mut plan_diagnostics);
    if diagnostics.is_empty() {
        if let Err(ice) = crate::region_walk::stamp_codegen_push(program, tcx, report) {
            diagnostics.push(ice);
        }
    }
    if diagnostics.is_empty() { Ok(memory_plan) } else { Err(diagnostics) }
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
    for variant in program.enums.iter().flat_map(|e| &e.variants) {
        variant.fields.iter().for_each(|field| {
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
