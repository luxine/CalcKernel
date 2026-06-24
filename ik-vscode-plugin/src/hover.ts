import * as vscode from "vscode";
import { analyzeIntKernelDocument, referenceAtPosition, symbolAtPosition, type IntKernelAnalysis } from "./languageService";

export function getHoverMarkdownAtPosition(analysis: IntKernelAnalysis, position: vscode.Position): vscode.MarkdownString | undefined {
  const symbol = symbolAtPosition(analysis, position);
  if (symbol) {
    return new vscode.MarkdownString(codeBlock(symbol.detail ?? symbol.signatureLabel ?? symbol.name));
  }

  const reference = referenceAtPosition(analysis, position);
  if (reference) {
    const label =
      reference.target?.detail ??
      reference.target?.signatureLabel ??
      `${reference.name}${reference.typeLabel ? `: ${reference.typeLabel}` : ""}`;
    return new vscode.MarkdownString(codeBlock(label));
  }

  return undefined;
}

export function registerHover(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerHoverProvider(
      { language: "intkernel" },
      {
        provideHover(document, position) {
          const analysis = analyzeIntKernelDocument(document);
          const markdown = getHoverMarkdownAtPosition(analysis, position);
          return markdown ? new vscode.Hover(markdown) : undefined;
        }
      }
    )
  );
}

function codeBlock(value: string): string {
  return `\`\`\`ik\n${value}\n\`\`\``;
}
