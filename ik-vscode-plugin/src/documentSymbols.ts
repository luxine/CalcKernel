import * as vscode from "vscode";
import { analyzeIntKernelDocument, type IntKernelAnalysis, type IntKernelSymbol } from "./languageService";

export function buildDocumentSymbols(analysis: IntKernelAnalysis): vscode.DocumentSymbol[] {
  const topLevel: vscode.DocumentSymbol[] = [];

  for (const symbol of analysis.symbols) {
    if (symbol.kind === "struct") {
      const documentSymbol = toDocumentSymbol(symbol, vscode.SymbolKind.Struct);
      documentSymbol.children = analysis.symbols
        .filter((child) => child.kind === "field" && child.containerName === symbol.name)
        .map((child) => toDocumentSymbol(child, vscode.SymbolKind.Field));
      topLevel.push(documentSymbol);
    }

    if (symbol.kind === "function") {
      const documentSymbol = toDocumentSymbol(symbol, vscode.SymbolKind.Function);
      documentSymbol.children = analysis.symbols
        .filter((child) => (child.kind === "parameter" || child.kind === "local") && child.functionName === symbol.name)
        .map((child) => toDocumentSymbol(child, vscode.SymbolKind.Variable));
      topLevel.push(documentSymbol);
    }
  }

  return topLevel;
}

export function registerDocumentSymbols(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerDocumentSymbolProvider(
      { language: "intkernel" },
      {
        provideDocumentSymbols(document) {
          return buildDocumentSymbols(analyzeIntKernelDocument(document));
        }
      }
    )
  );
}

function toDocumentSymbol(symbol: IntKernelSymbol, kind: vscode.SymbolKind): vscode.DocumentSymbol {
  return new vscode.DocumentSymbol(symbol.name, symbol.detail ?? symbol.signatureLabel ?? "", kind, symbol.range, symbol.selectionRange);
}
