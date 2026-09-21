# Cursor Bork LSP Integration Implementation Plan

> **For agentic workers:** Execute the tasks in order and verify each checkpoint.

**Goal:** Install a local Cursor/VS Code extension that starts the repository's `bork-lsp` server for `.bork` files.

**Architecture:** A small Node extension registers the Bork language and starts the workspace's debug LSP binary. It falls back to `cargo run --quiet --bin bork-lsp` when the binary is absent. The Rust server remains unchanged.

**Tech Stack:** Rust/Cargo, Node.js built-in test runner, `vscode-languageclient`, Cursor CLI, VSIX.

---

### Task 1: Add tested server command selection

**Files:**
- Create: `tools/bork-lsp-extension/test/server.test.js`
- Create: `tools/bork-lsp-extension/server.js`

- [ ] **Step 1: Write failing tests**

Test that an existing `target/debug/bork-lsp` is selected, and that a missing binary selects Cargo with the workspace as cwd. Use a temporary directory and the real filesystem.

- [ ] **Step 2: Run the tests and verify the expected module-not-found failure**

Run: `node --test tools/bork-lsp-extension/test/server.test.js`

Expected: FAIL because `server.js` does not exist yet.

- [ ] **Step 3: Implement the minimal selector**

Export `serverOptions(workspaceRoot)`. Resolve the platform-specific binary path, return it when executable exists, otherwise return `{ command: "cargo", args: ["run", "--quiet", "--bin", "bork-lsp"], options: { cwd: workspaceRoot } }`.

- [ ] **Step 4: Run the tests and verify they pass**

Run: `node --test tools/bork-lsp-extension/test/server.test.js`

Expected: all tests pass.

### Task 2: Add the Cursor extension

**Files:**
- Create: `tools/bork-lsp-extension/package.json`
- Create: `tools/bork-lsp-extension/extension.js`
- Create: `tools/bork-lsp-extension/language-configuration.json`
- Create: `tools/bork-lsp-extension/.vscodeignore`

- [ ] **Step 1: Define the extension manifest**

Register language id `bork`, `.bork` files, activation on `onLanguage:bork`, and the `vscode-languageclient` runtime dependency.

- [ ] **Step 2: Start the client from the first workspace folder**

Use `LanguageClient` with `serverOptions` from `server.js`, `documentSelector: [{ scheme: "file", language: "bork" }]`, and subscribe the client to the extension context.

- [ ] **Step 3: Add basic language configuration**

Register `{}`, `()`, and `[]` brackets and `//` line comments so Cursor treats Bork files as source code.

### Task 3: Package and document the integration

**Files:**
- Modify: `README.md` in the Editor / LSP section.
- Create: `tools/bork-lsp-extension/package-lock.json` via npm.

- [ ] **Step 1: Install the extension dependency**

Run: `npm install --omit=dev` in `tools/bork-lsp-extension`.

- [ ] **Step 2: Package the extension**

Run: `npx @vscode/vsce package --allow-missing-repository` in `tools/bork-lsp-extension`.

Expected: a `.vsix` containing the manifest, client, selector, language configuration, and runtime dependency.

- [ ] **Step 3: Document Cursor installation**

Add the exact build, package, install, and `.bork` usage commands to README.

### Task 4: Build, install, and verify

**Files:**
- Generated: `target/debug/bork-lsp` and the local VSIX.

- [ ] **Step 1: Run Rust verification**

Run: `cargo test`, `cargo test --no-default-features`, and `cargo build --bin bork-lsp`.

- [ ] **Step 2: Install and verify in Cursor**

Run: `cursor --install-extension tools/bork-lsp-extension/*.vsix --force` and confirm the extension appears in `cursor --list-extensions --show-versions`.

- [ ] **Step 3: Run an LSP stdio smoke test**

Send `initialize`, `initialized`, `textDocument/didOpen`, and `shutdown` messages to `target/debug/bork-lsp`; verify the server returns an initialize response and diagnostics notification.

- [ ] **Step 4: Review the diff and final status**

Run: `git diff --check`, `git status --short`, and inspect the generated extension manifest. Report exact paths and verification results.
