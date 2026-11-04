//! MIR data types (RFC 0006 3.1, 3.2, 3.2.1, 3.2.2). Plain data: no behaviour beyond small walkers.

use crate::ast::{BinOp, PatLit};
use crate::hir::{NodeId, VecOp};
use crate::region_walk::RegionSite;
use crate::span::Span;
use crate::ty::TyId;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Local(pub u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RegionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HiddenKind {
    CallerParent,
    ResultSink,
    /// Holds the arena token of one region; `RegionReset` reassigns it (3.2.2 item 4).
    RegionArena,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalKind {
    Arg,
    Var,
    Val,
    Temp,
    Return,
    Hidden(HiddenKind),
}

#[derive(Debug, Clone, PartialEq)]
pub struct LocalDecl {
    pub ty: TyId,
    pub kind: LocalKind,
    pub name: Option<String>,
}

/// What the runtime registry needs to name an allocation, copy or slot (3.2.2 item 7).
#[derive(Debug, Clone, PartialEq)]
pub struct DebugLabel {
    pub name: String,
    pub ty: TyId,
    pub span: Span,
    pub source: Option<&'static str>,
    pub declaration: bool,
    /// Parameter position, for DWARF `arg` variables.
    pub argument: Option<u32>,
    /// Slot labels: the owning arena and, for array elements, the runtime index.
    pub owner: Option<ArenaRef>,
    pub index: Option<Operand>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Proj {
    Field(u32),
    Index(Box<Operand>),
    Deref,
    Variant(u32),
    Some,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub local: Local,
    pub proj: Vec<Proj>,
}

impl Place {
    pub fn local(local: Local) -> Place {
        Place { local, proj: Vec::new() }
    }
    pub fn is_bare(&self) -> bool {
        self.proj.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Unit,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    /// A view passed on (RFC 3.10).
    Reborrow(Place),
    Const(Const),
}

/// The arena an allocating op names explicitly (INV-MIR-2).
#[derive(Debug, Clone, PartialEq)]
pub enum ArenaRef {
    Current,
    Region(RegionId),
    Home(Local),
    ResultSink,
    CallerParent,
    ObjectArena(Local),
    Fresh(u32),
    Root,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AllocKind {
    Object { class: String },
    EnumSlot { enum_name: String, variant: u32 },
    String,
    Array,
    /// A `Vec` header (and, with `cap`, its first buffer) owned by the arena.
    Vec { cap: Option<Operand> },
}

#[derive(Debug, Clone, PartialEq)]
pub enum DropGlue {
    RefSlot,
    Enum(String),
    Vec(TyId),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirUnOp {
    Not,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AggKind {
    Object(String),
    Variant { enum_name: String, index: u32 },
    Array,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Rvalue {
    Use(Operand),
    Binary(BinOp, Operand, Operand),
    Unary(MirUnOp, Operand),
    Cast(Operand, TyId),
    Aggregate(AggKind, Vec<Operand>),
    Len(Place),
    Slice { base: Operand, lo: Operand, hi: Operand },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntrinsicOp {
    PrintInt,
    PrintlnInt,
    PrintStr,
    PrintlnStr,
    StringEq,
    Utf8Valid,
    /// `stringFromUtf8(bytes, start, count)`: a `String?` allocated in `arena`.
    StringFromUtf8,
    Concat,
    StrongCount,
    WeakCount,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RefOp {
    Create { dst: Place, source: ArenaRef, target: ArenaRef, object: Operand },
    Clone { dst: Place, src: Operand },
    MoveOut { dst: Place, src: Place },
    Store { slot: Place, owner: ArenaRef, value: Operand },
    Rehome { dst: Place, owner: ArenaRef, value: Operand },
    Drop(Operand),
    Eq { dst: Place, lhs: Operand, rhs: Operand },
    /// Consumes `value` (3.2.2 item 2).
    Count { dst: Place, value: Operand, weak: bool },
    IsNone { dst: Place, value: Operand },
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Assign(Place, Rvalue),
    Alloc { dst: Place, kind: AllocKind, arena: ArenaRef },
    ArenaCreateDynamic { dst: Local, parent: ArenaRef, scoped: bool },
    RegisterStrongRoot { owner: ArenaRef, arena: Local },
    CopyInto { dst: Place, src: Place, arena: ArenaRef },
    Borrow { dst: Place, src: Place, rw: bool },
    Move { dst: Place, src: Place },
    Promote { dst: Place, src: Place, arena: ArenaRef },
    Ref(RefOp),
    Observe { obs: Local, object: Local, absent: Local, value: Operand },
    ObservationRegister { arena: ArenaRef, obs: Local },
    ObservationRelease(Local),
    RegisterDrop { arena: ArenaRef, place: Place, glue: DropGlue },
    RegionEnter(RegionId),
    RegionExit(RegionId),
    RegionReset { region: RegionId, dst: Local },
    VecOp { dst: Option<Place>, op: VecOp, receiver: Operand, args: Vec<Operand> },
    /// `arena` is the allocation arena of ops that return fresh bytes (`concat`, `stringFromUtf8`).
    Intrinsic { dst: Option<Place>, op: IntrinsicOp, args: Vec<Operand>, arena: Option<ArenaRef> },
    StorageLive(Local),
    StorageDead(Local),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub span: Span,
    pub node: Option<NodeId>,
    pub kind: StmtKind,
    pub debug: Option<DebugLabel>,
}

/// `MirPat` mirrors `HirPatKind` (3.2.1).
#[derive(Debug, Clone, PartialEq)]
pub enum MirPat {
    Wildcard,
    Bind,
    Lit(PatLit),
    Variant { enum_name: String, index: u32, fields: Vec<MirPat> },
    Some(Box<MirPat>),
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MirArm {
    pub pat: MirPat,
    /// Borrows of the matched value; the arm never owns the payload.
    pub bindings: Vec<(Local, Place)>,
    pub target: BlockId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortReason {
    Binop,
    Bounds,
    NegativeCapacity,
    Utf8Range,
    NullUnwrap,
    MatchFallthrough,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TermKind {
    Goto(BlockId),
    Switch { value: Operand, cases: Vec<(i64, BlockId)>, otherwise: BlockId },
    /// Arms tried in source order; no match is `Unreachable` (typeck proved exhaustiveness).
    Match { scrutinee: Place, arms: Vec<MirArm> },
    Call {
        callee: String,
        args: Vec<Operand>,
        /// Hidden arena arguments the callee's `CallAbi` asks for.
        hidden: Vec<ArenaRef>,
        dest: Place,
        target: Option<BlockId>,
        /// Indexes of `args` the result borrows from (#202 forward compatibility).
        ret_borrows: Vec<u32>,
    },
    Return,
    Unreachable,
    Abort(AbortReason),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Terminator {
    pub span: Span,
    pub node: Option<NodeId>,
    pub kind: TermKind,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Block {
    pub stmts: Vec<Statement>,
    /// `None` only while a body is being built; the verifier rejects it.
    pub term: Option<Terminator>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RegionInfo {
    pub parent: Option<RegionId>,
    pub site: RegionSite,
}

/// Hidden parameters the function takes (`call_abi.rs` today).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CallAbi {
    pub caller_parent: bool,
    pub result_sink: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Body {
    /// Mono instance name (`id<i32>` or a plain function name).
    pub name: String,
    pub locals: Vec<LocalDecl>,
    pub blocks: Vec<Block>,
    pub regions: Vec<RegionInfo>,
    pub abi: CallAbi,
    /// The `Return` local.
    pub ret: Local,
}

impl Body {
    pub fn local(&self, local: Local) -> Option<&LocalDecl> {
        self.locals.get(local.0 as usize)
    }
}

/// One thing a statement or terminator touches, for the verifier and printers.
pub enum Use<'a> {
    /// Writes to a place.
    Def(&'a Place),
    /// Reads a place without consuming it.
    Read(&'a Place),
    /// Reads a place and takes ownership of it (`Move`, `RefMoveOut`).
    Consume(&'a Place),
    Op(&'a Operand),
    Arena(&'a ArenaRef),
    /// A bare local written or read directly (dynamic arenas, observations, region tokens).
    Local(Local),
    Region(RegionId),
}

fn rvalue_uses<'a>(r: &'a Rvalue, f: &mut dyn FnMut(Use<'a>)) {
    match r {
        Rvalue::Use(a) | Rvalue::Unary(_, a) | Rvalue::Cast(a, _) => f(Use::Op(a)),
        Rvalue::Binary(_, a, b) => {
            f(Use::Op(a));
            f(Use::Op(b));
        }
        Rvalue::Aggregate(_, ops) => ops.iter().for_each(|o| f(Use::Op(o))),
        Rvalue::Len(p) => f(Use::Read(p)),
        Rvalue::Slice { base, lo, hi } => {
            f(Use::Op(base));
            f(Use::Op(lo));
            f(Use::Op(hi));
        }
    }
}

impl StmtKind {
    /// Operands and sources are reported before destinations.
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(Use<'a>)) {
        match self {
            StmtKind::Assign(dst, r) => {
                rvalue_uses(r, f);
                f(Use::Def(dst));
            }
            StmtKind::Alloc { dst, arena, kind } => {
                if let AllocKind::Vec { cap: Some(cap) } = kind {
                    f(Use::Op(cap));
                }
                f(Use::Arena(arena));
                f(Use::Def(dst));
            }
            StmtKind::ArenaCreateDynamic { dst, parent, .. } => {
                f(Use::Arena(parent));
                f(Use::Local(*dst));
            }
            StmtKind::RegisterStrongRoot { owner, arena } => {
                f(Use::Arena(owner));
                f(Use::Local(*arena));
            }
            StmtKind::CopyInto { dst, src, arena } | StmtKind::Promote { dst, src, arena } => {
                f(Use::Arena(arena));
                f(Use::Read(src));
                f(Use::Def(dst));
            }
            StmtKind::Borrow { dst, src, .. } => {
                f(Use::Read(src));
                f(Use::Def(dst));
            }
            StmtKind::Move { dst, src } => {
                f(Use::Consume(src));
                f(Use::Def(dst));
            }
            StmtKind::Ref(op) => match op {
                RefOp::Create { dst, source, target, object } => {
                    f(Use::Arena(source));
                    f(Use::Arena(target));
                    f(Use::Op(object));
                    f(Use::Def(dst));
                }
                RefOp::Clone { dst, src } => {
                    f(Use::Op(src));
                    f(Use::Def(dst));
                }
                RefOp::MoveOut { dst, src } => {
                    f(Use::Consume(src));
                    f(Use::Def(dst));
                }
                RefOp::Store { slot, owner, value } => {
                    f(Use::Arena(owner));
                    f(Use::Op(value));
                    f(Use::Def(slot));
                }
                RefOp::Rehome { dst, owner, value } => {
                    f(Use::Arena(owner));
                    f(Use::Op(value));
                    f(Use::Def(dst));
                }
                RefOp::Drop(v) => f(Use::Op(v)),
                RefOp::Eq { dst, lhs, rhs } => {
                    f(Use::Op(lhs));
                    f(Use::Op(rhs));
                    f(Use::Def(dst));
                }
                RefOp::Count { dst, value, .. } | RefOp::IsNone { dst, value } => {
                    f(Use::Op(value));
                    f(Use::Def(dst));
                }
            },
            StmtKind::Observe { obs, object, absent, value } => {
                f(Use::Op(value));
                f(Use::Local(*obs));
                f(Use::Local(*object));
                f(Use::Local(*absent));
            }
            StmtKind::ObservationRegister { arena, obs } => {
                f(Use::Arena(arena));
                f(Use::Local(*obs));
            }
            StmtKind::ObservationRelease(l) | StmtKind::StorageLive(l) | StmtKind::StorageDead(l) => {
                f(Use::Local(*l))
            }
            StmtKind::RegisterDrop { arena, place, .. } => {
                f(Use::Arena(arena));
                f(Use::Read(place));
            }
            StmtKind::RegionEnter(r) | StmtKind::RegionExit(r) => f(Use::Region(*r)),
            StmtKind::RegionReset { region, dst } => {
                f(Use::Region(*region));
                f(Use::Local(*dst));
            }
            StmtKind::VecOp { dst, receiver, args, .. } => {
                f(Use::Op(receiver));
                args.iter().for_each(|a| f(Use::Op(a)));
                if let Some(d) = dst {
                    f(Use::Def(d));
                }
            }
            StmtKind::Intrinsic { dst, args, arena, .. } => {
                args.iter().for_each(|a| f(Use::Op(a)));
                if let Some(a) = arena {
                    f(Use::Arena(a));
                }
                if let Some(d) = dst {
                    f(Use::Def(d));
                }
            }
        }
    }
}

impl TermKind {
    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(Use<'a>)) {
        match self {
            TermKind::Switch { value, .. } => f(Use::Op(value)),
            TermKind::Match { scrutinee, arms } => {
                f(Use::Read(scrutinee));
                for arm in arms {
                    for (bound, place) in &arm.bindings {
                        f(Use::Local(*bound));
                        f(Use::Read(place));
                    }
                }
            }
            TermKind::Call { args, hidden, dest, .. } => {
                args.iter().for_each(|a| f(Use::Op(a)));
                hidden.iter().for_each(|a| f(Use::Arena(a)));
                f(Use::Def(dest));
            }
            TermKind::Goto(_) | TermKind::Return | TermKind::Unreachable | TermKind::Abort(_) => {}
        }
    }

    pub fn successors(&self) -> Vec<BlockId> {
        match self {
            TermKind::Goto(b) => vec![*b],
            TermKind::Switch { cases, otherwise, .. } => {
                cases.iter().map(|(_, b)| *b).chain([*otherwise]).collect()
            }
            TermKind::Match { arms, .. } => arms.iter().map(|a| a.target).collect(),
            TermKind::Call { target, .. } => target.iter().copied().collect(),
            TermKind::Return | TermKind::Unreachable | TermKind::Abort(_) => Vec::new(),
        }
    }
}
