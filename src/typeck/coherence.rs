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
use crate::ty::{TraitRef, TyCtx};

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
                        "conflicting implementations of trait `{trait_name}`: `{}` overlaps with the earlier impl `{}`; make the types differ or remove one",
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

/// `Show for Box<T> where T: Eq`, with the impl's own parameter names and bounds.
fn header(tcx: &TyCtx, defs: &DefTable, info: &ImplInfo) -> String {
    let show = |ty| tcx.display_with(ty, &info.generics).to_string();
    let trait_ref = TraitRef { trait_id: info.trait_id, args: info.trait_args.clone() };
    let bounds: Vec<String> = info.predicates.iter().map(|p| p.render(defs, show)).collect();
    let bounds = if bounds.is_empty() { String::new() } else { format!(" where {}", bounds.join(", ")) };
    format!("{} for {}{bounds}", trait_ref.render(defs, show), show(info.self_ty))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ty::InferVar;

    #[test]
    fn checking_a_pair_leaves_no_inference_variables_behind() {
        let src = "trait S {\n}\nclass Box<T> {\n    v: T\n}\nclass A {\n    r: f64\n}\nimpl<T> S for Box<T> {\n}\nimpl S for Box<A> {\n}\nfun main() {}\n";
        let program = crate::parse(src).unwrap();
        let defs = DefTable::build(&program);
        let mut tcx = TyCtx::new(&defs);
        let mut diagnostics = Vec::new();
        let traits = super::super::traits::build_traits(&program, &defs, &mut tcx, &mut diagnostics);
        let table = super::super::traits::build_impls(&program, &defs, &traits, &mut tcx, &mut diagnostics);
        let impls = &table[&defs.trait_id(0)];
        let mut infcx = InferCtxt::new();
        assert!(overlap(&mut infcx, &mut tcx, &impls[0], &impls[1]));
        let next = infcx.new_var(&mut tcx, crate::infer::VarKind::General);
        assert_eq!(*tcx.data(next), crate::ty::TyData::Infer(InferVar(0)));
    }
}
