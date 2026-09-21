const fs = require("node:fs");
const path = require("node:path");

function serverOptions(workspaceRoot) {
  const binary = path.join(
    workspaceRoot,
    "target",
    "debug",
    process.platform === "win32" ? "bork-lsp.exe" : "bork-lsp",
  );

  return fs.existsSync(binary)
    ? { command: binary, args: [], options: { cwd: workspaceRoot } }
    : {
        command: "cargo",
        args: ["run", "--quiet", "--bin", "bork-lsp"],
        options: { cwd: workspaceRoot },
      };
}

module.exports = { serverOptions };
