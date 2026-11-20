//! Compiler frontend orchestration (parse -> typeck -> sema -> HIR).

use crate::diag::{self, Diagnostic};
use crate::hir::HirProgram;
use crate::sema::ArenaReport;
use crate::ty::TyCtx;
use crate::typeck;

/// Outcome of one frontend run over a source string.
pub struct CheckResult {
    /// Ownership arenas, kept even when later phases report errors. `None` only
    /// when the source failed to parse.
    pub report: Option<ArenaReport>,
    /// Typed HIR, present exactly when `diagnostics` is empty.
    pub hir: Option<HirProgram>,
    pub diagnostics: Vec<Diagnostic>,
    /// Warnings of a clean check: they never make it fail.
    pub warnings: Vec<Diagnostic>,
    /// Member names resolved by typeck (bare fields, method calls, method declarations).
    pub members: crate::typeck::MemberResolutions,
    /// Owns the type ids carried by `hir`; empty when the source failed to parse.
    pub tcx: TyCtx,
    /// Every numbered AST node with its span; the LSP resolves a position to an `AstId` with it.
    pub node_index: Vec<(crate::span::Span, crate::span::AstId)>,
}

impl CheckResult {
    pub fn is_ok(&self) -> bool {
        self.diagnostics.is_empty()
    }
}

fn has_generics(hir: &crate::hir::HirProgram) -> bool {
    hir.functions.iter().any(|function| !function.generics.is_empty())
        || hir.classes.iter().any(|class| !class.generics.is_empty())
    || hir.enums.iter().any(|e| !e.generics.is_empty())
}

fn label_prelude_functions(hir: &mut HirProgram, functions: &[crate::ast::Function]) {
    // shortcut: sources use chunk export names; track per-item origins before adding unlisted helpers.
    for declaration in functions {
        if let Some(function) = hir.functions.iter_mut().find(|function| function.name == declaration.name) {
            function.source = crate::prelude::source_file(&function.name);
        }
    }
}

/// Parse, analyze ownership, and lower program to typed HIR.
pub fn check(source: &str) -> CheckResult {
    check_with(source, true)
}

/// [`check`], optionally without the HIR escape pass (the MIR escape analysis, #217, runs on programs it rejects).
#[doc(hidden)]
pub fn check_with(source: &str, escape: bool) -> CheckResult {
    let (mut program, node_index) = match crate::parse_indexed(source) {
        Ok(parsed) => parsed,
        Err(err) => {
            return CheckResult {
                report: None,
                hir: None,
                diagnostics: vec![diag::from_parse(source, &err)],
                warnings: Vec::new(),
                members: Default::default(),
                tcx: TyCtx::new(&Default::default()),
                node_index: Vec::new(),
            }
        }
    };

    let user_functions = program.functions.len();
    let user_enums = program.enums.len();
    crate::prelude::add_prelude(&mut program, source, node_index.len() as u32);
    let typeck::Typeck { mut hir, node_tys, members, option_if_vals, diagnostics: mut type_diagnostics, tcx } =
        typeck::check_with_members(&program);
    label_prelude_functions(&mut hir, &program.functions[user_functions..]);
    for e in &program.enums[user_enums..] {
        if let Some(hir_enum) = hir.enums.iter_mut().find(|h| h.name == e.name.name) {
            for field in hir_enum.variants.iter_mut().flat_map(|v| &mut v.fields) {
                field.source = crate::prelude::source_file(&e.name.name);
            }
        }
    }
    let (report, ownership_errors) =
        crate::sema::analyze_with_members(&program, node_tys, members.clone(), option_if_vals, &tcx);
    let mut diagnostics: Vec<_> = ownership_errors.iter().map(diag::from_sema).collect();
    diagnostics.append(&mut type_diagnostics);

    // Instances of generic code are checked here too, so `check` and the LSP agree with `build`.
    // It runs on a copy of the type context (specialising interns new types), and the HIR that
    // hover and the LSP read stays the generic one.
    if diagnostics.is_empty() && has_generics(&hir) {
        diagnostics.extend(crate::mono::check_instances(&hir, &mut tcx.clone()));
    }
    let mut warnings = Vec::new();
    let mut hir = diagnostics.is_empty().then_some(hir);
    if let Some(mut program) = hir {
        crate::hir::number_nodes(&mut program);
        if escape {
            // Object lifetimes first: a program they reject would otherwise also get the String rules' more general
            // "inner region" message for the same value.
            diagnostics.extend(object_lifetimes(&program, &report, &tcx, source));
            if diagnostics.is_empty() {
                for function in &program.functions {
                    crate::escape::check_function(function, &tcx, &mut diagnostics);
                }
            }
        }
        if diagnostics.is_empty() && has_generics(&program) {
            diagnostics.extend(crate::mono::check_instance_regions(&program, &mut tcx.clone(), &report, escape));
        }
        if diagnostics.is_empty() && escape {
            warnings = ref_expiry_warnings(&program, &report, &tcx);
        }
        hir = diagnostics.is_empty().then_some(program);
    }

    CheckResult {
        report: Some(report),
        hir,
        diagnostics,
        warnings,
        members,
        tcx,
        node_index,
    }
}

/// Only the MIR object-lifetime errors of `source` (what `check` adds on top of the HIR pass), for tests.
#[doc(hidden)]
pub fn object_lifetime_errors(source: &str) -> Vec<Diagnostic> {
    let result = check_with(source, false);
    match (&result.hir, &result.report) {
        (Some(hir), Some(report)) => object_lifetimes(hir, report, &result.tcx, source),
        _ => Vec::new(),
    }
}

/// `mir::ref_expiry` over what `main` reaches, as warnings.
fn ref_expiry_warnings(hir: &HirProgram, report: &ArenaReport, tcx: &TyCtx) -> Vec<Diagnostic> {
    use crate::mir::{build::build_function_results_neutral, ref_expiry};
    let mut tcx = tcx.clone();
    let Ok(mono) = crate::mono::specialise_for_analysis(hir, &mut tcx, report) else { return Vec::new() };
    let Ok(results) = build_function_results_neutral(&mono, &tcx) else { return Vec::new() };
    let mut found: Vec<_> = results.iter().flatten().flat_map(ref_expiry::check).collect();
    found.sort_by_key(|(span, message)| (span.start, span.end, message.clone()));
    found.dedup();
    found
        .into_iter()
        .map(|(span, message)| Diagnostic { phase: diag::Phase::Ownership, severity: diag::Severity::Warning, message, span: Some(span), notes: Vec::new() })
        .collect()
}

/// Object-lifetime errors from MIR (`mir::escape`): an object stored into a longer-lived owner, or returned, from a
/// region that closes first. Only what `main` reaches is analysed, as `gate` does. A
/// `move` that would leave an object in a dying arena is the same error: `source` names the moved binding.
fn object_lifetimes(hir: &HirProgram, report: &ArenaReport, tcx: &TyCtx, source: &str) -> Vec<Diagnostic> {
    use crate::mir::{build::{build_function_results_neutral, BuildError}, capability::Capability, escape};
    let mut tcx = tcx.clone();
    let Ok(mono) = crate::mono::specialise_for_analysis(hir, &mut tcx, report) else { return Vec::new() };
    let Ok(results) = build_function_results_neutral(&mono, &tcx) else { return Vec::new() };
    let stored = |name: &str| format!("`{name}` is freed before the value that stores it, but that value would still point at it; build it directly in the constructor, or declare it at the top level of the function");
    let mut found: Vec<(crate::span::Span, String)> = Vec::new();
    for result in &results {
        match result {
            Ok(body) => found.extend(escape::check(body)),
            Err(BuildError::Unsupported(u)) if u.capability == Capability::MoveEscapesArena => {
                let name = source.get(u.span.start..u.span.end).unwrap_or("value");
                found.push((u.span, stored(name)));
            }
            Err(BuildError::Unsupported(u)) if u.capability == Capability::BorrowedStoreEscapes => found.push((u.span, u.message().to_string())),
            Err(_) => {}
        }
    }
    found.sort_by_key(|(span, message)| (span.start, span.end, message.clone()));
    found.dedup();
    found
        .into_iter()
        .map(|(span, message)| Diagnostic { phase: diag::Phase::Ownership, severity: diag::Severity::Error, message, span: Some(span), notes: Vec::new() })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Phase;

    fn hir_of(source: &str) -> HirProgram {
        let result = check(source);
        assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
        result.hir.expect("clean check produces HIR")
    }

    #[test]
    fn check_rejects_parse_error() {
        let result = check("fun oops(");
        assert!(result.diagnostics.iter().any(|d| d.phase == Phase::Parse));
        assert!(result.report.is_none());
        assert!(result.hir.is_none());
    }

    #[test]
    fn check_ok_on_minimal_typed_program() {
        assert_eq!(hir_of("fun main(): i32 { return 1 }").functions.len(), 1);
    }

    #[test]
    fn filtered_intrinsic_declarations_do_not_shift_prelude_sources() {
        let source = "fun print(value: i32) {}\nclass Node { v: i32 }\nfun main() {\n val node = Node(1)\n val reference: Ref<Node> = node\n val option = optionFromRef::<Node>(reference)\n}\n";
        let (mut program, index) = crate::parse_indexed(source).unwrap();
        let user_functions = program.functions.len();
        crate::prelude::add_prelude(&mut program, source, index.len() as u32);
        let typeck::Typeck { mut hir, diagnostics, .. } = typeck::check_with_members(&program);
        assert!(diagnostics.iter().any(|d| d.message.contains("cannot redefine builtin function `print`")));
        label_prelude_functions(&mut hir, &program.functions[user_functions..]);
        assert!(!hir.functions.iter().any(|function| function.name == "print"));
        for name in ["optionFromRef", "optionToRef"] {
            let function = hir.functions.iter().find(|function| function.name == name).unwrap();
            assert_eq!(function.source, Some("<bork-prelude>/option_conv.bork"), "{name}");
        }
        assert_eq!(hir.functions.iter().find(|function| function.name == "main").unwrap().source, None);
    }

    #[test]
    fn check_rejects_ownership_error() {
        let src = r#"
fun main() {
    var s: String = "hi"
    move (s) {}
    val t = s
}
"#;
        let result = check(src);
        assert!(result
            .diagnostics
            .iter()
            .any(|d| d.phase == Phase::Ownership));
        assert!(result.hir.is_none());
    }

    #[test]
    fn instantiation_error_notes_the_generic_definition() {
        let result = check("fun g<T>(x: T): Ref<T> {\n    return None\n}\nfun main(): i32 {\n    val r = g(1)\n    return 0\n}\n");
        let error = result.diagnostics.iter().find(|d| d.message.contains("managed Ref target")).expect("instance error");
        assert_eq!(error.notes.len(), 1, "{error:?}");
        assert!(error.notes[0].span.is_some() && error.notes[0].message.contains("generic definition"));
    }

    #[test]
    fn report_survives_semantic_errors() {
        let src = r#"
fun main(): i32 {
    var s: String = "a"
    val t = move s
    val u = s
    return 1 + "x"
}
"#;
        let result = check(src);
        assert!(!result.diagnostics.is_empty());
        assert!(result.report.is_some(), "arenas are kept alongside errors");
    }

    #[test]
    fn reports_ownership_and_type_together() {
        let src = r#"
fun main(): i32 {
    var s: String = "a"
    val t = move s
    val u = s
    return 1 + "x"
}
"#;
        let diagnostics = check(src).diagnostics;
        assert!(diagnostics.iter().any(|d| d.phase == Phase::Ownership));
        assert!(diagnostics.iter().any(|d| d.phase == Phase::Type));
    }

    #[test]
    fn check_lowers_function_shells() {
        let src = r#"
fun add(val a: Int, var b: Int): Int {
    return a
}
"#;
        let checked = check(src);
        assert!(checked.diagnostics.is_empty(), "{:?}", checked.diagnostics);
        let (hir, tcx) = (checked.hir.expect("clean check produces HIR"), checked.tcx);
        let i32_ty = tcx.prim_id(crate::hir::Prim::I32);
        assert_eq!(hir.functions.len(), 1);
        let f = &hir.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.return_ty, i32_ty);
        assert_eq!(f.params.len(), 2);
        assert_eq!(f.params[0].name, "a");
        assert_eq!(f.params[0].kind, crate::ast::BindingKind::Val);
        assert_eq!(f.params[0].ty, i32_ty);
        assert_eq!(f.params[1].name, "b");
        assert_eq!(f.params[1].kind, crate::ast::BindingKind::Var);
        assert_eq!(f.params[1].ty, i32_ty);
        assert!(matches!(
            f.body.stmts.as_slice(),
            [crate::hir::HirStmt::Return { value: Some(_) }]
        ));
    }

    #[test]
    fn warns_when_a_ref_stored_in_an_outer_object_dies_with_its_block() {
        let held = |inner: &str| {
            let src = format!(
                "class Node {{ id: i32 }}\nclass P {{ r: Ref<Node> }}\nfun main(): i32 {{\n val r0: Ref<Node> = Node(1)\n var p = P(r0)\n {{\n {inner}\n }}\n return 0\n}}\n"
            );
            check(&src)
        };
        let result = held("val r: Ref<Node> = Node(5)\n p = P(r)");
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let messages: Vec<_> = result.warnings.iter().map(|d| d.message.as_str()).collect();
        assert_eq!(messages.len(), 1, "{messages:?}");
        assert!(messages[0].contains("`r`") && messages[0].contains("`p`"), "{messages:?}");
        assert_eq!(result.warnings[0].severity, diag::Severity::Warning);
        // Same block, or a Ref made outside the block: nothing expires early.
        assert!(held("val q = P(r0)\n p = q").warnings.is_empty());
        assert!(held("p = P(r0)").warnings.is_empty());
        // Through a named local the Ref still expires.
        assert_eq!(held("val r: Ref<Node> = Node(5)\n val q = P(r)\n p = move q").warnings.len(), 1);
    }
}
