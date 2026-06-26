import * as vscode from "vscode";
import { analyzeCalcKernelDocument, referenceAtPosition, symbolAtPosition, type CalcKernelAnalysis } from "./languageService";

export function getDefinitionAtPosition(analysis: CalcKernelAnalysis, position: vscode.Position): vscode.Location | undefined {
  const reference = referenceAtPosition(analysis, position);
  const target = reference?.target ?? symbolAtPosition(analysis, position);
  return target ? new vscode.Location(analysis.document.uri, target.selectionRange) : undefined;
}

export function registerDefinitions(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerDefinitionProvider(
      { language: "calckernel" },
      {
        provideDefinition(document, position) {
          return getDefinitionAtPosition(analyzeCalcKernelDocument(document), position);
        }
      }
    )
  );
}
