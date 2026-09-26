const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const vscode = require('vscode');

function waitForDiagnostics(uri, predicate) {
  const current = vscode.languages.getDiagnostics(uri);
  if (predicate(current)) return Promise.resolve(current);
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => {
      listener.dispose();
      reject(new Error('Timed out waiting for CK diagnostics'));
    }, 15000);
    const listener = vscode.languages.onDidChangeDiagnostics((event) => {
      if (!event.uris.some((changed) => changed.toString() === uri.toString())) return;
      const diagnostics = vscode.languages.getDiagnostics(uri);
      if (!predicate(diagnostics)) return;
      clearTimeout(timer);
      listener.dispose();
      resolve(diagnostics);
    });
  });
}

exports.run = async function run() {
  const server = process.env.CK_VSCODE_TEST_CKC;
  assert.ok(server, 'CK_VSCODE_TEST_CKC must point to a built ckc');
  await vscode.workspace.getConfiguration('ck').update('server.path', server, vscode.ConfigurationTarget.Global);
  const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'ck-vscode-test-'));
  const file = path.join(folder, 'sample.ck');
  try {
    fs.writeFileSync(file, '@\n');
    const document = await vscode.workspace.openTextDocument(vscode.Uri.file(file));
    await vscode.window.showTextDocument(document);
    const extension = vscode.extensions.getExtension('luxine.calckernel-vscode');
    assert.ok(extension, 'CalcKernel extension must be discoverable');
    await extension.activate();
    const invalid = await waitForDiagnostics(document.uri, (items) => items.some((item) => item.code === 'CK0001'));
    assert.ok(invalid.some((item) => item.source === 'ckc'));

    const edit = new vscode.WorkspaceEdit();
    edit.replace(document.uri, new vscode.Range(document.positionAt(0), document.positionAt(document.getText().length)),
      'fn main() -> i32 { return 1; }\n');
    assert.equal(await vscode.workspace.applyEdit(edit), true);
    await waitForDiagnostics(document.uri, (items) => items.length === 0);
    await vscode.commands.executeCommand('ck.showOutput');
  } finally {
    fs.rmSync(folder, { recursive: true, force: true });
  }
};
