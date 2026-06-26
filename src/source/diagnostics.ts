import type { SourceFile, SourcePosition, SourceSpan } from "./source-file.js";

export type DiagnosticSeverity = "error";
export type DiagnosticCode =
  | "CK0001"
  | "CK1001"
  | "CK2001"
  | "CK2002"
  | "CK2003"
  | "CK2004"
  | "CK2005"
  | "CK2006"
  | "CK2007"
  | "CK2008";

export interface Diagnostic {
  code: DiagnosticCode;
  severity: DiagnosticSeverity;
  message: string;
  fileName: string;
  line: number;
  column: number;
  span: SourceSpan;
}

export function errorAt(source: SourceFile, span: SourceSpan, code: DiagnosticCode, message: string): Diagnostic {
  return {
    code,
    severity: "error",
    message,
    fileName: source.fileName,
    line: span.start.line,
    column: span.start.column,
    span
  };
}

export function spanAt(position: SourcePosition, width = 1): SourceSpan {
  return {
    start: position,
    end: {
      offset: position.offset + width,
      line: position.line,
      column: position.column + width
    }
  };
}

export function formatDiagnostic(sourceFile: SourceFile, diagnostic: Diagnostic): string {
  const sourceLine = sourceFile.text.split(/\r?\n/)[diagnostic.line - 1] ?? "";
  const markerWidth = diagnosticMarkerWidth(diagnostic, sourceLine);
  const caret = `${" ".repeat(Math.max(0, diagnostic.column - 1))}${"^".repeat(markerWidth)}`;
  return `${diagnostic.fileName}:${diagnostic.line}:${diagnostic.column}: ${diagnostic.severity} ${diagnostic.code}: ${diagnostic.message}\n${sourceLine}\n${caret}\n`;
}

export function formatDiagnostics(sourceFile: SourceFile, diagnostics: Diagnostic[]): string {
  return diagnostics.map((diagnostic) => formatDiagnostic(sourceFile, diagnostic)).join("");
}

function diagnosticMarkerWidth(diagnostic: Diagnostic, sourceLine: string): number {
  if (diagnostic.span.start.line === diagnostic.span.end.line) {
    return Math.max(1, diagnostic.span.end.column - diagnostic.span.start.column);
  }

  return Math.max(1, sourceLine.length - diagnostic.column + 1);
}
