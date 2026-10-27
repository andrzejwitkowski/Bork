# Debugging Bork programs

Build with `bork build -g program.bork` to emit native DWARF, disable LLVM optimization,
and include the debug registry. Normal builds do not contain the registry. Stop at a
source breakpoint before inspecting arenas; the tools never call target functions.

With `-g`, DebugDiag also checks arena memory before generated loads, stores and
memory intrinsics, including enum cleanup functions. Copies check both the source
and destination. Runtime Ref slots, observations, string operations, printing and
`Vec<Ref<T>>` cleanup are checked before they dereference payload memory. Empty
accesses do not read payload bytes; stack memory and program constants pass through.

An invalid access prints a report to stderr and aborts. It includes the operation,
address and byte count, allocation/arena IDs, memory state (`Live`, `Freed`, or
`UnusedInPool`), variable or field name, concrete type, creation/declaration/release
locations with original byte spans, Bork call stack, and retained arena ancestry.
Declarations from the prelude refer to their own source files. Replacement allocations
retain the original variable or field declaration location.
Cleanup can access live payloads; allocations become unused after callbacks finish,
with the release location captured before cleanup begins. Promotion and buffer
growth preserve the original provenance while ownership follows the actual allocator
token, including caller result sinks and dynamic Ref arenas.

Taking an arena from the pool does not revive its old allocations. Reuse makes only
the new allocation's bytes live and retains one previous occupancy description,
labelled as history. Arena IDs in class, enum, Option and Ref handles detect stale
handles even when another arena reuses their address. A raw pointer into bytes that
have already been reused can still look valid; DebugDiag does not change pointer
representations, quarantine memory or retain a complete execution history. Metadata
lives outside payload memory and the existing debugger ABI, and increases debug
execution time and memory use. Ordinary builds contain none of these new hooks.

Load the Python tools manually, using your checkout's absolute path:

```text
(gdb) source /path/to/Bork/tools/bork-debug/bork_gdb.py
(gdb) break program.bork:12
(gdb) run
(gdb) print local_ref
(gdb) print this
(gdb) bork-arenas
(gdb) bork-graph /tmp/arenas.dot --svg /tmp/arenas.svg
```

```text
(lldb) command script import /path/to/Bork/tools/bork-debug/bork_lldb.py
(lldb) breakpoint set --file program.bork --line 12
(lldb) breakpoint set --basename Point.sum
(lldb) run
(lldb) frame variable local_ref this
(lldb) bork-arenas
(lldb) bork-graph /tmp/arenas.dot --svg /tmp/arenas.svg
```

The adapters provide summaries and expandable children for `bork::Ref<T>` and
`bork::Object<T>` (including method `this`). Live typed object pointers within the arena allocation can expose
payload fields when the complete target type fits within the used bytes. None, stale IDs, expired Weak references, and non-Alive arenas
never cause payload dereferences. Object addresses are still shown for diagnosis.
A missing registry means the executable was not built with `-g`, or symbols are
unavailable. The tools require debugger Python support and use only Python's
standard library; no auto-load policy changes are needed.

`bork-arenas` shows arena IDs, parents, state, mode, runtime Strong/Weak/scoped
counts, allocation usage, and source locations. Registry entries can include dead
controls that still have Weak handles. Reset arenas have new IDs.

`bork-graph <file.dot> [--svg file.svg]` saves a snapshot of all registered persistent
Ref slots, including named locals, object fields and registered array elements.
Quote paths containing spaces. DOT has dashed parent edges, dotted slot ownership
edges, bold Strong edges, dashed Weak edges, and solid Arena/Borrow edges. Object nodes belong to their target arena. None and
expired slots remain visible. Labels include source names, indices and available
type names. Runtime counts and observed Ref-edge counts are reported separately:
registered slots are not necessarily every temporary handle contributing to a
runtime counter. Only actual Ref targets produce object nodes. Payload bytes are never needed to export a graph.

SVG export requires Graphviz's `dot` on PATH. It is launched without a shell; the
DOT file is preserved when Graphviz is missing or fails. DOT export itself has no
external dependency. Graphs contain variable names and addresses from the process.

The v1 reader supports 32/64-bit targets and their reported endianness, checks ABI
version, stable even mutation sequence, memory-read results, record counts and
string lengths. Inspection during a registry mutation produces an error: stop at
a stable source location and retry. V1 supports one Bork execution thread; it does
not merge independent thread-local ID spaces. The graph is a snapshot, with no
history or debugger GUI.

Run the decoder/export tests and native GDB/LLDB checks with:

```sh
DEBUGINFOD_URLS= python3 -m unittest discover -s tools/bork-debug -v
```

Native checks are skipped if their debugger/compiler is unavailable. Set `BORK_LLDB`
to use an LLDB binary outside PATH. Clearing `DEBUGINFOD_URLS` prevents these small
fixtures from waiting for external debug-symbol downloads.

## Registry ABI v1

The exported `bork_debug_registry` symbol directly contains this header. Every
field below occupies one target `usize` word, including pointers (4 or 8 bytes,
with target endianness). Records are packed as consecutive words with target
pointer alignment; no target function is evaluated to find or read them.

```text
Registry: version, sequence, arenas*, arena_count, slots*, slot_count
Arena:    id, parent, depth, strong_count, weak_count, live_scoped_children,
          state, mode, payload*, used, capacity, source*, line
Slot:     owner, address*, label*, type_name*, scope, index
Ref:      control, object*, kind:u8, padding to pointer alignment
Object:   arena, object*
```

`version` must equal 1. An odd `sequence` means a mutation is in progress. The
entire header must remain identical after the snapshot read. Arena IDs are
monotonic identities; parent 0 means no parent. States are 0 Alive, 1 Destroying,
2 Dead. Modes are 0 root, 1 lexical, 2 dynamicfree, 3 dynamicscoped. `payload`
is the arena allocation base, `used` its occupied bytes, and `capacity` its total
bytes. A null payload means absent. Nonzero Ref object addresses must lie between
payload and payload+used, inclusive: the endpoint permits zero-size objects but
never expansion of a nonzero-sized payload.

Ref kinds are 0 None, 1 Arena, 2 Borrow, 3 Strong, 4 Weak. The control word is an
arena ID, not an address to dereference. Object descriptors use the same two
initial words, without the Ref kind byte. Pointers to fields within payloads are
read only for typed children of a live, validated descriptor or Ref.

Slot `owner` is the arena holding persistent Ref storage at `address`. Slots with
missing or non-Alive owners are shown without reading their storage. `label`,
`type_name` and arena `source` point to process-lifetime NUL-terminated UTF-8
strings, or null. Scope 0 denotes heap/unknown storage; other values are dynamic
scope tokens for named locals. `index=usize::MAX` means no array index. Dead target
controls can remain in the arena table while Weak references exist, but their
payload is never read. A freed control ID is stale. Named locals leave the slot
table when their activation scope exits; owner destruction removes slots before
freeing storage.

The reader rejects more than 1,000,000 arena or slot records, strings longer than
4096 bytes, invalid tags/allocation bounds, partial or failed reads, and unstable
headers. These limits bound corrupted snapshots; they do not change runtime data.

After building the compiler, run actual Bork source/method/debugger integration:

```sh
cargo build --features codegen
DEBUGINFOD_URLS= python3 tests/debuggers.py
```

Compiler debug hooks register initialized Ref storage with its arena owner and
activation scope. Array aliases use `bork_debug_label_slot(slot, label, type_name,
index)` to rename an existing entry without transferring ownership or changing its
scope; borrowed parameters preserve caller labels. If several local array names
alias the same storage, the latest explicit local/field label identifies that
storage in the snapshot. TLS teardown invalidates the registry before freeing
payloads or published arrays.

Debug builds choose the bundled debug runtime archive. `BORK_RUNTIME_DEBUG_LIB`
overrides that archive; ordinary builds continue using `BORK_RUNTIME_LIB`. A debug
override must provide ABI v1 (`bork_debug_abi_v1`), otherwise linking fails. Debug
registry maintenance adds runtime work and memory use; `-g` is intended for
inspection, with O0 in both generated code and the debug runtime.
