//! Coherence (#57): two impls of one trait may not apply to the same type.
//!
//! Two impls overlap when their headers unify, each with fresh variables for its own parameters.
//! Bounds are ignored, so `impl<T: A> Show for Box<T>` and `impl<T: B> Show for Box<T>` overlap
//! even if no type has both. The later impl is reported and dropped, so the solver never sees an
//! overlap and uses of the first impl produce no second error.

use super::traits::{ImplInfo, ImplTable};
use super::type_error;
use crate::defs::DefTable;
use crate::diag::Diagnostic;
use crate::infer::InferCtxt;
use crate::ty::TyCtx;

pub(super) fn check_overlaps(
    table: &mut ImplTable,
    defs: &DefTable,
    tcx: &mut TyCtx,
    diagnostics: &mut Vec<Diagnostic>,
) {
    let mut found: Vec<(usize, Diagnostic)> = Vec::new();
    let mut infcx = InferCtxt::new();
    for (&trait_id, impls) in table.iter_mut() {
        let mut accepted: Vec<ImplInfo> = Vec::new();
        for info in impls.drain(..) {
            match accepted.iter().find(|earlier| overlap(&mut infcx, tcx, earlier, &info)) {
                Some(earlier) => {
                    let trait_name = &defs.get(trait_id).name;
                    let message = format!(
                        "conflicting implementations of trait `{trait_name}`: `{}` overlaps with `{}`; make the types differ or remove one",
                        header(tcx, defs, &info),
                        header(tcx, defs, earlier)
                    );
                    found.push((info.index, type_error(message, Some(info.span))));
                }
                None => accepted.push(info),
            }
        }
        *impls = accepted;
    }
    // The table is a map; source order keeps the output the same on every run.
    found.sort_by_key(|(index, _)| *index);
    diagnostics.extend(found.into_iter().map(|(_, diagnostic)| diagnostic));
}

fn overlap(infcx: &mut InferCtxt, tcx: &mut TyCtx, a: &ImplInfo, b: &ImplInfo) -> bool {
    let snap = infcx.snapshot();
    let unified = (|| {
        let (a_self, a_args, _) = a.instantiate(infcx, tcx)?;
        let (b_self, b_args, _) = b.instantiate(infcx, tcx)?;
        infcx.unify(tcx, a_self, b_self).ok()?;
        a_args.iter().zip(&b_args).all(|(&x, &y)| infcx.unify(tcx, x, y).is_ok()).then_some(())
    })();
    infcx.rollback_to(snap);
    unified.is_some()
}

/// `impl<T: Eq> Show for Box<T>`, with the impl's own parameter names and bounds.
fn header(tcx: &TyCtx, defs: &DefTable, info: &ImplInfo) -> String {
    let show = |ty| tcx.display_with(ty, &info.generics).to_string();
    let args: Vec<String> = info.trait_args.iter().map(|&a| show(a)).collect();
    let args = if args.is_empty() { String::new() } else { format!("<{}>", args.join(", ")) };
    let bounds: Vec<String> = info
        .predicates
        .iter()
        .map(|p| {
            let trait_args: Vec<String> = p.trait_ref.args.iter().map(|&a| show(a)).collect();
            let trait_args = if trait_args.is_empty() { String::new() } else { format!("<{}>", trait_args.join(", ")) };
            format!("{}: {}{trait_args}", show(p.self_ty), defs.get(p.trait_ref.trait_id).name)
        })
        .collect();
    let bounds = if bounds.is_empty() { String::new() } else { format!(" where {}", bounds.join(", ")) };
    format!("{}{args} for {}{bounds}", defs.get(info.trait_id).name, show(info.self_ty))
}
