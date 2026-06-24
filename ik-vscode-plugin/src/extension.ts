import * as vscode from "vscode";
import { registerCompletions } from "./completions";
import { registerDiagnostics } from "./diagnostics";
import { registerSemanticTokens } from "./semanticTokens";

export function activate(context: vscode.ExtensionContext): void {
  registerDiagnostics(context);
  registerCompletions(context);
  registerSemanticTokens(context);
}

export function deactivate(): void {
  // VSCode disposes subscriptions registered on the extension context.
}
