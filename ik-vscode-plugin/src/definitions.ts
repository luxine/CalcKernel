import * as vscode from "vscode";
import { analyzeIntKernelDocument, referenceAtPosition, symbolAtPosition, type IntKernelAnalysis } from "./languageService";

export function getDefinitionAtPosition(analysis: IntKernelAnalysis, position: vscode.Position): vscode.Location | undefined {
  const reference = referenceAtPosition(analysis, position);
  const target = reference?.target ?? symbolAtPosition(analysis, position);
  return target ? new vscode.Location(analysis.document.uri, target.selectionRange) : undefined;
}

export function registerDefinitions(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerDefinitionProvider(
      { language: "intkernel" },
      {
        provideDefinition(document, position) {
          return getDefinitionAtPosition(analyzeIntKernelDocument(document), position);
        }
      }
    )
  );
}
