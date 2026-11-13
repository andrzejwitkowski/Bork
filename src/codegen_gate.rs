//! Codegen subset checks shared by `bork build` and the LSP (no LLVM dependency).
//!
//! There is no list of rejected shapes here: MIR building reports the construct it cannot lower as a named
//! [`Capability`](crate::mir::Capability) and this turns each into a diagnostic (RFC 0006 3.8).

use crate::diag::{Diagnostic, Phase, Severity};
use crate::hir::HirProgram;
use crate::mir::build::{build_function_results, BuildError};
use crate::mir::Body;
use crate::mono::Mono;
use crate::sema::ArenaReport;
use crate::ty::TyCtx;

/// Diagnostics for the constructs of `hir` codegen cannot lower, instance by instance. Like `check` and `build`, it
/// specialises what `main` reaches (every non-generic function when there is no `main`), so a program that cannot be
/// specialised yields none: `check` already reported why.
pub fn gate(hir: &HirProgram, report: &ArenaReport, tcx: &TyCtx) -> Vec<Diagnostic> {
    let mut tcx = tcx.clone();
    let specialised = if hir.functions.iter().any(|f| f.name == "main") {
        crate::mono::monomorphize(hir, &mut tcx, report)
    } else {
        crate::mono::monomorphize_unchecked(hir, &mut tcx, report)
    };
    match specialised {
        Ok(mono) => gate_mono(&mono, &tcx),
        Err(_) => Vec::new(),
    }
}

/// [`gate`] over a program that is already monomorphized.
pub fn gate_mono(mono: &Mono, tcx: &TyCtx) -> Vec<Diagnostic> {
    match build_function_results(mono, tcx) {
        Ok(results) => gate_results(&results),
        Err(BuildError::Internal(message)) => gate_results(&[Err(BuildError::Internal(message))]),
        Err(_) => Vec::new(),
    }
}

/// The diagnostics of the functions in `results` that MIR could not build (the build path reuses the same
/// results for emission, so MIR is built once).
pub fn gate_results(results: &[Result<Body, BuildError>]) -> Vec<Diagnostic> {
    let mut diagnostics: Vec<Diagnostic> = Vec::new();
    for error in results.iter().filter_map(|result| result.as_ref().err()) {
        let (message, span) = match error {
            BuildError::Unsupported(u) => (u.message().to_owned(), Some(u.span)),
            BuildError::Unlowered { what, span } => (format!("{what} is not supported by codegen yet"), Some(*span)),
            BuildError::Internal(message) => (format!("internal error: MIR: {message}"), None),
        };
        // A generic body is built once per instance: report its mistake once.
        if !diagnostics.iter().any(|d| d.message == message && d.span == span) {
            diagnostics.push(Diagnostic { phase: Phase::Codegen, severity: Severity::Error, message, span, notes: Vec::new() });
        }
    }
    diagnostics
}

#[cfg(test)]
mod tests {
    use crate::diag::Phase;

    #[test]
    fn gate_rejects_trailing_closure() {
        let result = crate::frontend::check(crate::MVP_SAMPLE);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);

        assert!(diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.phase == Phase::Codegen && diagnostic.span.is_some() }));
    }

    #[test]
    fn methods_pass_codegen_gate() {
        let source = "class P {\n    x: i32\n    fun get(): i32 {\n        return x\n    }\n}\nfun main(): i32 {\n    val p = P(1)\n    return p.get()\n}";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        assert!(super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx).is_empty());
    }

    #[test]
    fn nullable_sample_passes_codegen_gate() {
        let source = r#"
fun main(name: String?): String {
    val empty: String? = None
    val wrapped: String? = Some("x")
    val asserted: String = wrapped!!
    return name ?: asserted
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);

        assert!(
            diagnostics.is_empty(),
            "nullable constructs should pass codegen gate: {diagnostics:?}"
        );
    }

    #[test]
    fn length_field_passes_codegen_gate() {
        let source = r#"fun main(): i32 {
    val a = [1, 2, 3]
    return a.length
}"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);

        assert!(
            diagnostics.is_empty(),
            "`.length` on arrays should be allowed at the codegen gate: {diagnostics:?}"
        );
    }

    #[test]
    fn record_and_presence_pass_codegen_gate() {
        let source = r#"
class Box { value: i32 }
fun main(): i32 {
    val item = Box(7)
    val reference: Ref<Box> = item
    if val live = reference { return live.value }
    return item.value
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.is_empty(),
            "records and if-val should lower: {diagnostics:?}"
        );
    }

    #[test]
    fn moving_an_array_of_ref_is_rejected_by_the_codegen_gate() {
        let source = r#"
class Box { value: i32 }
fun main() {
    val item = Box(1)
    val first: Ref<Box> = item
    val refs = [first, None]
    val _taken = move refs
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("array of managed Ref")),
            "{diagnostics:?}"
        );
    }

    #[test]
    fn float_optional_passes_codegen_gate() {
        let source = "fun main(): i32 {\n    val n: f32? = None\n    return 0\n}\n";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);

        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.is_empty(),
            "f32? is a scalar T? and must lower: {diagnostics:?}"
        );
    }

    #[test]
    fn managed_ref_safe_field_passes_codegen_gate() {
        let source = r#"
class Point { x: i32 }
fun main(): i32 {
    val point = Point(1)
    val p: Ref<Point> = point
    return p?.x ?: 0
}
"#;
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);
        assert!(
            diagnostics.is_empty(),
            "Ref<Point> `?.x` must pass codegen gate: {diagnostics:?}"
        );
    }

    #[test]
    fn a_mistake_in_a_generic_body_is_reported_once_per_site() {
        let source = "class P { x: i32 }\nfun id<T>(v: T, k: i32): i32 {\n    val p = P(k)\n    val q = move p\n    return q.x\n}\nfun main(): i32 {\n    return id(1, 2) + id(\"s\", 3)\n}\n";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);
        assert_eq!(diagnostics.len(), 1, "{diagnostics:?}");
    }

    #[test]
    fn an_unused_helper_that_cannot_be_specialised_does_not_hide_a_gate_error() {
        let source = "class P { x: i32 }\nfun g<T>(): i32 { return 1 }\nfun unused(): i32 { return g::<i32>() }\nfun main(): i32 {\n    val p = P(1)\n    val q = move p\n    return q.x\n}\n";
        let result = crate::frontend::check(source);
        assert!(result.is_ok(), "{:?}", result.diagnostics);
        let diagnostics = super::gate(result.hir.as_ref().unwrap(), result.report.as_ref().unwrap(), &result.tcx);
        assert!(diagnostics.iter().any(|d| d.message.contains("moving class objects")), "{diagnostics:?}");
    }
}
