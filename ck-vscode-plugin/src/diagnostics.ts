import * as vscode from "vscode";
import { analyzeCalcKernelDocument, clearAnalysisCache } from "./languageService";

const debounceMs = 250;

export function registerDiagnostics(context: vscode.ExtensionContext): void {
  const collection = vscode.languages.createDiagnosticCollection("calckernel");
  const pending = new Map<string, NodeJS.Timeout>();

  function validateNow(document: vscode.TextDocument): void {
    if (!isCalcKernelDocument(document)) {
      return;
    }

    clearPending(document, pending);
    validateDocument(document, collection);
  }

  function validateSoon(document: vscode.TextDocument): void {
    if (!isCalcKernelDocument(document)) {
      return;
    }

    clearPending(document, pending);
    const key = document.uri.toString();
    pending.set(
      key,
      setTimeout(() => {
        pending.delete(key);
        validateDocument(document, collection);
      }, debounceMs)
    );
  }

  context.subscriptions.push(
    collection,
    vscode.workspace.onDidOpenTextDocument(validateNow),
    vscode.workspace.onDidSaveTextDocument(validateNow),
    vscode.workspace.onDidChangeTextDocument((event) => validateSoon(event.document)),
    vscode.workspace.onDidCloseTextDocument((document) => {
      clearPending(document, pending);
      clearAnalysisCache(document.uri);
      collection.delete(document.uri);
    }),
    {
      dispose: () => {
        for (const timeout of pending.values()) {
          clearTimeout(timeout);
        }
        pending.clear();
      }
    }
  );

  for (const document of vscode.workspace.textDocuments) {
    validateNow(document);
  }
}

function validateDocument(document: vscode.TextDocument, collection: vscode.DiagnosticCollection): void {
  const analysis = analyzeCalcKernelDocument(document);
  collection.set(document.uri, [...analysis.diagnostics]);
}

function isCalcKernelDocument(document: vscode.TextDocument): boolean {
  return document.languageId === "calckernel" || document.fileName.endsWith(".ck");
}

function clearPending(document: vscode.TextDocument, pending: Map<string, NodeJS.Timeout>): void {
  const key = document.uri.toString();
  const timeout = pending.get(key);
  if (timeout) {
    clearTimeout(timeout);
    pending.delete(key);
  }
}
