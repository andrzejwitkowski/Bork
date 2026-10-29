//! Inference variables and unification (RFC 0001 section 5). Typeck uses one per function body
//! for untyped literal locals. Every method takes the `TyCtx` that issued the variables' ids.

mod error;
mod obligation;
mod unify;

#[cfg(test)]
mod tests;

pub use error::TypeError;
pub use obligation::Obligation;

use crate::hir::Prim;
use crate::ty::{InferVar, NestedNullable, TyCtx, TyData, TyId, TypeFolder};

/// What a variable may become. Literals are not nullable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    General,
    /// An integer literal: any integer primitive.
    Int,
    /// A float literal: `f32` or `f64`.
    Float,
}

impl VarKind {
    /// Whether a (non-variable) type is allowed for this kind.
    fn admits(self, tcx: &TyCtx, ty: TyId) -> bool {
        match self {
            VarKind::General => true,
            VarKind::Int => matches!(tcx.data(ty), TyData::Prim(p) if p.is_integer()),
            VarKind::Float => matches!(tcx.data(ty), TyData::Prim(Prim::F32 | Prim::F64)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VarData {
    /// Union-find parent; a root points at itself.
    parent: u32,
    rank: u8,
    /// Set on roots only, and never to a variable.
    value: Option<TyId>,
    kind: VarKind,
}

/// A point to roll back to. Taken with [`InferCtxt::snapshot`], ended by exactly one of
/// [`InferCtxt::rollback_to`] or [`InferCtxt::commit`], innermost first.
#[derive(Debug)]
pub struct Snapshot {
    vars_len: usize,
    log_len: usize,
    obligations_len: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InferCtxt {
    vars: Vec<VarData>,
    /// Previous contents of every variable slot changed while a snapshot is open.
    log: Vec<(u32, VarData)>,
    open: usize,
    /// Trait requirements registered by calls, not yet solved (#56). The solver is in `typeck`.
    obligations: Vec<Obligation>,
}

impl InferCtxt {
    pub fn new() -> InferCtxt {
        InferCtxt::default()
    }

    /// A fresh unbound variable, as a type.
    pub fn new_var(&mut self, tcx: &mut TyCtx, kind: VarKind) -> TyId {
        let index = self.vars.len() as u32;
        self.vars.push(VarData { parent: index, rank: 0, value: None, kind });
        tcx.infer(InferVar(index))
    }

    pub fn snapshot(&mut self) -> Snapshot {
        self.open += 1;
        Snapshot { vars_len: self.vars.len(), log_len: self.log.len(), obligations_len: self.obligations.len() }
    }

    /// Undoes everything since `snap`, including variables created since.
    pub fn rollback_to(&mut self, snap: Snapshot) {
        while self.log.len() > snap.log_len {
            let (index, old) = self.log.pop().expect("log is longer than the snapshot");
            self.vars[index as usize] = old;
        }
        self.vars.truncate(snap.vars_len);
        self.obligations.truncate(snap.obligations_len);
        self.end_snapshot();
    }

    /// Keeps everything since `snap`.
    pub fn commit(&mut self, _snap: Snapshot) {
        self.end_snapshot();
    }

    fn end_snapshot(&mut self) {
        self.open -= 1;
        if self.open == 0 {
            self.log.clear();
        }
    }

    fn set(&mut self, index: u32, data: VarData) {
        if self.open > 0 {
            self.log.push((index, self.vars[index as usize].clone()));
        }
        self.vars[index as usize] = data;
    }

    fn root(&self, mut index: u32) -> u32 {
        while self.vars[index as usize].parent != index {
            index = self.vars[index as usize].parent;
        }
        index
    }

    /// The kind of a variable (of its whole equivalence class).
    pub fn kind_of(&self, var: InferVar) -> VarKind {
        self.vars[self.root(var.0) as usize].kind
    }

    /// Binds every unbound literal variable to its default (`i32`, `f64`). General variables stay.
    pub fn default_literals(&mut self, tcx: &mut TyCtx) {
        for index in 0..self.vars.len() as u32 {
            let root = self.root(index);
            let data = &self.vars[root as usize];
            let default = match (data.value, data.kind) {
                (None, VarKind::Int) => Prim::I32,
                (None, VarKind::Float) => Prim::F64,
                _ => continue,
            };
            let mut data = data.clone();
            data.value = Some(tcx.prim_id(default));
            self.set(root, data);
        }
    }

    /// What the variable's class is bound to, if anything (read-only `resolve_shallow` step).
    pub fn value_of(&self, var: InferVar) -> Option<TyId> {
        self.vars[self.root(var.0) as usize].value
    }

    /// Follows variable links until a non-variable type or an unbound variable (as its class
    /// representative). Does not look inside the type.
    pub fn resolve_shallow(&self, tcx: &mut TyCtx, mut ty: TyId) -> TyId {
        loop {
            let TyData::Infer(var) = *tcx.data(ty) else {
                return ty;
            };
            let root = self.root(var.0);
            match self.vars[root as usize].value {
                Some(value) => ty = value,
                None => return tcx.infer(InferVar(root)),
            }
        }
    }

    /// Replaces every bound variable inside `ty`. Unbound variables stay (as their class
    /// representative). Fails only if a substitution would nest nullables (`T??`).
    pub fn resolve(&self, tcx: &mut TyCtx, ty: TyId) -> Result<TyId, TypeError> {
        Resolver { infcx: self }.fold(tcx, ty).map_err(|NestedNullable| TypeError::NestedNullable)
    }
}

struct Resolver<'a> {
    infcx: &'a InferCtxt,
}

impl TypeFolder for Resolver<'_> {
    fn fold(&mut self, tcx: &mut TyCtx, ty: TyId) -> Result<TyId, NestedNullable> {
        if matches!(tcx.data(ty), TyData::Infer(_)) {
            let shallow = self.infcx.resolve_shallow(tcx, ty);
            if matches!(tcx.data(shallow), TyData::Infer(_)) {
                return Ok(shallow);
            }
            return self.fold(tcx, shallow);
        }
        crate::ty::super_fold(self, tcx, ty)
    }
}
