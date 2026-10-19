//! The one type printer. Text matches `hir::Ty`'s `Display` exactly (`(a, b)->r` without spaces,
//! `?` as a plain suffix), so diagnostics read the same whichever representation they print.

use std::fmt;

use super::{TyCtx, TyData, TyId};

pub struct TyPrinter<'a> {
    ctx: &'a TyCtx,
    id: TyId,
}

impl TyCtx {
    pub fn display(&self, id: TyId) -> TyPrinter<'_> {
        TyPrinter { ctx: self, id }
    }
}

impl fmt::Display for TyPrinter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let show = |id| TyPrinter { ctx: self.ctx, id };
        match self.ctx.data(self.id) {
            TyData::Prim(prim) => write!(f, "{prim}"),
            TyData::String => f.write_str("String"),
            TyData::Class(def) => f.write_str(&self.ctx.class_names[def]),
            TyData::Array { elem, len } => write!(f, "[{}; {len}]", show(*elem)),
            TyData::Func { params, ret } => {
                f.write_str("(")?;
                for (i, param) in params.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{}", show(*param))?;
                }
                write!(f, ")->{}", show(*ret))
            }
            TyData::Ref(t) => write!(f, "&{}", show(*t)),
            TyData::ManagedRef(t) => write!(f, "Ref<{}>", show(*t)),
            TyData::Vec(t) => write!(f, "Vec<{}>", show(*t)),
            TyData::Range(t) => write!(f, "Range<{}>", show(*t)),
            TyData::Nullable(t) => write!(f, "{}?", show(*t)),
            // No source syntax yet; chosen so they cannot be mistaken for a written type.
            TyData::Param(idx) => write!(f, "T{}", idx.0),
            TyData::Infer(var) => write!(f, "?{}", var.0),
            TyData::Error => f.write_str("<unknown>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::hir::{Prim, Ty, TyKind};
    use crate::ty::tests::{ctx, named, samples};
    use crate::ty::{InferVar, ParamIdx};

    #[test]
    fn matches_hir_display_for_every_shape() {
        let mut c = ctx();
        let mut all = samples();
        all.push(Ty::new(TyKind::Func { params: vec![], ret: Box::new(Ty::i32()) }, true));
        for ty in all {
            let id = c.intern(&ty).unwrap();
            let want = if ty.kind == TyKind::Unknown { "<unknown>".to_string() } else { ty.to_string() };
            assert_eq!(c.display(id).to_string(), want);
        }
    }

    #[test]
    fn prints_the_forms_diagnostics_use() {
        let mut c = ctx();
        let mut show = |ty: Ty| {
            let id = c.intern(&ty).unwrap();
            c.display(id).to_string()
        };
        assert_eq!(show(Ty::new(TyKind::Prim(Prim::I32), true)), "i32?");
        assert_eq!(show(Ty::new(TyKind::ManagedRef(Box::new(named("Node"))), false)), "Ref<Node>");
        assert_eq!(show(Ty::new(TyKind::Func { params: vec![Ty::i32()], ret: Box::new(Ty::i32()) }, false)), "(i32)->i32");
        assert_eq!(show(Ty::new(TyKind::Func { params: vec![], ret: Box::new(Ty::i32()) }, true)), "()->i32?");
        assert_eq!(show(Ty::new(TyKind::Array { elem: Box::new(Ty::i32()), len: 3 }, false)), "[i32; 3]");
        let (p, v) = (c.param(ParamIdx(0)), c.infer(InferVar(1)));
        assert_eq!((c.display(p).to_string(), c.display(v).to_string()), ("T0".into(), "?1".into()));
    }
}
