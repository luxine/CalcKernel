# IntKernel VSCode V2-B Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build compiler-aware VSCode editor features for IntKernel `.ik` files: shared document analysis, semantic tokens, hover, AST-aware completions, go to definition, and document symbols.

**Architecture:** Keep this as a VSCode extension, not an LSP server. Add a shared `languageService` that wraps the public `intkernel` compiler API and caches per-document analysis by URI/version. Providers consume the shared analysis and expose focused VSCode features without each re-running `check()`.

**Tech Stack:** TypeScript, VSCode Extension API, Vitest, esbuild, local `intkernel` package dependency.

---

## Scope Boundaries

- Work in `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b`.
- Modify only files under `ik-vscode-plugin/`.
- Do not edit root compiler source files unless a task explicitly stops and reports an approved compiler API gap.
- Keep all new tests under `ik-vscode-plugin/test`.
- Use `pnpm test`, `pnpm compile`, and `pnpm package` from `ik-vscode-plugin` as the verification path.

## File Structure

- Create `ik-vscode-plugin/src/astTraversal.ts`
  - Shared AST walking helpers and range containment helpers.
- Create `ik-vscode-plugin/test/astTraversal.test.ts`
  - Unit tests for statement/expression traversal and range containment.
- Create `ik-vscode-plugin/src/typeLabels.ts`
  - Pure formatting helpers for IntKernel types, function signatures, and symbol labels.
- Create `ik-vscode-plugin/test/typeLabels.test.ts`
  - Unit tests for type and signature formatting.
- Create `ik-vscode-plugin/src/languageService.ts`
  - Cached document analysis, diagnostics conversion, symbol extraction, references, and lookup helpers.
- Create `ik-vscode-plugin/test/languageService.test.ts`
  - Unit tests for analysis caching, symbol extraction, reference targeting, and failure fallback.
- Modify `ik-vscode-plugin/src/diagnostics.ts`
  - Use `languageService` instead of calling `check()` directly.
- Create `ik-vscode-plugin/src/semanticTokens.ts`
  - Semantic token legend, builder, and provider registration.
- Create `ik-vscode-plugin/test/semanticTokens.test.ts`
  - Unit tests for token classification and ordering.
- Create `ik-vscode-plugin/src/hover.ts`
  - Hover formatting and provider registration.
- Create `ik-vscode-plugin/test/hover.test.ts`
  - Unit tests for hover content selection.
- Modify `ik-vscode-plugin/src/completions.ts`
  - Keep static completions and add analysis-backed symbol and field completions.
- Create `ik-vscode-plugin/test/completions.test.ts`
  - Unit tests for local/function/type/field completion items.
- Create `ik-vscode-plugin/src/definitions.ts`
  - Definition lookup and provider registration.
- Create `ik-vscode-plugin/test/definitions.test.ts`
  - Unit tests for target resolution.
- Create `ik-vscode-plugin/src/documentSymbols.ts`
  - Outline symbol builder and provider registration.
- Create `ik-vscode-plugin/test/documentSymbols.test.ts`
  - Unit tests for struct/function outline output.
- Modify `ik-vscode-plugin/src/extension.ts`
  - Register all V2-B providers.
- Modify `ik-vscode-plugin/README.md`
  - Document V2-B features and manual verification.
- Modify `ik-vscode-plugin/CHANGELOG.md`
  - Add V2-B feature entry.

## Task 1: AST Traversal And Label Formatting

**Files:**
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/astTraversal.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/typeLabels.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/astTraversal.test.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/typeLabels.test.ts`

- [ ] **Step 1: Write failing traversal tests**

Create `test/astTraversal.test.ts`:

```ts
import { SourceFile, parse } from "intkernel";
import { describe, expect, it } from "vitest";
import { containsPosition, walkProgram } from "../src/astTraversal";

const sourceText = `
struct Item {
  price: i64;
}

fn add_tax(price: i64, tax: i64) -> i64 {
  let total: i64 = price + tax;
  return total;
}
`.trimStart();

describe("astTraversal", () => {
  it("visits declarations, statements, and expressions in source order", () => {
    const parsed = parse(new SourceFile("sample.ik", sourceText));
    const visits: string[] = [];

    walkProgram(parsed.ast, {
      declaration: (node) => visits.push(node.kind),
      statement: (node) => visits.push(node.kind),
      expression: (node) => visits.push(node.kind)
    });

    expect(visits).toEqual([
      "StructDeclaration",
      "FunctionDeclaration",
      "LetStatement",
      "BinaryExpression",
      "IdentifierExpression",
      "IdentifierExpression",
      "ReturnStatement",
      "IdentifierExpression"
    ]);
  });

  it("checks zero-based positions against one-based compiler spans", () => {
    const parsed = parse(new SourceFile("sample.ik", sourceText));
    const functionDeclaration = parsed.ast.declarations[1]!;

    expect(containsPosition(functionDeclaration.span, { line: 5, character: 3 })).toBe(true);
    expect(containsPosition(functionDeclaration.span, { line: 0, character: 0 })).toBe(false);
  });
});
```

- [ ] **Step 2: Run traversal tests and verify red**

Run:

```sh
pnpm test -- test/astTraversal.test.ts
```

Expected: fails with an import error for `../src/astTraversal`.

- [ ] **Step 3: Implement traversal helpers**

Create `src/astTraversal.ts`:

```ts
import type {
  BinaryExpression,
  BlockStatement,
  CallExpression,
  Declaration,
  Expression,
  FunctionDeclaration,
  IfStatement,
  IndexExpression,
  Program,
  Statement,
  UnaryExpression,
  WhileStatement
} from "intkernel";
import type { SourcePosition, SourceSpan } from "intkernel";

export interface ZeroBasedPosition {
  line: number;
  character: number;
}

export interface AstVisitor {
  declaration?: (node: Declaration) => void;
  statement?: (node: Statement) => void;
  expression?: (node: Expression) => void;
}

export function walkProgram(program: Program, visitor: AstVisitor): void {
  for (const declaration of program.declarations) {
    visitor.declaration?.(declaration);
    if (declaration.kind === "FunctionDeclaration") {
      walkFunction(declaration, visitor);
    }
  }
}

export function containsPosition(span: SourceSpan, position: ZeroBasedPosition): boolean {
  const start = toZeroBased(span.start);
  const end = toZeroBased(span.end);
  if (position.line < start.line || position.line > end.line) {
    return false;
  }
  if (position.line === start.line && position.character < start.character) {
    return false;
  }
  if (position.line === end.line && position.character > end.character) {
    return false;
  }
  return true;
}

export function toZeroBased(position: SourcePosition): ZeroBasedPosition {
  return {
    line: Math.max(0, position.line - 1),
    character: Math.max(0, position.column - 1)
  };
}

function walkFunction(declaration: FunctionDeclaration, visitor: AstVisitor): void {
  walkBlock(declaration.body, visitor);
}

function walkBlock(block: BlockStatement, visitor: AstVisitor): void {
  for (const statement of block.statements) {
    visitor.statement?.(statement);
    walkStatement(statement, visitor);
  }
}

function walkStatement(statement: Statement, visitor: AstVisitor): void {
  switch (statement.kind) {
    case "BlockStatement":
      walkBlock(statement, visitor);
      return;
    case "LetStatement":
      walkExpression(statement.initializer, visitor);
      return;
    case "AssignmentStatement":
      walkExpression(statement.target, visitor);
      walkExpression(statement.value, visitor);
      return;
    case "ReturnStatement":
      walkExpression(statement.value, visitor);
      return;
    case "IfStatement":
      walkIfStatement(statement, visitor);
      return;
    case "WhileStatement":
      walkWhileStatement(statement, visitor);
      return;
    case "ErrorStatement":
      return;
  }
}

function walkIfStatement(statement: IfStatement, visitor: AstVisitor): void {
  walkExpression(statement.condition, visitor);
  walkBlock(statement.thenBlock, visitor);
  if (statement.elseBlock) {
    walkBlock(statement.elseBlock, visitor);
  }
}

function walkWhileStatement(statement: WhileStatement, visitor: AstVisitor): void {
  walkExpression(statement.condition, visitor);
  walkBlock(statement.body, visitor);
}

function walkExpression(expression: Expression, visitor: AstVisitor): void {
  visitor.expression?.(expression);
  switch (expression.kind) {
    case "UnaryExpression":
      walkUnaryExpression(expression, visitor);
      return;
    case "BinaryExpression":
      walkBinaryExpression(expression, visitor);
      return;
    case "CallExpression":
      walkCallExpression(expression, visitor);
      return;
    case "FieldExpression":
      walkExpression(expression.object, visitor);
      return;
    case "IndexExpression":
      walkIndexExpression(expression, visitor);
      return;
    case "ParenthesizedExpression":
      walkExpression(expression.expression, visitor);
      return;
    case "IdentifierExpression":
    case "IntegerLiteral":
    case "BoolLiteral":
    case "ErrorExpression":
      return;
  }
}

function walkUnaryExpression(expression: UnaryExpression, visitor: AstVisitor): void {
  walkExpression(expression.operand, visitor);
}

function walkBinaryExpression(expression: BinaryExpression, visitor: AstVisitor): void {
  walkExpression(expression.left, visitor);
  walkExpression(expression.right, visitor);
}

function walkCallExpression(expression: CallExpression, visitor: AstVisitor): void {
  walkExpression(expression.callee, visitor);
  for (const arg of expression.args) {
    walkExpression(arg, visitor);
  }
}

function walkIndexExpression(expression: IndexExpression, visitor: AstVisitor): void {
  walkExpression(expression.object, visitor);
  walkExpression(expression.index, visitor);
}
```

- [ ] **Step 4: Write failing type label tests**

Create `test/typeLabels.test.ts`:

```ts
import { SourceFile, check, getFunctionInfo, getStructInfo } from "intkernel";
import { describe, expect, it } from "vitest";
import { formatFunctionSignature, formatSymbolLabel, formatTypeLabel } from "../src/typeLabels";

const sourceText = `
struct Item {
  price: i64;
}

fn add_tax(price: i64, tax: i64) -> i64 {
  return price + tax;
}
`.trimStart();

describe("typeLabels", () => {
  const checked = check(new SourceFile("sample.ik", sourceText)).checkedProgram;

  it("formats primitive, pointer, and struct types", () => {
    expect(formatTypeLabel({ kind: "primitive", name: "i64" })).toBe("i64");
    expect(formatTypeLabel({ kind: "pointer", elementType: { kind: "struct", name: "Item" } })).toBe("ptr<Item>");
  });

  it("formats function signatures from checked program info", () => {
    const info = getFunctionInfo(checked, "add_tax");
    expect(info && formatFunctionSignature(info)).toBe("fn add_tax(price: i64, tax: i64) -> i64");
  });

  it("formats symbol labels", () => {
    const structInfo = getStructInfo(checked, "Item");
    expect(structInfo && formatSymbolLabel("struct", "Item", undefined, structInfo.name)).toBe("struct Item");
    expect(formatSymbolLabel("local", "total", "i64")).toBe("local total: i64");
  });
});
```

- [ ] **Step 5: Run type label tests and verify red**

Run:

```sh
pnpm test -- test/typeLabels.test.ts
```

Expected: fails with an import error for `../src/typeLabels`.

- [ ] **Step 6: Implement type label helpers**

Create `src/typeLabels.ts`:

```ts
import type { FunctionInfo, IntKernelType } from "intkernel";

export type LabelSymbolKind = "struct" | "field" | "function" | "parameter" | "local" | "type";

export function formatTypeLabel(type: IntKernelType): string {
  switch (type.kind) {
    case "primitive":
      return type.name;
    case "pointer":
      return `ptr<${formatTypeLabel(type.elementType)}>`;
    case "struct":
      return type.name;
    case "integerLiteral":
      return "integer";
    case "unknown":
      return "unknown";
  }
}

export function formatFunctionSignature(info: FunctionInfo): string {
  const params = info.params.map((param) => `${param.name}: ${formatTypeLabel(param.type)}`).join(", ");
  return `fn ${info.name}(${params}) -> ${formatTypeLabel(info.returnType)}`;
}

export function formatSymbolLabel(kind: LabelSymbolKind, name: string, typeLabel?: string, containerName?: string): string {
  if (kind === "function") {
    return name;
  }
  if (kind === "struct") {
    return `struct ${name}`;
  }
  if (kind === "field") {
    return `field ${containerName ? `${containerName}.` : ""}${name}${typeLabel ? `: ${typeLabel}` : ""}`;
  }
  if (kind === "parameter") {
    return `parameter ${name}${typeLabel ? `: ${typeLabel}` : ""}`;
  }
  if (kind === "local") {
    return `local ${name}${typeLabel ? `: ${typeLabel}` : ""}`;
  }
  return `type ${name}`;
}
```

- [ ] **Step 7: Verify Task 1 green**

Run:

```sh
pnpm test -- test/astTraversal.test.ts test/typeLabels.test.ts
```

Expected: both test files pass.

- [ ] **Step 8: Commit Task 1**

Run:

```sh
git add ik-vscode-plugin/src/astTraversal.ts ik-vscode-plugin/src/typeLabels.ts ik-vscode-plugin/test/astTraversal.test.ts ik-vscode-plugin/test/typeLabels.test.ts
git commit -m "feat: add intkernel vscode analysis utilities"
```

## Task 2: Shared Language Service And Symbol Index

**Files:**
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/languageService.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/languageService.test.ts`

- [ ] **Step 1: Write failing language service tests**

Create `test/languageService.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";

const sourceText = `
struct Item {
  price: i64;
  qty: i64;
}

fn line_total(item: Item, tax_rate: i64) -> i64 {
  let subtotal: i64 = item.price * item.qty;
  return subtotal + tax_rate;
}
`.trimStart();

describe("languageService", () => {
  it("extracts document-local symbols and references", () => {
    const document = createMemoryDocument(sourceText);
    const analysis = analyzeIntKernelDocument(document);

    expect(analysis.diagnostics).toHaveLength(0);
    expect(analysis.symbols.map((symbol) => `${symbol.kind}:${symbol.name}`)).toEqual([
      "struct:Item",
      "field:price",
      "field:qty",
      "function:line_total",
      "parameter:item",
      "parameter:tax_rate",
      "local:subtotal"
    ]);
    expect(analysis.references.some((reference) => reference.kind === "field" && reference.name === "price")).toBe(true);
    expect(analysis.references.some((reference) => reference.kind === "local" && reference.name === "subtotal")).toBe(true);
  });

  it("reuses cached analysis for the same URI and version", () => {
    const document = createMemoryDocument(sourceText, "memory:///sample.ik", 7);
    const first = analyzeIntKernelDocument(document);
    const second = analyzeIntKernelDocument(document);
    expect(second).toBe(first);
  });

  it("falls back to a single diagnostic when check throws", () => {
    const document = createMemoryDocument(sourceText, "memory:///sample.ik", 1);
    const analysis = analyzeIntKernelDocument(document, {
      checkDocument: () => {
        throw new Error("forced failure");
      }
    });

    expect(analysis.diagnostics).toHaveLength(1);
    expect(analysis.diagnostics[0]?.message).toContain("IntKernel validation failed: forced failure");
    expect(analysis.symbols).toHaveLength(0);
    expect(analysis.references).toHaveLength(0);
  });
});
```

- [ ] **Step 2: Run language service tests and verify red**

Run:

```sh
pnpm test -- test/languageService.test.ts
```

Expected: fails with an import error for `../src/languageService`.

- [ ] **Step 3: Implement public types and memory document helper**

Create `src/languageService.ts` with these exported types and helper first:

```ts
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
  type LetStatement,
  type StructDeclaration,
  type StructField
} from "intkernel";
import { spanToRangeCoordinates } from "./diagnosticMapping";
import { walkProgram } from "./astTraversal";
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

export function createMemoryDocument(text: string, uri = "memory:///sample.ik", version = 1): Pick<vscode.TextDocument, "fileName" | "getText" | "uri" | "version"> {
  return {
    fileName: uri.replace("memory://", ""),
    uri: vscode.Uri.parse(uri),
    version,
    getText: () => text
  };
}
```

- [ ] **Step 4: Implement analysis cache and diagnostics conversion**

Continue `src/languageService.ts`:

```ts
const cache = new Map<string, IntKernelAnalysis>();

export function analyzeIntKernelDocument(
  document: Pick<vscode.TextDocument, "fileName" | "getText" | "uri" | "version">,
  options: AnalyzeOptions = {}
): IntKernelAnalysis {
  const cacheKey = `${document.uri.toString()}@${document.version}`;
  const cached = cache.get(cacheKey);
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
    cache.set(cacheKey, analysis);
    return analysis;
  } catch (error) {
    const analysis = {
      document,
      sourceText,
      diagnostics: [unexpectedValidationDiagnostic(error)],
      symbols: [],
      references: []
    };
    cache.set(cacheKey, analysis);
    return analysis;
  }
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
```

- [ ] **Step 5: Implement symbol and reference extraction**

Add the index builder to `src/languageService.ts`:

```ts
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

  walkProgram(checkResult.ast, {
    expression: (expression) => {
      const reference = referenceFromExpression(sourceText, checkResult, symbolsByName, expression);
      if (reference) {
        references.push(reference);
      }
    }
  });

  return { symbols, references };
}

function addStructSymbols(sourceText: string, declaration: StructDeclaration, remember: (symbol: IntKernelSymbol) => void): void {
  const structSymbol = symbolFromNode(sourceText, "struct", declaration.name.name, declaration.name.span, {
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
  remember(symbolFromNode(sourceText, "function", declaration.name.name, declaration.name.span, { signatureLabel, detail: signatureLabel }));
  declaration.params.forEach((param, index) => {
    const type = functionInfo?.params[index]?.type;
    remember(paramSymbolFromNode(sourceText, param, type ? formatTypeLabel(type) : undefined));
  });
  for (const statement of declaration.body.statements) {
    collectLocalSymbols(sourceText, checkResult, statement, remember);
  }
}
```

Add small helper functions in the same file. Keep these helpers private:

```ts
function symbolFromNode(
  sourceText: string,
  kind: IntKernelSymbolKind,
  name: string,
  span: { start: { line: number; column: number }; end: { line: number; column: number } },
  extra: Partial<IntKernelSymbol> = {}
): IntKernelSymbol {
  const range = rangeFromCompilerSpan(sourceText, span);
  return { kind, name, range, selectionRange: range, ...extra };
}

function fieldSymbolFromNode(sourceText: string, containerName: string, field: StructField): IntKernelSymbol {
  const typeLabel = field.type.kind === "PrimitiveType" ? field.type.name : field.type.kind === "NamedType" ? field.type.name.name : undefined;
  return symbolFromNode(sourceText, "field", field.name.name, field.name.span, {
    typeLabel,
    containerName,
    detail: formatSymbolLabel("field", field.name.name, typeLabel, containerName)
  });
}

function paramSymbolFromNode(sourceText: string, param: FunctionParam, typeLabel?: string): IntKernelSymbol {
  return symbolFromNode(sourceText, "parameter", param.name.name, param.name.span, {
    typeLabel,
    detail: formatSymbolLabel("parameter", param.name.name, typeLabel)
  });
}

function collectLocalSymbols(
  sourceText: string,
  checkResult: CheckResult,
  statement: import("intkernel").Statement,
  remember: (symbol: IntKernelSymbol) => void
): void {
  if (statement.kind === "LetStatement") {
    const type = checkResult.checkedProgram.localTypes.get(statement);
    remember(localSymbolFromNode(sourceText, statement, type ? formatTypeLabel(type) : undefined));
  }
  if (statement.kind === "BlockStatement") {
    statement.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, remember));
  }
  if (statement.kind === "IfStatement") {
    statement.thenBlock.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, remember));
    statement.elseBlock?.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, remember));
  }
  if (statement.kind === "WhileStatement") {
    statement.body.statements.forEach((child) => collectLocalSymbols(sourceText, checkResult, child, remember));
  }
}

function localSymbolFromNode(sourceText: string, statement: LetStatement, typeLabel?: string): IntKernelSymbol {
  return symbolFromNode(sourceText, "local", statement.name.name, statement.name.span, {
    typeLabel,
    detail: formatSymbolLabel("local", statement.name.name, typeLabel)
  });
}

function rangeFromCompilerSpan(
  sourceText: string,
  span: { start: { line: number; column: number }; end: { line: number; column: number } }
): vscode.Range {
  const coordinates = spanToRangeCoordinates(sourceText, span);
  return new vscode.Range(coordinates.start.line, coordinates.start.character, coordinates.end.line, coordinates.end.character);
}
```

- [ ] **Step 6: Implement reference resolution**

Add reference helpers to `src/languageService.ts`:

```ts
function referenceFromExpression(
  sourceText: string,
  checkResult: CheckResult,
  symbolsByName: Map<string, IntKernelSymbol[]>,
  expression: Expression
): IntKernelReference | undefined {
  if (expression.kind === "IdentifierExpression") {
    const functionSymbol = checkResult.checkedProgram.functionMap.get(expression.name);
    const target = symbolsByName.get(expression.name)?.find((symbol) => functionSymbol ? symbol.kind === "function" : symbol.kind === "local" || symbol.kind === "parameter");
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
    const objectType = getExprType(checkResult.checkedProgram, expression.object);
    const containerName = objectType?.kind === "struct" ? objectType.name : undefined;
    const target = symbolsByName.get(expression.field.name)?.find((symbol) => symbol.kind === "field" && symbol.containerName === containerName);
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

export function symbolAtPosition(analysis: IntKernelAnalysis, position: vscode.Position): IntKernelSymbol | undefined {
  return analysis.symbols.find((symbol) => symbol.selectionRange.contains(position));
}

export function referenceAtPosition(analysis: IntKernelAnalysis, position: vscode.Position): IntKernelReference | undefined {
  return analysis.references.find((reference) => reference.range.contains(position));
}

export function symbolsInDocument(analysis: IntKernelAnalysis, kind?: IntKernelSymbolKind): readonly IntKernelSymbol[] {
  return kind ? analysis.symbols.filter((symbol) => symbol.kind === kind) : analysis.symbols;
}
```

- [ ] **Step 7: Run language service tests and fix type errors**

Run:

```sh
pnpm test -- test/languageService.test.ts
pnpm compile
```

Expected: test passes and TypeScript compile passes. If type imports need adjustment, keep changes in `languageService.ts` and rerun both commands.

- [ ] **Step 8: Commit Task 2**

Run:

```sh
git add ik-vscode-plugin/src/languageService.ts ik-vscode-plugin/test/languageService.test.ts
git commit -m "feat: add intkernel vscode language service"
```

## Task 3: Diagnostics Refactor To Shared Analysis

**Files:**
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/diagnostics.ts`
- Test: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/languageService.test.ts`

- [ ] **Step 1: Add a regression test for diagnostics conversion**

Append this test to `test/languageService.test.ts`:

```ts
it("converts compiler diagnostics to vscode diagnostics", () => {
  const invalid = `
fn broken() -> i64 {
  let value: i64 = true;
  return value;
}
`.trimStart();
  const analysis = analyzeIntKernelDocument(createMemoryDocument(invalid, "memory:///broken.ik", 1));
  expect(analysis.diagnostics.length).toBeGreaterThan(0);
  expect(analysis.diagnostics[0]?.source).toBe("intkernel");
  expect(analysis.diagnostics.some((diagnostic) => diagnostic.message.includes("Cannot initialize"))).toBe(true);
});
```

- [ ] **Step 2: Run diagnostics conversion test**

Run:

```sh
pnpm test -- test/languageService.test.ts
```

Expected: passes if Task 2 diagnostics conversion is correct.

- [ ] **Step 3: Refactor diagnostics provider**

Replace direct `SourceFile` and `check` imports in `src/diagnostics.ts` with:

```ts
import * as vscode from "vscode";
import { analyzeIntKernelDocument, clearAnalysisCache } from "./languageService";
```

Replace `validateDocument` with:

```ts
function validateDocument(document: vscode.TextDocument, collection: vscode.DiagnosticCollection): void {
  const analysis = analyzeIntKernelDocument(document);
  collection.set(document.uri, [...analysis.diagnostics]);
}
```

Replace the close handler body with:

```ts
clearPending(document, pending);
clearAnalysisCache(document.uri);
collection.delete(document.uri);
```

Remove now-unused `toVscodeDiagnostic`, `unexpectedValidationDiagnostic`, and `errorMessage` from `src/diagnostics.ts`.

- [ ] **Step 4: Verify diagnostics refactor**

Run:

```sh
pnpm test
pnpm compile
```

Expected: all plugin tests pass and TypeScript compile passes.

- [ ] **Step 5: Commit Task 3**

Run:

```sh
git add ik-vscode-plugin/src/diagnostics.ts ik-vscode-plugin/test/languageService.test.ts
git commit -m "refactor: share intkernel diagnostic analysis"
```

## Task 4: Semantic Tokens

**Files:**
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/semanticTokens.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/semanticTokens.test.ts`
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/extension.ts`

- [ ] **Step 1: Write failing semantic token tests**

Create `test/semanticTokens.test.ts`:

```ts
import * as vscode from "vscode";
import { describe, expect, it } from "vitest";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";
import { buildSemanticTokenData, semanticTokenLegend } from "../src/semanticTokens";

const sourceText = `
struct Item {
  price: i64;
}

fn total(item: Item) -> i64 {
  return item.price;
}
`.trimStart();

describe("semanticTokens", () => {
  it("classifies declarations and references", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const tokens = buildSemanticTokenData(analysis);
    const typeIndex = semanticTokenLegend.tokenTypes.indexOf("type");
    const functionIndex = semanticTokenLegend.tokenTypes.indexOf("function");
    const parameterIndex = semanticTokenLegend.tokenTypes.indexOf("parameter");
    const propertyIndex = semanticTokenLegend.tokenTypes.indexOf("property");

    expect(tokens.some((token) => token.tokenType === typeIndex && token.text === "Item")).toBe(true);
    expect(tokens.some((token) => token.tokenType === functionIndex && token.text === "total")).toBe(true);
    expect(tokens.some((token) => token.tokenType === parameterIndex && token.text === "item")).toBe(true);
    expect(tokens.some((token) => token.tokenType === propertyIndex && token.text === "price")).toBe(true);
  });

  it("creates a provider result for a document", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const builder = new vscode.SemanticTokensBuilder(semanticTokenLegend);
    for (const token of buildSemanticTokenData(analysis)) {
      builder.push(token.range, token.tokenType, token.tokenModifiers);
    }
    expect(builder.build().data.length).toBeGreaterThan(0);
  });
});
```

- [ ] **Step 2: Run semantic token tests and verify red**

Run:

```sh
pnpm test -- test/semanticTokens.test.ts
```

Expected: fails with an import error for `../src/semanticTokens`.

- [ ] **Step 3: Implement semantic token module**

Create `src/semanticTokens.ts`:

```ts
import * as vscode from "vscode";
import { analyzeIntKernelDocument, type IntKernelAnalysis, type IntKernelReference, type IntKernelSymbol } from "./languageService";

export const semanticTokenLegend = new vscode.SemanticTokensLegend(
  ["type", "function", "parameter", "variable", "property"],
  ["declaration"]
);

export interface SemanticTokenData {
  text: string;
  range: vscode.Range;
  tokenType: number;
  tokenModifiers: number;
}

const declarationModifier = 1 << semanticTokenLegend.tokenModifiers.indexOf("declaration");

export function buildSemanticTokenData(analysis: IntKernelAnalysis): SemanticTokenData[] {
  const tokens: SemanticTokenData[] = [
    ...analysis.symbols.map(symbolToToken),
    ...analysis.references.map(referenceToToken)
  ];
  return tokens.sort((left, right) => left.range.start.compareTo(right.range.start));
}

export function registerSemanticTokens(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.languages.registerDocumentSemanticTokensProvider(
      { language: "intkernel" },
      {
        provideDocumentSemanticTokens(document) {
          const analysis = analyzeIntKernelDocument(document);
          const builder = new vscode.SemanticTokensBuilder(semanticTokenLegend);
          for (const token of buildSemanticTokenData(analysis)) {
            builder.push(token.range, token.tokenType, token.tokenModifiers);
          }
          return builder.build();
        }
      },
      semanticTokenLegend
    )
  );
}

function symbolToToken(symbol: IntKernelSymbol): SemanticTokenData {
  return {
    text: symbol.name,
    range: symbol.selectionRange,
    tokenType: tokenTypeForKind(symbol.kind),
    tokenModifiers: declarationModifier
  };
}

function referenceToToken(reference: IntKernelReference): SemanticTokenData {
  return {
    text: reference.name,
    range: reference.range,
    tokenType: tokenTypeForKind(reference.kind),
    tokenModifiers: 0
  };
}

function tokenTypeForKind(kind: string): number {
  if (kind === "struct" || kind === "type") return semanticTokenLegend.tokenTypes.indexOf("type");
  if (kind === "function") return semanticTokenLegend.tokenTypes.indexOf("function");
  if (kind === "parameter") return semanticTokenLegend.tokenTypes.indexOf("parameter");
  if (kind === "field") return semanticTokenLegend.tokenTypes.indexOf("property");
  return semanticTokenLegend.tokenTypes.indexOf("variable");
}
```

- [ ] **Step 4: Register semantic tokens in extension**

Modify `src/extension.ts`:

```ts
import { registerSemanticTokens } from "./semanticTokens";
```

Inside `activate`, call:

```ts
registerSemanticTokens(context);
```

- [ ] **Step 5: Verify semantic tokens**

Run:

```sh
pnpm test -- test/semanticTokens.test.ts
pnpm compile
```

Expected: semantic token tests pass and TypeScript compile passes.

- [ ] **Step 6: Commit Task 4**

Run:

```sh
git add ik-vscode-plugin/src/semanticTokens.ts ik-vscode-plugin/test/semanticTokens.test.ts ik-vscode-plugin/src/extension.ts
git commit -m "feat: add intkernel semantic tokens"
```

## Task 5: Hover Provider

**Files:**
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/hover.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/hover.test.ts`
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/extension.ts`

- [ ] **Step 1: Write failing hover tests**

Create `test/hover.test.ts`:

```ts
import * as vscode from "vscode";
import { describe, expect, it } from "vitest";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";
import { getHoverMarkdownAtPosition } from "../src/hover";

const sourceText = `
struct Item {
  price: i64;
}

fn total(item: Item) -> i64 {
  let subtotal: i64 = item.price;
  return subtotal;
}
`.trimStart();

describe("hover", () => {
  it("shows local variable type information", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const markdown = getHoverMarkdownAtPosition(analysis, new vscode.Position(6, 9));
    expect(markdown?.value).toContain("local subtotal: i64");
  });

  it("shows function signature information", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const markdown = getHoverMarkdownAtPosition(analysis, new vscode.Position(4, 3));
    expect(markdown?.value).toContain("fn total(item: Item) -> i64");
  });

  it("shows field type information", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const markdown = getHoverMarkdownAtPosition(analysis, new vscode.Position(6, 28));
    expect(markdown?.value).toContain("field Item.price: i64");
  });
});
```

- [ ] **Step 2: Run hover tests and verify red**

Run:

```sh
pnpm test -- test/hover.test.ts
```

Expected: fails with an import error for `../src/hover`.

- [ ] **Step 3: Implement hover provider**

Create `src/hover.ts`:

```ts
import * as vscode from "vscode";
import { analyzeIntKernelDocument, referenceAtPosition, symbolAtPosition, type IntKernelAnalysis } from "./languageService";

export function getHoverMarkdownAtPosition(analysis: IntKernelAnalysis, position: vscode.Position): vscode.MarkdownString | undefined {
  const symbol = symbolAtPosition(analysis, position);
  if (symbol) {
    return new vscode.MarkdownString(codeBlock(symbol.detail ?? symbol.signatureLabel ?? symbol.name));
  }

  const reference = referenceAtPosition(analysis, position);
  if (reference) {
    const label = reference.target?.detail ?? reference.target?.signatureLabel ?? `${reference.name}${reference.typeLabel ? `: ${reference.typeLabel}` : ""}`;
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
```

- [ ] **Step 4: Register hover in extension**

Modify `src/extension.ts`:

```ts
import { registerHover } from "./hover";
```

Inside `activate`, call:

```ts
registerHover(context);
```

- [ ] **Step 5: Verify hover**

Run:

```sh
pnpm test -- test/hover.test.ts
pnpm compile
```

Expected: hover tests pass and TypeScript compile passes.

- [ ] **Step 6: Commit Task 5**

Run:

```sh
git add ik-vscode-plugin/src/hover.ts ik-vscode-plugin/test/hover.test.ts ik-vscode-plugin/src/extension.ts
git commit -m "feat: add intkernel hover provider"
```

## Task 6: AST-Aware Completions

**Files:**
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/completions.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/completions.test.ts`

- [ ] **Step 1: Write failing completion tests**

Create `test/completions.test.ts`:

```ts
import * as vscode from "vscode";
import { describe, expect, it } from "vitest";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";
import { buildCompletionItems } from "../src/completions";

const sourceText = `
struct Item {
  price: i64;
  qty: i64;
}

fn total(item: Item) -> i64 {
  let subtotal: i64 = item.price;
  return subtotal;
}
`.trimStart();

describe("completions", () => {
  it("includes static keywords and document symbols", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const items = buildCompletionItems(analysis, new vscode.Position(7, 9));
    const labels = items.map((item) => item.label.toString());
    expect(labels).toContain("while");
    expect(labels).toContain("subtotal");
    expect(labels).toContain("total");
    expect(labels).toContain("Item");
  });

  it("suggests struct fields after member access", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const items = buildCompletionItems(analysis, new vscode.Position(6, 30), "item.");
    const labels = items.map((item) => item.label.toString());
    expect(labels).toContain("price");
    expect(labels).toContain("qty");
  });
});
```

- [ ] **Step 2: Run completion tests and verify red**

Run:

```sh
pnpm test -- test/completions.test.ts
```

Expected: fails because `buildCompletionItems` is not exported.

- [ ] **Step 3: Export static and analysis-backed completion builder**

Modify `src/completions.ts` to export:

```ts
export function buildCompletionItems(
  analysis?: import("./languageService").IntKernelAnalysis,
  position?: vscode.Position,
  linePrefix = ""
): vscode.CompletionItem[] {
  const items = [...keywordCompletions(), ...typeCompletions(), ...snippetCompletions()];
  if (!analysis || !position) {
    return items;
  }

  const receiverName = memberReceiverName(linePrefix);
  if (receiverName) {
    const receiver = analysis.references.find((reference) => reference.name === receiverName) ?? analysis.symbols.find((symbol) => symbol.name === receiverName);
    const receiverType = receiver?.typeLabel;
    const structName = receiverType?.startsWith("ptr<") ? receiverType.slice(4, -1) : receiverType;
    return [
      ...items,
      ...analysis.symbols
        .filter((symbol) => symbol.kind === "field" && symbol.containerName === structName)
        .map((symbol) => symbolCompletion(symbol, vscode.CompletionItemKind.Field))
    ];
  }

  return [
    ...items,
    ...analysis.symbols
      .filter((symbol) => symbol.kind === "local" || symbol.kind === "parameter")
      .map((symbol) => symbolCompletion(symbol, vscode.CompletionItemKind.Variable)),
    ...analysis.symbols
      .filter((symbol) => symbol.kind === "function")
      .map((symbol) => symbolCompletion(symbol, vscode.CompletionItemKind.Function)),
    ...analysis.symbols
      .filter((symbol) => symbol.kind === "struct")
      .map((symbol) => symbolCompletion(symbol, vscode.CompletionItemKind.Struct))
  ];
}

function symbolCompletion(symbol: import("./languageService").IntKernelSymbol, kind: vscode.CompletionItemKind): vscode.CompletionItem {
  const item = new vscode.CompletionItem(symbol.name, kind);
  item.detail = symbol.detail ?? symbol.signatureLabel;
  item.sortText = `3_${symbol.kind}_${symbol.name}`;
  return item;
}

function memberReceiverName(linePrefix: string): string | undefined {
  const match = /([A-Za-z_][A-Za-z0-9_]*)\.$/.exec(linePrefix);
  return match?.[1];
}
```

- [ ] **Step 4: Wire provider to analysis**

Modify `registerCompletions` in `src/completions.ts`:

```ts
import { analyzeIntKernelDocument } from "./languageService";
```

Use:

```ts
provideCompletionItems: (document, position) => {
  const linePrefix = document.lineAt(position).text.slice(0, position.character);
  return buildCompletionItems(analyzeIntKernelDocument(document), position, linePrefix);
}
```

- [ ] **Step 5: Verify completions**

Run:

```sh
pnpm test -- test/completions.test.ts
pnpm compile
```

Expected: completion tests pass and TypeScript compile passes.

- [ ] **Step 6: Commit Task 6**

Run:

```sh
git add ik-vscode-plugin/src/completions.ts ik-vscode-plugin/test/completions.test.ts
git commit -m "feat: add intkernel ast aware completions"
```

## Task 7: Definitions And Document Symbols

**Files:**
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/definitions.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/documentSymbols.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/definitions.test.ts`
- Create: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/test/documentSymbols.test.ts`
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/extension.ts`

- [ ] **Step 1: Write failing definition tests**

Create `test/definitions.test.ts`:

```ts
import * as vscode from "vscode";
import { describe, expect, it } from "vitest";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";
import { getDefinitionAtPosition } from "../src/definitions";

const sourceText = `
struct Item {
  price: i64;
}

fn total(item: Item) -> i64 {
  let subtotal: i64 = item.price;
  return subtotal;
}
`.trimStart();

describe("definitions", () => {
  it("resolves local variable references", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const location = getDefinitionAtPosition(analysis, new vscode.Position(7, 9));
    expect(location?.range.start.line).toBe(6);
  });

  it("resolves field references", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const location = getDefinitionAtPosition(analysis, new vscode.Position(6, 28));
    expect(location?.range.start.line).toBe(1);
  });
});
```

- [ ] **Step 2: Write failing document symbol tests**

Create `test/documentSymbols.test.ts`:

```ts
import { describe, expect, it } from "vitest";
import { analyzeIntKernelDocument, createMemoryDocument } from "../src/languageService";
import { buildDocumentSymbols } from "../src/documentSymbols";

const sourceText = `
struct Item {
  price: i64;
}

fn total(item: Item) -> i64 {
  let subtotal: i64 = item.price;
  return subtotal;
}
`.trimStart();

describe("documentSymbols", () => {
  it("builds outline entries for structs, fields, functions, params, and locals", () => {
    const analysis = analyzeIntKernelDocument(createMemoryDocument(sourceText));
    const symbols = buildDocumentSymbols(analysis);
    expect(symbols.map((symbol) => symbol.name)).toEqual(["Item", "total"]);
    expect(symbols[0]?.children.map((symbol) => symbol.name)).toEqual(["price"]);
    expect(symbols[1]?.children.map((symbol) => symbol.name)).toEqual(["item", "subtotal"]);
  });
});
```

- [ ] **Step 3: Run definition and document symbol tests and verify red**

Run:

```sh
pnpm test -- test/definitions.test.ts test/documentSymbols.test.ts
```

Expected: fails with import errors for `../src/definitions` and `../src/documentSymbols`.

- [ ] **Step 4: Implement definitions**

Create `src/definitions.ts`:

```ts
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
```

- [ ] **Step 5: Implement document symbols**

Create `src/documentSymbols.ts`:

```ts
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
        .filter((child) => child.kind === "parameter" || child.kind === "local")
        .map((child) => toDocumentSymbol(child, child.kind === "parameter" ? vscode.SymbolKind.Variable : vscode.SymbolKind.Variable));
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
```

- [ ] **Step 6: Register definitions and document symbols**

Modify `src/extension.ts`:

```ts
import { registerDefinitions } from "./definitions";
import { registerDocumentSymbols } from "./documentSymbols";
```

Inside `activate`, call:

```ts
registerDefinitions(context);
registerDocumentSymbols(context);
```

- [ ] **Step 7: Verify definitions and document symbols**

Run:

```sh
pnpm test -- test/definitions.test.ts test/documentSymbols.test.ts
pnpm compile
```

Expected: tests pass and TypeScript compile passes.

- [ ] **Step 8: Commit Task 7**

Run:

```sh
git add ik-vscode-plugin/src/definitions.ts ik-vscode-plugin/src/documentSymbols.ts ik-vscode-plugin/test/definitions.test.ts ik-vscode-plugin/test/documentSymbols.test.ts ik-vscode-plugin/src/extension.ts
git commit -m "feat: add intkernel navigation providers"
```

## Task 8: Activation, Documentation, Packaging, And Manual Verification

**Files:**
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/src/extension.ts`
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/README.md`
- Modify: `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/CHANGELOG.md`

- [ ] **Step 1: Check extension activation registers every provider**

Open `src/extension.ts` and confirm `activate` calls:

```ts
registerDiagnostics(context);
registerCompletions(context);
registerSemanticTokens(context);
registerHover(context);
registerDefinitions(context);
registerDocumentSymbols(context);
```

If any call is missing, add it.

- [ ] **Step 2: Update README feature list**

In `README.md`, replace the feature list with:

```md
- `.ik` file recognition as the `intkernel` language.
- TextMate syntax highlighting for IntKernel keywords, types, comments, numbers, declarations, variables, and fields.
- Semantic highlighting backed by IntKernel compiler analysis for structs, functions, parameters, locals, fields, and references.
- Language configuration for `//` comments and auto-closing braces, square brackets, and parentheses.
- Snippets for common declarations and statements.
- Basic keyword, primitive type, pointer type, and declaration completions.
- AST-aware completions for document-local symbols and struct fields.
- Hover information for variable types, field types, function signatures, and struct names.
- Go to Definition for functions, structs, fields, parameters, and locals.
- Document symbols / Outline for structs, fields, functions, parameters, and locals.
- Compiler-backed diagnostics using the local `intkernel` package.
```

- [ ] **Step 3: Update README manual verification**

In `README.md`, replace the manual verification list with:

```md
1. Open `/Users/lynn/code/IntKernel/demo/dijkstra.ik`.
2. Confirm the language mode is `IntKernel`.
3. Confirm semantic highlighting distinguishes variables, fields, parameters, functions, and types.
4. Hover `settled_count`, `configs`, `DijkstraConfig`, `matrix_index`, and `node_count`.
5. Type `configs[0].` and confirm field completions include `node_count`, `source`, and `inf`.
6. Use Go to Definition on `matrix_index`, `DijkstraConfig`, `settled_count`, and `node_count`.
7. Confirm Outline displays `DijkstraConfig`, `matrix_index`, `is_unvisited`, `should_relax`, and `dijkstra_matrix`.
8. Introduce a type error such as assigning `true` to an `i64`.
9. Confirm the Problems panel shows an `intkernel` diagnostic.
10. Revert the type error and confirm the diagnostic clears.
```

- [ ] **Step 4: Update CHANGELOG**

Add this entry above `0.1.0`:

```md
## Unreleased

- Add compiler-aware semantic highlighting.
- Add hover information for IntKernel symbols.
- Add AST-aware completions for document-local symbols and struct fields.
- Add Go to Definition support for local symbols, functions, structs, and fields.
- Add document symbols / Outline support.
- Share compiler analysis across diagnostics and editor providers.
```

If an `Unreleased` section already exists, merge these bullets into that section.

- [ ] **Step 5: Run full automated verification**

Run:

```sh
pnpm test
pnpm compile
pnpm package
```

Expected:

- `pnpm test`: all plugin tests pass.
- `pnpm compile`: TypeScript and esbuild pass.
- `pnpm package`: VSIX is generated at `ik-vscode-plugin/ik-vscode-plugin-0.1.0.vsix`; repository/LICENSE warnings may remain.

- [ ] **Step 6: Install VSIX into default VSCode**

Run:

```sh
'/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code' --install-extension /Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin/ik-vscode-plugin-0.1.0.vsix --force
'/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code' --list-extensions --show-versions | rg '^local\\.ik-vscode-plugin@'
```

Expected: `local.ik-vscode-plugin@0.1.0`.

- [ ] **Step 7: Perform manual VSCode verification**

Open the worktree plugin folder or installed extension in VSCode and use `/Users/lynn/code/IntKernel/demo/dijkstra.ik` for visual checks:

```sh
'/Applications/Visual Studio Code.app/Contents/Resources/app/bin/code' /Users/lynn/code/IntKernel/demo/dijkstra.ik
```

Manual expected results:

- semantic highlighting changes variables, fields, parameters, functions, and types;
- hover returns type/signature text;
- `configs[0].` suggests `node_count`, `source`, and `inf`;
- Go to Definition jumps to local, parameter, function, struct, and field declarations;
- Outline shows structs and functions;
- diagnostics still appear and clear.

- [ ] **Step 8: Commit Task 8**

Run:

```sh
git add ik-vscode-plugin/src/extension.ts ik-vscode-plugin/README.md ik-vscode-plugin/CHANGELOG.md
git commit -m "docs: document intkernel vscode v2b"
```

## Final Verification

After all tasks are complete, run from `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b/ik-vscode-plugin`:

```sh
pnpm test
pnpm compile
pnpm package
```

Expected: all commands exit 0. `pnpm package` may still warn about missing repository and LICENSE metadata.

Run from `/Users/lynn/.config/superpowers/worktrees/IntKernel/ik-vscode-v2-b`:

```sh
git status --short --branch --ignored=matching
```

Expected: tracked worktree is clean after commits; ignored entries may include `build/`, `dist/`, `node_modules/`, `ik-vscode-plugin/dist/`, `ik-vscode-plugin/node_modules/`, and `ik-vscode-plugin/ik-vscode-plugin-0.1.0.vsix`.

## Execution Notes

- Prefer Subagent-Driven execution because each provider can be implemented and reviewed independently.
- Keep commits small and exactly aligned with tasks.
- If a compiler API gap blocks field/type/reference resolution, stop and report the missing API before editing files outside `ik-vscode-plugin`.
- Manual verification must mention any VSCode UI checks that could not be completed.
