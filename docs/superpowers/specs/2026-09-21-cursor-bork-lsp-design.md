# Bork LSP integration with Cursor

## Goal

Make the existing `bork-lsp` stdio server start automatically in Cursor for
Bork source files with the `.bork` extension.

## Design

- Add a small VS Code-compatible extension under `tools/bork-lsp-extension`.
- Register language id `bork`, the `.bork` extension, and basic bracket/comment
  language configuration.
- Start the workspace binary at `target/debug/bork-lsp`.
- Fall back to `cargo run --quiet --bin bork-lsp` when the binary is missing.
- Package the extension as `tools/bork-lsp-extension/bork-language-support.vsix`
  and install it with the local Cursor CLI.
- Keep the Rust LSP implementation unchanged; it remains the source of parser
  diagnostics and uses stdio transport.

## Data flow

1. Cursor opens a `.bork` file and activates the extension.
2. The extension starts `bork-lsp` with the workspace root as its working
   directory.
3. Cursor and the server communicate over stdin/stdout using LSP.
4. `didOpen` and `didChange` produce parser diagnostics in the editor.

## Failure handling

- Missing `target/debug/bork-lsp` uses Cargo as a reproducible fallback.
- Missing workspace produces a clear extension error and does not start a
  process.
- The server's stderr remains available to Cursor's language-server output.

## Verification

- `cargo test`
- `cargo test --no-default-features`
- `cargo build --bin bork-lsp`
- Package and install the extension with Cursor.
- Validate the generated VSIX and run the server's stdio smoke test.
