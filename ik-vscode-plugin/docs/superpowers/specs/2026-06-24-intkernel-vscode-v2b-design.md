# IntKernel VSCode Plugin V2-B Design

## Summary

V2-B upgrades the IntKernel VSCode plugin from static editor support plus diagnostics into a compiler-aware extension. It still does not introduce a Language Server Protocol process. Instead, the extension uses the public `intkernel` package APIs directly inside VSCode and shares one checked document model across diagnostics, hover, semantic tokens, completions, definitions, and document symbols.

## Goals

- Keep all plugin work under `/Users/lynn/code/IntKernel/ik-vscode-plugin`.
- Keep the extension architecture simple: VSCode Extension API plus bundled `intkernel`.
- Reuse the compiler front end: `SourceFile`, `check`, AST nodes, spans, checked symbol/type helpers.
- Provide editor features that are hard to implement correctly with TextMate regex alone:
  - semantic tokens for declarations and references
  - hover type/signature information
  - AST-aware completions
  - go to definition
  - document symbols / outline
- Avoid repeated compiler work by sharing a cached document analysis result.

## Non-Goals

- Do not build an LSP server in V2-B.
- Do not support cross-editor integration outside VSCode.
- Do not add compiler language features.
- Do not change IntKernel runtime semantics or generated code behavior.
- Do not publish the extension to Marketplace in this iteration.
- Do not implement full project-wide indexing; V2-B is document-local.

## Current Baseline

The V1 plugin already provides:

- `.ik` language registration.
- TextMate grammar and language configuration.
- snippets and static completions.
- compiler-backed diagnostics via `SourceFile` and `check`.
- grammar tests and diagnostics mapping tests.

The `intkernel` package currently exports enough public API for V2-B:

- `SourceFile`
- `parse`
- `check`
- parser AST types
- `CheckedProgram`, `CheckResult`
- helper accessors such as `getExprType`, `getFunctionInfo`, `getStructInfo`, and `getFieldInfo`

## Architecture

### Shared Analysis Layer

Create `src/languageService.ts` as the central analysis module.

Responsibilities:

- Accept a `vscode.TextDocument`.
- Build a `SourceFile` from `document.fileName` and `document.getText()`.
- Run `check(sourceFile)`.
- Convert compiler spans to VSCode ranges through the existing diagnostic mapping utilities.
- Cache the latest analysis per document URI and version.
- Build editor-facing indexes:
  - declarations by source span
  - references by source span
  - symbols by name and category
  - expression/type information
  - function signatures
  - struct fields
  - document symbols

The cache key is:

```ts
type AnalysisCacheKey = {
  uri: string;
  version: number;
};
```

Each provider asks the language service for the current document analysis. If the document version is unchanged, the provider reuses the cached result. If the document changed, it rechecks the document.

### Analysis Result Shape

The service should expose a small, stable interface rather than leaking the entire compiler object graph to every provider.

```ts
export interface IntKernelAnalysis {
  document: vscode.TextDocument;
  sourceText: string;
  checkResult: CheckResult;
  diagnostics: readonly vscode.Diagnostic[];
  symbols: readonly IntKernelSymbol[];
  references: readonly IntKernelReference[];
}

export interface IntKernelSymbol {
  kind: "struct" | "field" | "function" | "parameter" | "local";
  name: string;
  typeLabel?: string;
  signatureLabel?: string;
  range: vscode.Range;
  selectionRange: vscode.Range;
  detail?: string;
  containerName?: string;
}

export interface IntKernelReference {
  kind: "field" | "function" | "parameter" | "local" | "type";
  name: string;
  range: vscode.Range;
  target?: IntKernelSymbol;
  typeLabel?: string;
}
```

This keeps provider code independent from compiler internals. If the compiler AST changes later, most changes stay in `languageService.ts`.

## Providers

### Diagnostics

Refactor `src/diagnostics.ts` so it uses `getAnalysis(document)` from `languageService.ts` instead of calling `check()` directly. Existing debounce, close cleanup, and diagnostic collection behavior should remain.

Expected behavior:

- diagnostics update on open, save, and debounced changes.
- diagnostics clear when a file closes.
- unexpected compiler exceptions still become one fallback diagnostic at the top of the file.

### Semantic Tokens

Create `src/semanticTokens.ts`.

Register `vscode.languages.registerDocumentSemanticTokensProvider` for `intkernel`.

Token types:

- `struct`
- `type`
- `function`
- `parameter`
- `variable`
- `property`

Token modifiers:

- `declaration`

Mapping:

- struct declarations and named type references -> `type`
- function declarations and calls -> `function`
- function parameters -> `parameter`
- `let` declarations and local references -> `variable`
- struct fields and member access -> `property`

TextMate grammar remains as a fallback. Semantic tokens should be the authoritative highlighting layer when VSCode semantic highlighting is enabled.

### Hover

Create `src/hover.ts`.

Register a hover provider for `intkernel`.

Hover content examples:

```text
parameter items: ptr<Item>
local subtotal: i64
field Item.price: i64
fn matrix_index(row: i32, col: i32, node_count: i32) -> i32
struct DijkstraConfig
```

If the cursor is on a diagnostic range, the normal Problems UI already shows the diagnostic; the hover provider should focus on symbol/type information.

### AST-Aware Completions

Refactor `src/completions.ts` into static plus analysis-backed completions.

Keep existing keyword/type/snippet completions.

Add:

- local variables and parameters in the current function.
- function names from the document.
- struct names from the document.
- field names after member access where the receiver type is known.

Field completion is the highest-value case:

```ik
items[i].
```

If `items[i]` has type `Item`, the provider should suggest `price`, `qty`, `discount`, and `tax_rate_ppm`.

V2-B only needs document-local symbols. It does not need cross-file imports because IntKernel does not currently have an import/module system.

### Go To Definition

Create `src/definitions.ts`.

Register `vscode.languages.registerDefinitionProvider`.

Definition targets:

- function calls -> function declaration
- named type references -> struct declaration
- field access -> struct field declaration when receiver type is known
- local variable references -> `let` declaration
- parameter references -> function parameter declaration

If the service cannot resolve a target confidently, return no definition rather than guessing.

### Document Symbols

Create `src/documentSymbols.ts`.

Register `vscode.languages.registerDocumentSymbolProvider`.

Outline shape:

- struct
  - fields
- function
  - parameters
  - top-level local declarations where useful

This helps users inspect larger `.ik` files like algorithm demos.

## Traversal Strategy

Create `src/astTraversal.ts` for shared AST walking helpers.

The traversal layer should expose focused functions such as:

```ts
walkProgram(program, visitor)
rangeFromSpan(sourceText, span)
containsPosition(range, position)
```

Providers should not each implement their own recursive AST traversal.

## Scope Resolution

V2-B should support enough scope resolution for current IntKernel:

- function parameters are in function scope.
- `let` declarations are visible after declaration within their block.
- nested blocks can shadow outer names if the compiler permits it.
- field references are resolved by type, not by name alone.

If exact block scope proves too large for the first pass, V2-B may start with function-scope locals and then refine block scoping in a follow-up task. The implementation plan must make this decision explicit before coding.

## Error Handling

If `check()` returns diagnostics but still returns an AST, providers may still use partial symbols for semantic tokens and outline.

If `check()` throws unexpectedly:

- diagnostics show one fallback diagnostic.
- hover/completion/definition/semantic tokens return empty results.
- the exception must not break the extension host.

## Tests

Keep tests in `ik-vscode-plugin/test`.

Add unit tests for:

- span/range conversion reuse where needed.
- AST traversal over representative `.ik` snippets.
- language service symbol extraction.
- hover text formatting.
- semantic token classification.
- definition lookup for locals, params, functions, structs, and fields.
- completion item construction for local and field completions.

Run:

```sh
pnpm test
pnpm compile
pnpm package
```

Manual verification:

1. Install the generated VSIX.
2. Open `/Users/lynn/code/IntKernel/demo/dijkstra.ik`.
3. Confirm semantic highlighting distinguishes variables, fields, parameters, functions, and types.
4. Hover representative symbols and confirm type/signature output.
5. Type `configs[0].` and confirm struct field completions.
6. Use Go to Definition on function calls, struct type names, local variables, parameters, and fields.
7. Confirm Outline displays structs, fields, and functions.
8. Introduce a type error and confirm diagnostics still update and clear.

## Documentation

Update:

- `README.md` feature list and manual verification.
- `CHANGELOG.md` with V2-B feature set.

No root repository source files should be changed unless a compiler API gap blocks V2-B. If that happens, stop and document the exact missing public API before editing compiler source.

## Risks

- AST/type helper APIs may not expose every reference target directly; the language service may need careful AST traversal and local indexing.
- Semantic tokens can conflict with TextMate theme colors. This is acceptable because VSCode semantic highlighting is designed to override TextMate where available.
- Field completion needs receiver type resolution. If expression type maps are not enough for every case, implement the reliable cases first and avoid speculative completions.
- Rechecking on every provider call can be expensive. The cache must be version-based and shared.

## Acceptance Criteria

- Existing V1 diagnostics behavior still works.
- `pnpm test`, `pnpm compile`, and `pnpm package` pass.
- VSIX installs locally.
- Semantic highlighting is AST/type aware for representative examples.
- Hover, document-local completion, definition lookup, and outline work on `demo/dijkstra.ik`.
- All implementation remains in `ik-vscode-plugin` unless a documented compiler API gap is approved separately.
