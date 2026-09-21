const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");
const { serverOptions } = require("./server");

let client;

function activate(context) {
  const folder = vscode.workspace.workspaceFolders?.[0];
  if (!folder) {
    vscode.window.showErrorMessage("Bork LSP requires an open workspace.");
    return;
  }

  client = new LanguageClient(
    "borkLanguageServer",
    "Bork Language Server",
    serverOptions(folder.uri.fsPath),
    { documentSelector: [{ scheme: "file", language: "bork" }] },
  );

  context.subscriptions.push(client.start());
}

function deactivate() {
  return client?.stop();
}

module.exports = { activate, deactivate };
