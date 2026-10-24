use super::{InferCtxt, TypeError, VarData, VarKind};
use crate::ty::{InferVar, TyCtx, TyData, TyId};

impl InferCtxt {
    /// Makes `a` and `b` the same type, or fails leaving the context exactly as it was.
    pub fn unify(&mut self, tcx: &mut TyCtx, a: TyId, b: TyId) -> Result<(), TypeError> {
        let snap = self.snapshot();
        match self.unify_inner(tcx, a, b) {
            Ok(()) => {
                self.commit(snap);
                Ok(())
            }
            Err(err) => {
                self.rollback_to(snap);
                Err(err)
            }
        }
    }

    fn unify_inner(&mut self, tcx: &mut TyCtx, a: TyId, b: TyId) -> Result<(), TypeError> {
        let a = self.resolve_shallow(tcx, a);
        let b = self.resolve_shallow(tcx, b);
        if a == b {
            return Ok(());
        }
        let mismatch = TypeError::Mismatch { expected: a, found: b };
        match (tcx.data(a).clone(), tcx.data(b).clone()) {
            (TyData::Infer(x), TyData::Infer(y)) => self.union(tcx, x, y),
            (TyData::Infer(x), _) => self.bind(tcx, x, b),
            (_, TyData::Infer(y)) => self.bind(tcx, y, a),
            // An error already has a diagnostic; unifying with it must not cascade.
            (TyData::Error, _) | (_, TyData::Error) => Ok(()),
            (TyData::Array { elem: ea, len: la }, TyData::Array { elem: eb, len: lb }) => {
                if la != lb {
                    return Err(mismatch);
                }
                self.unify_inner(tcx, ea, eb)
            }
            (TyData::Func { params: pa, ret: ra }, TyData::Func { params: pb, ret: rb }) => {
                if pa.len() != pb.len() {
                    return Err(TypeError::Arity { expected: pa.len(), found: pb.len() });
                }
                for (x, y) in pa.into_iter().zip(pb) {
                    self.unify_inner(tcx, x, y)?;
                }
                self.unify_inner(tcx, ra, rb)
            }
            (TyData::Class(da, xa), TyData::Class(db, xb)) | (TyData::Enum(da, xa), TyData::Enum(db, xb)) => {
                if da != db || xa.len() != xb.len() {
                    return Err(mismatch);
                }
                for (x, y) in xa.into_iter().zip(xb) {
                    self.unify_inner(tcx, x, y)?;
                }
                Ok(())
            }
            (TyData::Ref(x), TyData::Ref(y))
            | (TyData::ManagedRef(x), TyData::ManagedRef(y))
            | (TyData::Vec(x), TyData::Vec(y))
            | (TyData::Range(x), TyData::Range(y))
            | (TyData::Nullable(x), TyData::Nullable(y)) => self.unify_inner(tcx, x, y),
            _ => Err(mismatch),
        }
    }

    /// Joins two unbound variables into one class.
    fn union(&mut self, tcx: &mut TyCtx, x: InferVar, y: InferVar) -> Result<(), TypeError> {
        let (rx, ry) = (self.root(x.0), self.root(y.0));
        if rx == ry {
            return Ok(());
        }
        let (dx, dy) = (self.vars[rx as usize].clone(), self.vars[ry as usize].clone());
        let kind = match (dx.kind, dy.kind) {
            (VarKind::General, k) | (k, VarKind::General) => k,
            (a, b) if a == b => a,
            (_, kind) => return Err(TypeError::Kind { kind, ty: tcx.infer(InferVar(rx)) }),
        };
        let (winner, loser) = if dx.rank >= dy.rank { (rx, ry) } else { (ry, rx) };
        let rank = if dx.rank == dy.rank { dx.rank + 1 } else { dx.rank.max(dy.rank) };
        self.set(loser, VarData { parent: winner, ..self.vars[loser as usize].clone() });
        self.set(winner, VarData { parent: winner, rank, value: None, kind });
        Ok(())
    }

    /// Binds an unbound variable to a non-variable type.
    fn bind(&mut self, tcx: &mut TyCtx, var: InferVar, ty: TyId) -> Result<(), TypeError> {
        let root = self.root(var.0);
        let kind = self.vars[root as usize].kind;
        if !kind.admits(tcx, ty) && !tcx.is_error(ty) {
            return Err(TypeError::Kind { kind, ty });
        }
        if self.occurs(tcx, root, ty) {
            return Err(TypeError::Occurs { var: InferVar(root), ty });
        }
        let data = self.vars[root as usize].clone();
        self.set(root, VarData { value: Some(ty), ..data });
        Ok(())
    }

    /// Whether the class `root` appears anywhere inside `ty`, looking through bound variables.
    fn occurs(&self, tcx: &mut TyCtx, root: u32, ty: TyId) -> bool {
        let ty = self.resolve_shallow(tcx, ty);
        match tcx.data(ty).clone() {
            TyData::Infer(var) => self.root(var.0) == root,
            data => data.children().into_iter().any(|child| self.occurs(tcx, root, child)),
        }
    }
}
