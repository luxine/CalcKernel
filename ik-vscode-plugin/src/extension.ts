import * as vscode from "vscode";
import { registerCompletions } from "./completions";
import { registerDefinitions } from "./definitions";
import { registerDiagnostics } from "./diagnostics";
import { registerDocumentSymbols } from "./documentSymbols";
import { registerHover } from "./hover";
import { registerSemanticTokens } from "./semanticTokens";

export function activate(context: vscode.ExtensionContext): void {
  registerDiagnostics(context);
  registerCompletions(context);
  registerHover(context);
  registerSemanticTokens(context);
  registerDefinitions(context);
  registerDocumentSymbols(context);
}

export function deactivate(): void {
  // VSCode disposes subscriptions registered on the extension context.
}
