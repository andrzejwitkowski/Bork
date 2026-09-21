const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const { serverOptions } = require("../server");

const workspace = () => fs.mkdtempSync(path.join(os.tmpdir(), "bork-lsp-"));

test("uses the built workspace server when present", () => {
  const root = workspace();
  const binary = path.join(root, "target", "debug", "bork-lsp");
  fs.mkdirSync(path.dirname(binary), { recursive: true });
  fs.writeFileSync(binary, "binary");

  const options = serverOptions(root);

  assert.equal(options.command, binary);
  assert.deepEqual(options.args, []);
  assert.equal(options.options.cwd, root);
});

test("falls back to Cargo when the workspace server is missing", () => {
  const root = workspace();

  const options = serverOptions(root);

  assert.equal(options.command, "cargo");
  assert.deepEqual(options.args, ["run", "--quiet", "--bin", "bork-lsp"]);
  assert.equal(options.options.cwd, root);
});
