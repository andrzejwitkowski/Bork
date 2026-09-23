const vscode = require("vscode");
const { LanguageClient } = require("vscode-languageclient/node");
const { serverOptions } = require("./server");

let client;
let arenasChannel;

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

  arenasChannel = vscode.window.createOutputChannel("Bork Arenas");
  context.subscriptions.push(arenasChannel);

  const dumpCmd = vscode.commands.registerCommand("bork.dumpArenas", async () => {
    const editor = vscode.window.activeTextEditor;
    if (!editor || editor.document.languageId !== "bork") {
      vscode.window.showWarningMessage("Open a .bork file to dump arenas.");
      return;
    }
    try {
      const result = await client.sendRequest("workspace/executeCommand", {
        command: "bork.dumpArenas",
        arguments: [editor.document.uri.toString()],
      });
      if (result?.dump) {
        arenasChannel.clear();
        arenasChannel.append(result.dump);
        arenasChannel.show(true);
      } else {
        vscode.window.showErrorMessage(result?.error || "Dump arenas failed");
      }
    } catch (err) {
      vscode.window.showErrorMessage(String(err));
    }
  });
  context.subscriptions.push(dumpCmd);
}

function deactivate() {
  return client?.stop();
}

module.exports = { activate, deactivate };
