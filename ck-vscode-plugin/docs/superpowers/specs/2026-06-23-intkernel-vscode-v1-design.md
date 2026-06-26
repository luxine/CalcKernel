# IntKernel VSCode Extension V1 Design

## Goal

Build this VSCode extension project at
`/Users/lynn/code/IntKernel/ik-vscode-plugin` to provide first-pass editor
support for `.ik` files.

V1 should make IntKernel source files pleasant to read and catch real language
errors in VSCode without introducing a Language Server Protocol implementation.
The extension must reuse the existing IntKernel compiler package instead of
duplicating lexer, parser, or type-checker logic.

## Scope

V1 includes:

- `.ik` language registration.
- TextMate syntax highlighting.
- VSCode language configuration for comments, brackets, and auto-closing pairs
  for `{}`, `[]`, and `()` only.
- Snippets for common IntKernel declarations and statements.
- Basic completion items for keywords, primitive types, and common templates.
- Diagnostics for the active workspace by calling IntKernel `SourceFile` and
  `check()`, then mapping compiler diagnostics to VSCode Problems.

V1 does not include:

- LSP client/server.
- Hover type display.
- go-to-definition.
- rename.
- workspace symbol index.
- semantic tokens.
- formatter.
- struct-field completion after `.`.
- function signature help.

These are V2 language-service features.

## Project Layout

The V1 project layout is:

```text
/Users/lynn/code/IntKernel/ik-vscode-plugin/
  package.json
  tsconfig.json
  esbuild.mjs
  README.md
  CHANGELOG.md
  language-configuration.json
  syntaxes/
    intkernel.tmLanguage.json
  snippets/
    intkernel.json
  src/
    extension.ts
    diagnostics.ts
    diagnosticMapping.ts
    completions.ts
  test/
    diagnosticMapping.test.ts
  docs/
    superpowers/
      specs/
        2026-06-23-intkernel-vscode-v1-design.md
```

The extension project depends on the local compiler package with:

```json
"intkernel": "file:.."
```

The extension bundle should be produced by `esbuild`, with `vscode` marked as
external and the local `intkernel` dependency bundled into the extension output.
This avoids depending on a sibling checkout at runtime after packaging.

## VSCode Contributions

`package.json` contributes:

- language id: `intkernel`
- aliases: `IntKernel`, `ik`
- extensions: `.ik`
- grammar path: `./syntaxes/intkernel.tmLanguage.json`
- language configuration path: `./language-configuration.json`
- snippets path: `./snippets/intkernel.json`
- activation events:
  - `onLanguage:intkernel`

The extension entrypoint is the bundled output, for example
`dist/extension.cjs`.

## Syntax Highlighting

Use a TextMate grammar as the first implementation. It should cover:

- line comments: `// ...`
- keywords: `struct`, `export`, `fn`, `let`, `return`, `if`, `else`, `while`
- primitive types: `i32`, `i64`, `u32`, `u64`, `bool`
- pointer type constructor: `ptr`
- boolean literals: `true`, `false`
- integer literals
- function declaration names after `fn`
- struct declaration names after `struct`
- operators and punctuation

The grammar does not need to understand nested type structure beyond common
patterns like `ptr<Item>`.

## Snippets

Provide snippets for:

- `struct`
- `fn`
- `export fn`
- `let`
- `if`
- `if else`
- `while`
- `return`
- `ptr`
- `pricing item loop` as an optional longer example only if it does not clutter
  completion results.

Snippet bodies should follow the style used in the IntKernel README examples:
explicit types, semicolons, and compact control-flow blocks.

## Basic Completions

Register a `CompletionItemProvider` for `intkernel` documents.

V1 completions are static and context-light:

- keywords
- primitive types
- `ptr<T>` template
- declaration templates that mirror snippets

The provider should not try to parse the document for symbol-aware completions.
That avoids fragile partial-AST behavior in V1.
`ptr<T>` insertion is covered by snippets and completions, so `<` remains
comfortable as a comparison operator.

## Diagnostics

Diagnostics use the compiler as the source of truth:

1. On activation, create one `DiagnosticCollection` named `intkernel`.
2. Validate all open `.ik` documents.
3. Validate a document on open and on save.
4. Validate after content changes with a short debounce, around 250 ms.
5. Clear diagnostics when an `.ik` document closes.
6. For each validation, construct:

```ts
new SourceFile(document.fileName, document.getText())
```

7. Call `check(sourceFile)`.
8. Map every IntKernel diagnostic to `vscode.Diagnostic`.

The VSCode diagnostic should include:

- range mapped from `diagnostic.span`
- severity `Error`
- message from `diagnostic.message`
- code from `diagnostic.code`
- source `intkernel`

The extension should catch unexpected exceptions from validation and report a
single document-level diagnostic instead of crashing the extension host.

## Diagnostic Position Mapping

IntKernel diagnostics are 1-based line and column values with source spans.
VSCode ranges are 0-based.

Mapping rules:

- start line: `span.start.line - 1`
- start character: `span.start.column - 1`
- end line: `span.end.line - 1`
- end character: `span.end.column - 1`
- clamp all positions to document boundaries
- if the resulting range is empty, expand it by one character when possible

Tests should cover same-line spans, empty spans, multiline spans, end-of-line
clamping, and diagnostics at EOF.

## Commands

V1 does not need user-facing commands. Diagnostics and completions are enough.

A future version may add:

- `IntKernel: Check File`
- `IntKernel: Emit C`
- `IntKernel: Emit WAT`

Those commands are intentionally out of scope because they involve output paths,
build artifacts, and workflow choices beyond editor assistance.

## Build And Package

Use pnpm, TypeScript, and esbuild.

Expected scripts:

```json
"compile": "tsc -p . --noEmit && node esbuild.mjs",
"watch": "tsc -p . --noEmit --watch",
"test": "vitest run",
"package": "vsce package"
```

Use CommonJS output for the VSCode extension bundle unless VSCode packaging in
the local environment clearly supports ESM extension entrypoints without extra
friction.

## Testing

V1 verification should include:

- `pnpm install`
- `pnpm compile`
- `pnpm test`
- a smoke test that imports the bundled extension files successfully
- a diagnostic mapping unit test
- manual VSCode Extension Development Host check:
  - open a valid `.ik` file and confirm no Problems
  - introduce a type error and confirm a compiler diagnostic appears
  - confirm snippets and completions appear in a `.ik` file
  - confirm syntax highlighting applies to README-style examples

Because this is an editor extension, the manual Extension Development Host check
is part of V1 acceptance.

## Risks

Bundling the local `intkernel` package may fail if the package exports or ESM
shape changes. The implementation should keep the import path public:

```ts
import { SourceFile, check } from "intkernel";
```

Do not import from `../IntKernel/src/...`.

The current compiler checks one source file at a time. V1 diagnostics should
therefore remain single-file. If the language later adds imports or modules, the
extension should move diagnostics into a language service or LSP server.

TextMate highlighting is regex-based. It will not perfectly understand nested
syntax, but it is sufficient for V1 because IntKernel grammar is deliberately
small.

## Acceptance Criteria

V1 is complete when:

- `/Users/lynn/code/IntKernel/ik-vscode-plugin` exists as an independent extension project.
- `.ik` files are recognized by VSCode as IntKernel files.
- syntax highlighting works for all current examples in `/Users/lynn/code/IntKernel/examples`.
- snippets and basic completions are available in `.ik` files.
- compiler diagnostics appear in Problems for invalid `.ik` files.
- valid current examples do not produce diagnostics.
- `pnpm compile` and `pnpm test` pass in the extension project.
- implementation does not modify IntKernel compiler source code.
