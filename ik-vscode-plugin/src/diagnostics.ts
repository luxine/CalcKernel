import * as vscode from "vscode";
import { SourceFile, check, type Diagnostic as IntKernelDiagnostic } from "intkernel";
import { spanToRangeCoordinates } from "./diagnosticMapping";

const debounceMs = 250;

export function registerDiagnostics(context: vscode.ExtensionContext): void {
  const collection = vscode.languages.createDiagnosticCollection("intkernel");
  const pending = new Map<string, NodeJS.Timeout>();

  function validateNow(document: vscode.TextDocument): void {
    if (!isIntKernelDocument(document)) {
      return;
    }

    clearPending(document, pending);
    validateDocument(document, collection);
  }

  function validateSoon(document: vscode.TextDocument): void {
    if (!isIntKernelDocument(document)) {
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
      collection.delete(document.uri);
    })
  );

  for (const document of vscode.workspace.textDocuments) {
    validateNow(document);
  }
}

function validateDocument(document: vscode.TextDocument, collection: vscode.DiagnosticCollection): void {
  try {
    const source = new SourceFile(document.fileName, document.getText());
    const result = check(source);
    collection.set(
      document.uri,
      result.diagnostics.map((diagnostic) => toVscodeDiagnostic(document, diagnostic))
    );
  } catch (error) {
    collection.set(document.uri, [unexpectedValidationDiagnostic(document, error)]);
  }
}

function toVscodeDiagnostic(document: vscode.TextDocument, diagnostic: IntKernelDiagnostic): vscode.Diagnostic {
  const coordinates = spanToRangeCoordinates(document.getText(), diagnostic.span);
  const vscodeDiagnostic = new vscode.Diagnostic(
    new vscode.Range(
      coordinates.start.line,
      coordinates.start.character,
      coordinates.end.line,
      coordinates.end.character
    ),
    diagnostic.message,
    vscode.DiagnosticSeverity.Error
  );
  vscodeDiagnostic.code = diagnostic.code;
  vscodeDiagnostic.source = "intkernel";
  return vscodeDiagnostic;
}

function unexpectedValidationDiagnostic(document: vscode.TextDocument, error: unknown): vscode.Diagnostic {
  const text = document.getText();
  const endCharacter = Math.min(1, text.split(/\r\n|\r|\n/)[0]?.length ?? 0);
  const diagnostic = new vscode.Diagnostic(
    new vscode.Range(0, 0, 0, endCharacter),
    `IntKernel validation failed: ${errorMessage(error)}`,
    vscode.DiagnosticSeverity.Error
  );
  diagnostic.source = "intkernel";
  return diagnostic;
}

function isIntKernelDocument(document: vscode.TextDocument): boolean {
  return document.languageId === "intkernel" || document.fileName.endsWith(".ik");
}

function clearPending(document: vscode.TextDocument, pending: Map<string, NodeJS.Timeout>): void {
  const key = document.uri.toString();
  const timeout = pending.get(key);
  if (timeout) {
    clearTimeout(timeout);
    pending.delete(key);
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
