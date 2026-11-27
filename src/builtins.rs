use crate::ty::{FnSig, TyCtx, TyData, TyId};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Builtin {
    Print,
    Println,
    Concat,
    StringFromUtf8,
    StrongCount,
    WeakCount,
    // Buffer primitives (#285): typeck accepts them only inside `unsafe`, so only the prelude reaches them.
    BufAlloc,
    BufEmpty,
    SlotLoad,
    SlotStore,
    SlotDrop,
    /// Lowered to `IntrinsicOp::SlotStore` in `mir/build/buf.rs`; the only difference is the closure-environment copy done there.
    SlotCopy,
    SlotTake,
    RefNone,
    BufGrow,
    Abort,
    ArenaOf,
}

pub(crate) fn resolve(name: &str) -> Option<Builtin> {
    match name {
        "print" => Some(Builtin::Print),
        "println" => Some(Builtin::Println),
        "concat" => Some(Builtin::Concat),
        "stringFromUtf8" => Some(Builtin::StringFromUtf8),
        "strongCount" => Some(Builtin::StrongCount),
        "weakCount" => Some(Builtin::WeakCount),
        "bufAlloc" => Some(Builtin::BufAlloc),
        "bufEmpty" => Some(Builtin::BufEmpty),
        "slotLoad" => Some(Builtin::SlotLoad),
        "slotStore" => Some(Builtin::SlotStore),
        "slotDrop" => Some(Builtin::SlotDrop),
        "slotCopy" => Some(Builtin::SlotCopy),
        "slotTake" => Some(Builtin::SlotTake),
        "refNone" => Some(Builtin::RefNone),
        "bufGrow" => Some(Builtin::BufGrow),
        "abort" => Some(Builtin::Abort),
        "arenaOf" => Some(Builtin::ArenaOf),
        _ => None,
    }
}

pub(crate) fn is_print(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::Print | Builtin::Println))
}

pub(crate) fn is_concat(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::Concat))
}

/// Builtins whose result bytes are a new allocation in the consumer's sink (`concat`, `stringFromUtf8`).
pub(crate) fn is_string_alloc(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::Concat | Builtin::StringFromUtf8))
}

pub(crate) fn is_borrowed_arg(name: &str, index: usize) -> bool {
    index == 0
        && matches!(
            resolve(name),
            Some(Builtin::StringFromUtf8 | Builtin::StrongCount | Builtin::WeakCount)
        )
}

/// The unsafe-only buffer primitives (#285): typeck checks them in `check_buf_primitive`.
pub(crate) fn is_buf_primitive(name: &str) -> bool {
    matches!(
        resolve(name),
        Some(Builtin::BufAlloc | Builtin::BufEmpty | Builtin::SlotLoad | Builtin::SlotStore | Builtin::SlotDrop | Builtin::SlotCopy | Builtin::SlotTake | Builtin::RefNone | Builtin::BufGrow | Builtin::Abort | Builtin::ArenaOf)
    )
}

/// `strongCount(ref)` / `weakCount(ref)`: diagnostic counters on the arena a `Ref<T>` points into.
pub(crate) fn is_ref_count(name: &str) -> bool {
    matches!(resolve(name), Some(Builtin::StrongCount | Builtin::WeakCount))
}

/// Signature and one-line meaning of a builtin, for LSP hover.
pub(crate) fn hover_doc(name: &str) -> Option<&'static str> {
    Some(match resolve(name)? {
        Builtin::Print => "`print(value: i32 | i64 | String)`\nWrites a value without a newline.",
        Builtin::Println => "`println(value: i32 | i64 | String)`\nWrites a value and a newline.",
        Builtin::Concat => "`concat(a: String, b: String): String`\nNew string allocated in the consumer's arena.",
        Builtin::StringFromUtf8 => {
            "`stringFromUtf8(&bytes: &[u8; N], start: i32, count: i32): String?`\nCopies a byte range; `None` if it is not valid UTF-8."
        }
        Builtin::StrongCount => {
            "`strongCount(r: Ref<T>): i32`\nStrong count of the arena `r` points into (shared by all its Refs). `None` reads as 0."
        }
        Builtin::WeakCount => {
            "`weakCount(r: Ref<T>): i32`\nWeak count of the arena `r` points into, including its child arenas. `None` reads as 0."
        }
        Builtin::BufAlloc => "`bufAlloc<T>(a: Arena, cap: i32): Buf<T>`\nA buffer of `cap` slots in arena `a`. `unsafe` only.",
        Builtin::BufEmpty => "`bufEmpty<T>(): Buf<T>`\nA zero-slot buffer that owns no memory: no arena, no allocation; `bufGrow` makes the first real one. `unsafe` only.",
        Builtin::SlotLoad => "`slotLoad<T>(b: &Buf<T>, i: i32): T`\nReads slot `i`; aborts outside `0..cap`. `unsafe` only.",
        Builtin::SlotStore => "`slotStore<T>(b: &Buf<T>, i: i32, v: T)`\nWrites slot `i`; aborts outside `0..cap`. `unsafe` only.",
        Builtin::SlotCopy => "`slotCopy<T>(b: &Buf<T>, i: i32, v: T)`\nLike `slotStore`, but a closure `v` first has its environment copied into the arena of `this` (a closure built at the call site is already there and uses `slotStore`). `unsafe` only.",
        Builtin::SlotTake => "`slotTake<T>(b: &Buf<T>, i: i32): T`\nMoves the value out of slot `i`: a `Ref` leaves `None` behind. Aborts outside `0..cap`. `unsafe` only.",
        Builtin::RefNone => "`refNone<T>(): T`\nThe `None` handle of a `Ref<C>` type `T`. `unsafe` only.",
        Builtin::SlotDrop => "`slotDrop<T>(b: &Buf<T>, i: i32)`\nReleases the `Ref` in slot `i` and leaves it `None`; does nothing for other slot types. Aborts outside `0..cap`. `unsafe` only.",
        Builtin::BufGrow => {
            "`bufGrow<T>(b: &Buf<T>, newCap: i32): Buf<T>`\nA bigger copy in the arena of `this` (`b` must have been allocated in `this`'s arena); aborts unless `newCap > cap`. Consumes `b`: the old storage is freed (kept until the arena dies for `Ref` elements), so `b` must not be used afterwards. `unsafe` only."
        }
        Builtin::Abort => "`abort()`\nStops the program. `unsafe` only.",
        Builtin::ArenaOf => "`arenaOf(this): Arena`\nThe arena the receiver lives in. `unsafe` only.",
    })
}

pub(crate) fn is_intrinsic(name: &str) -> bool {
    resolve(name).is_some()
}

pub(crate) fn supports_print_arg(tcx: &TyCtx, ty: TyId) -> bool {
    matches!(
        tcx.data(ty),
        TyData::Prim(crate::hir::Prim::I32 | crate::hir::Prim::I64) | TyData::String
    )
}

/// Builtins with a fixed signature, in declaration order.
const SIGNATURE_BUILTINS: [Builtin; 3] = [Builtin::Print, Builtin::Println, Builtin::Concat];

pub(crate) fn signature_names() -> [&'static str; 3] {
    SIGNATURE_BUILTINS.map(|builtin| builtin.name())
}

pub(crate) fn signatures(tcx: &mut TyCtx) -> [(&'static str, FnSig<TyId>); 3] {
    SIGNATURE_BUILTINS.map(|builtin| {
        let (name, params, return_ty) = builtin.sig(tcx);
        (name, FnSig::new(params, return_ty))
    })
}

impl Builtin {
    fn name(self) -> &'static str {
        match self {
            Builtin::Print => "print",
            Builtin::Println => "println",
            Builtin::Concat => "concat",
            Builtin::StringFromUtf8 => "stringFromUtf8",
            Builtin::StrongCount => "strongCount",
            Builtin::WeakCount => "weakCount",
            Builtin::BufAlloc => "bufAlloc",
            Builtin::BufEmpty => "bufEmpty",
            Builtin::SlotLoad => "slotLoad",
            Builtin::SlotStore => "slotStore",
            Builtin::SlotDrop => "slotDrop",
            Builtin::SlotCopy => "slotCopy",
            Builtin::SlotTake => "slotTake",
            Builtin::RefNone => "refNone",
            Builtin::BufGrow => "bufGrow",
            Builtin::Abort => "abort",
            Builtin::ArenaOf => "arenaOf",
        }
    }

    fn sig(self, tcx: &mut TyCtx) -> (&'static str, Vec<TyId>, TyId) {
        let name = self.name();
        match self {
            Builtin::Print | Builtin::Println => (name, vec![tcx.i32()], tcx.unit()),
            // Typeck checks `stringFromUtf8` itself: its buffer parameter is any `&[u8; N]`.
            Builtin::StringFromUtf8 => (name, Vec::new(), tcx.string(true)),
            // Typeck checks the counters itself: the argument is any `Ref<T>`.
            Builtin::StrongCount | Builtin::WeakCount => (name, Vec::new(), tcx.i32()),
            // Typeck checks the buffer primitives itself: they are generic over the slot type.
            Builtin::BufAlloc | Builtin::BufEmpty | Builtin::SlotLoad | Builtin::SlotStore | Builtin::SlotDrop | Builtin::SlotCopy | Builtin::SlotTake | Builtin::RefNone | Builtin::BufGrow | Builtin::Abort | Builtin::ArenaOf => {
                (name, Vec::new(), tcx.unit())
            }
            Builtin::Concat => (name, vec![tcx.string(false), tcx.string(false)], tcx.string(false)),
        }
    }
}
