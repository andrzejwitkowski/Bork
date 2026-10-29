//! The one type printer. Diagnostics print `(a, b)->r` without spaces and `?` as a plain suffix.

use std::fmt;

use super::{TyCtx, TyData, TyId};

pub struct TyPrinter<'a> {
    ctx: &'a TyCtx,
    id: TyId,
    /// Parameter names to print instead of the context's, e.g. an impl's own.
    names: Option<&'a [String]>,
}

impl TyCtx {
    pub fn display(&self, id: TyId) -> TyPrinter<'_> {
        TyPrinter { ctx: self, id, names: None }
    }

    /// Like [`TyCtx::display`], naming `Param(i)` as `names[i]`.
    pub fn display_with<'a>(&'a self, id: TyId, names: &'a [String]) -> TyPrinter<'a> {
        TyPrinter { ctx: self, id, names: Some(names) }
    }
}

impl fmt::Display for TyPrinter<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let show = |id| TyPrinter { ctx: self.ctx, id, names: self.names };
        match self.ctx.data(self.id) {
            TyData::Prim(prim) => write!(f, "{prim}"),
            TyData::String => f.write_str("String"),
            // `T?` is `Option<T>`; print it as written unless `T` could not be written with `?`.
            TyData::Enum(def, args)
                if Some(*def) == self.ctx.option_def() && args.len() == 1 && self.ctx.supports_nullable(args[0]) && !self.ctx.is_nullable(args[0]) =>
            {
                write!(f, "{}?", show(args[0]))
            }
            TyData::Class(def, args) | TyData::Enum(def, args) => {
                f.write_str(&self.ctx.class_names[def])?;
                if args.is_empty() {
                    return Ok(());
                }
                let shown: Vec<String> = args.iter().map(|&arg| show(arg).to_string()).collect();
                write!(f, "<{}>", shown.join(", "))
            }
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
            // No source syntax yet; chosen so they cannot be mistaken for a written type.
            TyData::Param(idx) => match self.names.unwrap_or(&self.ctx.param_names).get(idx.0 as usize) {
                Some(name) => f.write_str(name),
                None => write!(f, "T{}", idx.0),
            },
            TyData::Infer(var) => write!(f, "?{}", var.0),
            TyData::Error => f.write_str("<unknown>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::hir::Prim;
    use crate::ty::tests::{ctx, named, samples};
    use crate::ty::{InferVar, ParamIdx};

    #[test]
    fn prints_every_sample_without_panicking() {
        let mut c = ctx();
        for id in samples(&mut c) {
            assert!(!c.display(id).to_string().is_empty());
        }
    }

    #[test]
    fn prints_the_forms_diagnostics_use() {
        let mut c = ctx();
        let (i32_ty, node) = (c.i32(), named(&mut c, "Node"));
        let opt_i32 = c.with_nullable(i32_ty, true);
        let managed = c.managed_ref(node);
        let func = c.func(vec![i32_ty], i32_ty);
        let nullable_func = {
            let f = c.func(vec![], i32_ty);
            c.with_nullable(f, true)
        };
        let array = c.array(i32_ty, 3);
        let (range, vec_ty, string) = (c.range(i32_ty), c.vec_of(i32_ty), c.string(true));
        let reference = c.ref_to(node);
        let unknown = c.error();
        let _ = Prim::I32;
        let shown = |c: &crate::ty::TyCtx, id| c.display(id).to_string();
        assert_eq!(shown(&c, opt_i32), "i32?");
        assert_eq!(shown(&c, managed), "Ref<Node>");
        assert_eq!(shown(&c, func), "(i32)->i32");
        assert_eq!(shown(&c, nullable_func), "()->i32?");
        assert_eq!(shown(&c, array), "[i32; 3]");
        assert_eq!(shown(&c, range), "Range<i32>");
        assert_eq!(shown(&c, vec_ty), "Vec<i32>");
        assert_eq!(shown(&c, string), "String?");
        assert_eq!(shown(&c, reference), "&Node");
        assert_eq!(shown(&c, unknown), "<unknown>");
        let (p, v) = (c.param(ParamIdx(0)), c.infer(InferVar(1)));
        assert_eq!((shown(&c, p), shown(&c, v)), ("T0".into(), "?1".into()));
    }
}
