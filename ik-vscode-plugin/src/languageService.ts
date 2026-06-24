import * as vscode from "vscode";
import {
  SourceFile,
  check,
  getExprType,
  type CheckResult,
  type Diagnostic as IntKernelDiagnostic,
  type Expression,
  type FunctionDeclaration,
  type FunctionParam,
  type IfStatement,
  type IndexExpression,
  type LetStatement,
  type SourceSpan,
  type Statement,
  type StructDeclaration,
  type StructField,
  type TypeNode
} from "intkernel";
import { spanToRangeCoordinates } from "./diagnosticMapping";
import { formatFunctionSignature, formatSymbolLabel, formatTypeLabel } from "./typeLabels";

export type IntKernelSymbolKind = "struct" | "field" | "function" | "parameter" | "local";
export type IntKernelReferenceKind = "field" | "function" | "parameter" | "local" | "type";

export interface IntKernelSymbol {
  kind: IntKernelSymbolKind;
  name: string;
  typeLabel?: string;
  signatureLabel?: string;
  range: vscode.Range;
  selectionRange: vscode.Range;
  detail?: string;
  containerName?: string;
  functionName?: string;
  scopeRange?: vscode.Range;
}

export interface IntKernelReference {
  kind: IntKernelReferenceKind;
  name: string;
  range: vscode.Range;
  target?: IntKernelSymbol;
  typeLabel?: string;
}

export interface IntKernelAnalysis {
  document: Pick<vscode.TextDocument, "fileName" | "getText" | "uri" | "version">;
  sourceText: string;
  checkResult?: CheckResult;
  diagnostics: readonly vscode.Diagnostic[];
  symbols: readonly IntKernelSymbol[];
  references: readonly IntKernelReference[];
}

export interface AnalyzeOptions {
  checkDocument?: (source: SourceFile) => CheckResult;
}

interface ReferenceContext {
  isCallCallee?: boolean;
}

export function createMemoryDocument(
  text: string,
  uri = "memory:///sample.ik",
  version = 1
): Pick<vscode.TextDocument, "fileName" | "getText" | "uri" | "version"> {
  return {
    fileName: uri.replace("memory://", ""),
    uri: vscode.Uri.parse(uri),
    version,
    getText: () => text
  };
}

const cache = new Map<string, IntKernelAnalysis>();

export function analyzeIntKernelDocument(
  document: Pick<vscode.TextDocument, "fileName" | "getText" | "uri" | "version">,
  options: AnalyzeOptions = {}
): IntKernelAnalysis {
  const cacheKey = `${document.uri.toString()}@${document.version}`;
  const usesInjectedCheck = Boolean(options.checkDocument);
  const cached = usesInjectedCheck ? undefined : cache.get(cacheKey);
  if (cached) {
    return cached;
  }

  const sourceText = document.getText();
  try {
    const source = new SourceFile(document.fileName, sourceText);
    const checkResult = (options.checkDocument ?? check)(source);
    const diagnostics = checkResult.diagnostics.map((diagnostic) => toVscodeDiagnostic(sourceText, diagnostic));
    const { symbols, references } = buildIndex(sourceText, checkResult);
    const analysis = { document, sourceText, checkResult, diagnostics, symbols, references };
    if (!usesInjectedCheck) {
      storeCachedAnalysis(document.uri, cacheKey, analysis);
    }
    return analysis;
  } catch (error) {
    const analysis = {
      document,
      sourceText,
      diagnostics: [unexpectedValidationDiagnostic(error)],
      symbols: [],
      references: []
    };
    if (!usesInjectedCheck) {
      storeCachedAnalysis(document.uri, cacheKey, analysis);
    }
    return analysis;
  }
}

function storeCachedAnalysis(uri: vscode.Uri, cacheKey: string, analysis: IntKernelAnalysis): void {
  clearAnalysisCache(uri);
  cache.set(cacheKey, analysis);
}

export function clearAnalysisCache(uri?: vscode.Uri): void {
  if (!uri) {
    cache.clear();
    return;
  }
  const prefix = `${uri.toString()}@`;
  for (const key of cache.keys()) {
    if (key.startsWith(prefix)) {
      cache.delete(key);
    }
  }
}

function toVscodeDiagnostic(sourceText: string, diagnostic: IntKernelDiagnostic): vscode.Diagnostic {
  const coordinates = spanToRangeCoordinates(sourceText, diagnostic.span);
  const vscodeDiagnostic = new vscode.Diagnostic(
    new vscode.Range(coordinates.start.line, coordinates.start.character, coordinates.end.line, coordinates.end.character),
    diagnostic.message,
    vscode.DiagnosticSeverity.Error
  );
  vscodeDiagnostic.code = diagnostic.code;
  vscodeDiagnostic.source = "intkernel";
  return vscodeDiagnostic;
}

function unexpectedValidationDiagnostic(error: unknown): vscode.Diagnostic {
  const diagnostic = new vscode.Diagnostic(
    new vscode.Range(0, 0, 0, 1),
    `IntKernel validation failed: ${error instanceof Error ? error.message : String(error)}`,
    vscode.DiagnosticSeverity.Error
  );
  diagnostic.source = "intkernel";
  return diagnostic;
}

function buildIndex(sourceText: string, checkResult: CheckResult): { symbols: IntKernelSymbol[]; references: IntKernelReference[] } {
  const symbols: IntKernelSymbol[] = [];
  const references: IntKernelReference[] = [];
  const symbolsByName = new Map<string, IntKernelSymbol[]>();

  function remember(symbol: IntKernelSymbol): void {
    symbols.push(symbol);
    const bucket = symbolsByName.get(symbol.name) ?? [];
    bucket.push(symbol);
    symbolsByName.set(symbol.name, bucket);
  }

  for (const declaration of checkResult.ast.declarations) {
    if (declaration.kind === "StructDeclaration") {
      addStructSymbols(sourceText, declaration, remember);
    }
    if (declaration.kind === "FunctionDeclaration") {
      addFunctionSymbols(sourceText, checkResult, declaration, remember);
    }
  }

  collectTypeReferences(sourceText, checkResult, symbolsByName, references);
  collectReferences(sourceText, checkResult, symbolsByName, references);

  return { symbols, references };
}

function collectTypeReferences(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[]
): void {
  for (const declaration of checkResult.ast.declarations) {
    if (declaration.kind === "StructDeclaration") {
      declaration.fields.forEach((field) => collectTypeNodeReferences(sourceText, symbolsByName, references, field.type));
    }
    if (declaration.kind === "FunctionDeclaration") {
      declaration.params.forEach((param) => collectTypeNodeReferences(sourceText, symbolsByName, references, param.type));
      collectTypeNodeReferences(sourceText, symbolsByName, references, declaration.returnType);
      for (const statement of declaration.body.statements) {
        collectStatementTypeReferences(sourceText, symbolsByName, references, statement);
      }
    }
  }
}

function collectStatementTypeReferences(
  sourceText: string,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[],
  statement: Statement
): void {
  switch (statement.kind) {
    case "LetStatement":
      collectTypeNodeReferences(sourceText, symbolsByName, references, statement.type);
      return;
    case "BlockStatement":
      statement.statements.forEach((child) => collectStatementTypeReferences(sourceText, symbolsByName, references, child));
      return;
    case "IfStatement":
      statement.thenBlock.statements.forEach((child) => collectStatementTypeReferences(sourceText, symbolsByName, references, child));
      statement.elseBlock?.statements.forEach((child) => collectStatementTypeReferences(sourceText, symbolsByName, references, child));
      return;
    case "WhileStatement":
      statement.body.statements.forEach((child) => collectStatementTypeReferences(sourceText, symbolsByName, references, child));
      return;
    case "AssignmentStatement":
    case "ReturnStatement":
    case "ErrorStatement":
      return;
  }
}

function collectTypeNodeReferences(
  sourceText: string,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[],
  typeNode: TypeNode
): void {
  if (typeNode.kind === "NamedType") {
    const target = symbolsByName.get(typeNode.name.name)?.find((symbol) => symbol.kind === "struct");
    references.push({
      kind: "type",
      name: typeNode.name.name,
      range: rangeFromCompilerSpan(sourceText, typeNode.name.span),
      target
    });
    return;
  }
  if (typeNode.kind === "PointerType") {
    collectTypeNodeReferences(sourceText, symbolsByName, references, typeNode.elementType);
  }
}

function collectReferences(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[]
): void {
  for (const declaration of checkResult.ast.declarations) {
    if (declaration.kind === "FunctionDeclaration") {
      for (const statement of declaration.body.statements) {
        collectStatementReferences(sourceText, checkResult, symbolsByName, references, statement);
      }
    }
  }
}

function collectStatementReferences(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[],
  statement: Statement
): void {
  switch (statement.kind) {
    case "BlockStatement":
      statement.statements.forEach((child) => collectStatementReferences(sourceText, checkResult, symbolsByName, references, child));
      return;
    case "LetStatement":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, statement.initializer);
      return;
    case "AssignmentStatement":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, statement.target);
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, statement.value);
      return;
    case "ReturnStatement":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, statement.value);
      return;
    case "IfStatement":
      collectIfStatementReferences(sourceText, checkResult, symbolsByName, references, statement);
      return;
    case "WhileStatement":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, statement.condition);
      statement.body.statements.forEach((child) => collectStatementReferences(sourceText, checkResult, symbolsByName, references, child));
      return;
    case "ErrorStatement":
      return;
  }
}

function collectIfStatementReferences(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[],
  statement: IfStatement
): void {
  collectExpressionReferences(sourceText, checkResult, symbolsByName, references, statement.condition);
  statement.thenBlock.statements.forEach((child) => collectStatementReferences(sourceText, checkResult, symbolsByName, references, child));
  statement.elseBlock?.statements.forEach((child) => collectStatementReferences(sourceText, checkResult, symbolsByName, references, child));
}

function collectExpressionReferences(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[],
  expression: Expression,
  context: ReferenceContext = {}
): void {
  const reference = referenceFromExpression(sourceText, checkResult, symbolsByName, expression, context);
  if (reference) {
    references.push(reference);
  }

  switch (expression.kind) {
    case "UnaryExpression":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.operand);
      return;
    case "BinaryExpression":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.left);
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.right);
      return;
    case "CallExpression":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.callee, { isCallCallee: true });
      expression.args.forEach((arg) => collectExpressionReferences(sourceText, checkResult, symbolsByName, references, arg));
      return;
    case "FieldExpression":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.object);
      return;
    case "IndexExpression":
      collectIndexExpressionReferences(sourceText, checkResult, symbolsByName, references, expression);
      return;
    case "ParenthesizedExpression":
      collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.expression, context);
      return;
    case "IdentifierExpression":
    case "IntegerLiteral":
    case "BoolLiteral":
    case "ErrorExpression":
      return;
  }
}

function collectIndexExpressionReferences(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  references: IntKernelReference[],
  expression: IndexExpression
): void {
  collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.object);
  collectExpressionReferences(sourceText, checkResult, symbolsByName, references, expression.index);
}

function addStructSymbols(sourceText: string, declaration: StructDeclaration, remember: (symbol: IntKernelSymbol) => void): void {
  const structSymbol = symbolFromNode(sourceText, "struct", declaration.name.name, declaration.span, declaration.name.span, {
    detail: formatSymbolLabel("struct", declaration.name.name)
  });
  remember(structSymbol);
  for (const field of declaration.fields) {
    remember(fieldSymbolFromNode(sourceText, declaration.name.name, field));
  }
}

function addFunctionSymbols(
  sourceText: string,
  checkResult: CheckResult,
  declaration: FunctionDeclaration,
  remember: (symbol: IntKernelSymbol) => void
): void {
  const functionInfo = checkResult.checkedProgram.functionMap.get(declaration.name.name);
  const signatureLabel = functionInfo ? formatFunctionSignature(functionInfo) : undefined;
  remember(symbolFromNode(sourceText, "function", declaration.name.name, declaration.span, declaration.name.span, { signatureLabel, detail: signatureLabel }));
  const functionScopeRange = rangeFromCompilerSpan(sourceText, declaration.body.span);
  declaration.params.forEach((param, index) => {
    const type = functionInfo?.params[index]?.type;
    remember(paramSymbolFromNode(sourceText, param, declaration.name.name, functionScopeRange, type ? formatTypeLabel(type) : undefined));
  });
  for (const statement of declaration.body.statements) {
    collectLocalSymbols(sourceText, checkResult, statement, declaration.name.name, functionScopeRange, remember);
  }
}

function symbolFromNode(
  sourceText: string,
  kind: IntKernelSymbolKind,
  name: string,
  rangeSpan: SourceSpan,
  selectionSpan: SourceSpan,
  extra: Partial<IntKernelSymbol> = {}
): IntKernelSymbol {
  const range = rangeFromCompilerSpan(sourceText, rangeSpan);
  const selectionRange = rangeFromCompilerSpan(sourceText, selectionSpan);
  return { kind, name, range, selectionRange, ...extra };
}

function fieldSymbolFromNode(sourceText: string, containerName: string, field: StructField): IntKernelSymbol {
  const typeLabel = field.type.kind === "PrimitiveType" ? field.type.name : field.type.kind === "NamedType" ? field.type.name.name : undefined;
  return symbolFromNode(sourceText, "field", field.name.name, field.span, field.name.span, {
    typeLabel,
    containerName,
    detail: formatSymbolLabel("field", field.name.name, typeLabel, containerName)
  });
}

function paramSymbolFromNode(
  sourceText: string,
  param: FunctionParam,
  functionName: string,
  scopeRange: vscode.Range,
  typeLabel?: string
): IntKernelSymbol {
  return symbolFromNode(sourceText, "parameter", param.name.name, param.span, param.name.span, {
    functionName,
    scopeRange,
    typeLabel,
    detail: formatSymbolLabel("parameter", param.name.name, typeLabel)
  });
}

function collectLocalSymbols(
  sourceText: string,
  checkResult: CheckResult,
  statement: Statement,
  functionName: string,
  scopeRange: vscode.Range,
  remember: (symbol: IntKernelSymbol) => void
): void {
  if (statement.kind === "LetStatement") {
    const type = checkResult.checkedProgram.localTypes.get(statement);
    remember(localSymbolFromNode(sourceText, statement, functionName, scopeRange, type ? formatTypeLabel(type) : undefined));
  }
  if (statement.kind === "BlockStatement") {
    const blockScopeRange = rangeFromCompilerSpan(sourceText, statement.span);
    statement.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, functionName, blockScopeRange, remember));
  }
  if (statement.kind === "IfStatement") {
    const thenScopeRange = rangeFromCompilerSpan(sourceText, statement.thenBlock.span);
    statement.thenBlock.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, functionName, thenScopeRange, remember));
    if (statement.elseBlock) {
      const elseScopeRange = rangeFromCompilerSpan(sourceText, statement.elseBlock.span);
      statement.elseBlock.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, functionName, elseScopeRange, remember));
    }
  }
  if (statement.kind === "WhileStatement") {
    const bodyScopeRange = rangeFromCompilerSpan(sourceText, statement.body.span);
    statement.body.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, functionName, bodyScopeRange, remember));
  }
}

function localSymbolFromNode(
  sourceText: string,
  statement: LetStatement,
  functionName: string,
  scopeRange: vscode.Range,
  typeLabel?: string
): IntKernelSymbol {
  return symbolFromNode(sourceText, "local", statement.name.name, statement.span, statement.name.span, {
    functionName,
    scopeRange,
    typeLabel,
    detail: formatSymbolLabel("local", statement.name.name, typeLabel)
  });
}

function rangeFromCompilerSpan(sourceText: string, span: SourceSpan): vscode.Range {
  const coordinates = spanToRangeCoordinates(sourceText, span);
  return new vscode.Range(coordinates.start.line, coordinates.start.character, coordinates.end.line, coordinates.end.character);
}

function referenceFromExpression(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  expression: Expression,
  context: ReferenceContext = {}
): IntKernelReference | undefined {
  if (expression.kind === "IdentifierExpression") {
    const scopedTarget = scopedIdentifierTarget(symbolsByName, expression);
    const functionSymbol = checkResult.checkedProgram.functionMap.get(expression.name);
    const functionTarget = symbolsByName.get(expression.name)?.find((symbol) => functionSymbol && symbol.kind === "function");
    const target = context.isCallCallee ? functionTarget ?? scopedTarget : scopedTarget ?? functionTarget;
    const exprType = getExprType(checkResult.checkedProgram, expression);
    return {
      kind: target?.kind === "function" ? "function" : target?.kind === "parameter" ? "parameter" : "local",
      name: expression.name,
      range: rangeFromCompilerSpan(sourceText, expression.span),
      target,
      typeLabel: exprType ? formatTypeLabel(exprType) : undefined
    };
  }

  if (expression.kind === "FieldExpression") {
    if (expression.field.name === "" || isZeroWidthSpan(expression.field.span)) {
      return undefined;
    }
    const objectType = getExprType(checkResult.checkedProgram, expression.object);
    const containerName = objectType?.kind === "struct" ? objectType.name : undefined;
    const target = symbolsByName
      .get(expression.field.name)
      ?.find((symbol) => symbol.kind === "field" && symbol.containerName === containerName);
    const exprType = getExprType(checkResult.checkedProgram, expression);
    return {
      kind: "field",
      name: expression.field.name,
      range: rangeFromCompilerSpan(sourceText, expression.field.span),
      target,
      typeLabel: exprType ? formatTypeLabel(exprType) : undefined
    };
  }

  return undefined;
}

function isZeroWidthSpan(span: SourceSpan): boolean {
  return span.start.offset === span.end.offset;
}

function scopedIdentifierTarget(
  symbolsByName: Map<string, IntKernelSymbol[]>,
  expression: Extract<Expression, { kind: "IdentifierExpression" }>
): IntKernelSymbol | undefined {
  const referenceRangeStart = expression.span.start;
  return symbolsByName
    .get(expression.name)
    ?.filter((symbol) => {
      if (symbol.kind !== "local" && symbol.kind !== "parameter") {
        return false;
      }
      if (!symbol.scopeRange?.contains(new vscode.Position(referenceRangeStart.line - 1, referenceRangeStart.column - 1))) {
        return false;
      }
      return compareSpanStarts(symbol.selectionRange.start, referenceRangeStart) <= 0;
    })
    .sort((left, right) => comparePositions(right.selectionRange.start, left.selectionRange.start))[0];
}

function compareSpanStarts(position: vscode.Position, spanStart: SourceSpan["start"]): number {
  return comparePositions(position, new vscode.Position(spanStart.line - 1, spanStart.column - 1));
}

function comparePositions(left: vscode.Position, right: vscode.Position): number {
  if (left.line !== right.line) {
    return left.line - right.line;
  }
  return left.character - right.character;
}

export function symbolAtPosition(analysis: IntKernelAnalysis, position: vscode.Position): IntKernelSymbol | undefined {
  return analysis.symbols.find((symbol) => symbol.selectionRange.contains(position));
}

export function referenceAtPosition(analysis: IntKernelAnalysis, position: vscode.Position): IntKernelReference | undefined {
  return analysis.references.find((reference) => reference.range.contains(position));
}

export function symbolsInDocument(analysis: IntKernelAnalysis, kind?: IntKernelSymbolKind): readonly IntKernelSymbol[] {
  return kind ? analysis.symbols.filter((symbol) => symbol.kind === kind) : analysis.symbols;
}
