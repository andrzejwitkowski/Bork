# Ref and arena debugging implementation plan

Approved in conversation: native Linux source debugging in LLDB and GDB, plus DOT/SVG snapshots of all persistent Ref slots. Implement on `feat/debug-refs-arenas` based on `origin/main` d82e79f (includes class methods).

## Constraints

- `bork build -g`: DWARF, O0, debug-only registry and hooks. Ordinary build retains O3 and existing Ref ABI/lifetime semantics.
- Memory-only debugger readers; no observe/retain/upgrade or other inferior calls.
- Snapshot only, no tracing, DAP, own UI or optimized debugging.
- Keep existing build/backend API wrappers. Debug API requires source context.
- Methods show qualified source names and visible borrowed `this`; runtime ABI parameters stay hidden.

## Tasks and validation

- [x] CLI/build: add BuildOptions/SourceContext/build_with_options, preserve build wrapper; separate debug archive and override BORK_RUNTIME_DEBUG_LIB. Test `-g` program result, DWARF sections, registry symbol, missing source context and incompatible override.
- [x] LLVM: propagate source declaration spans and lexical scopes, emit source types and locations using target data layout; O0 for target and IR; instrument arena labels and stable Ref slots with dynamic scope tokens and early-exit cleanup. Test debug IR and live debugger behavior, methods/this, shadowing, arrays and expired weak.
- [x] Runtime: cfg-gated repr(C) registry, version and mutation sequence, arena state/counters/payload/labels, Ref slot lifecycle and scope hooks; preserve ABI and semantics. Test state transitions, resets/pool reuse, retained Dead weak, reentrancy and local scope cleanup.
- [x] Tools: common Python memory decoder/snapshot/DOT exporter, LLDB summaries/synthetic children and GDB printers, `bork-arenas`/`bork-graph`; docs and literal-memory tests including invalid/partial snapshots, escaping, absent Graphviz and no reads of dead payload.
- [x] Integration: full compiler/runtime suites, default-feature and codegen builds, both real debuggers, graph equivalence and unchanged counters after repeated display. Review whole change before reporting completion.

## Interface and ownership record

The implementation's exact shared ABI and worker boundaries are recorded in `.superpowers/debug-refs-arenas/contract.md` during development; the public format and usage are documented in `docs/debugging.md`. Runtime workers own only crates/bork_runtime; compiler workers only LLVM/source-position plumbing; tools workers only tools/bork-debug and debugging docs; integrator owns CLI/build/link and integration checks.

## Review focus

Stale stack slots after early exits or compacted scopes; debug reads while payload is Destroying/Dead; Ref copies into fields and arrays; type-recursive class layouts; method source names and borrowed receiver rendering. The registry must not mix per-thread arena ID spaces.

## Execution record

Baseline `cargo test --workspace --features codegen` passed using local LLVM library paths and the existing clang shim. This environment does not put LLVM 23 dependencies or clang on default search paths. LLDB is being extracted under /tmp for integration testing without changing repository dependencies.

Final verification passed: workspace default, no-default-features and codegen suites;
debug runtime (28 unit tests and production registry teardown regression); Python
adapter suite (11 tests); actual Bork GDB/LLDB integration (6 tests, including
source stepping/shadowing, methods/this, borrowed arrays, expired Weak, stable
snapshots and graph equivalence). Fresh review findings for TLS teardown and array
alias ownership were fixed and covered by regressions. Native execution was tested
on Linux x86_64; 32-bit/big-endian decoding uses literal-memory fixtures.
