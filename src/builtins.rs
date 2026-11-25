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
    SlotLoad,
    SlotStore,
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
        "slotLoad" => Some(Builtin::SlotLoad),
        "slotStore" => Some(Builtin::SlotStore),
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
        Some(Builtin::BufAlloc | Builtin::SlotLoad | Builtin::SlotStore | Builtin::BufGrow | Builtin::Abort | Builtin::ArenaOf)
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
        Builtin::SlotLoad => "`slotLoad<T>(b: &Buf<T>, i: i32): T`\nReads slot `i`; aborts outside `0..cap`. `unsafe` only.",
        Builtin::SlotStore => "`slotStore<T>(b: &Buf<T>, i: i32, v: T)`\nWrites slot `i`; aborts outside `0..cap`. `unsafe` only.",
        Builtin::BufGrow => {
            "`bufGrow<T>(b: &Buf<T>, newCap: i32): Buf<T>`\nA bigger copy in the same arena; aborts unless `newCap > cap`. `unsafe` only."
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
            Builtin::SlotLoad => "slotLoad",
            Builtin::SlotStore => "slotStore",
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
            Builtin::BufAlloc | Builtin::SlotLoad | Builtin::SlotStore | Builtin::BufGrow | Builtin::Abort | Builtin::ArenaOf => {
                (name, Vec::new(), tcx.unit())
            }
            Builtin::Concat => (name, vec![tcx.string(false), tcx.string(false)], tcx.string(false)),
        }
    }
}
