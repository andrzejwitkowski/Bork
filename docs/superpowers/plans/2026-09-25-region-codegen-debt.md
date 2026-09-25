# Region codegen debt — closed 2026-09-25

- Single path: `region_enter` / `region_exit` + visitor `enter_region` / `exit_region`.
- `WalkError` embeds `Diagnostic`; no parallel `walk.err` slot.
- `CodegenWalkState`: trailing value + driver pin on `FnEmitter::walk`.
- `emit_bool` uses walk when driver active.
- `arena_cursor_shared!` for mut/ref cursors.
- Test: `codegen_arena_push_pop_counts_match_schedule`.
