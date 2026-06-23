# IntKernel VSCode Variable Highlighting Design

## Goal

Improve `.ik` syntax highlighting so common variables and fields are visually distinct in VSCode without introducing an LSP or semantic-token provider.

## Approach

Use TextMate grammar heuristics only. Add scoped regex rules for:

- local variable declarations after `let`
- function parameter names before `:`
- struct field names at the beginning of field declaration lines
- member access names after `.`
- capitalized type references such as `Item`
- function-call identifiers before `(`
- lowercase variable references that are not keywords, primitive types, `ptr`, or booleans

## Trade-Offs

This is intentionally less precise than semantic tokens because TextMate does not parse the IntKernel AST. The broad identifier fallback may color some expression identifiers conservatively, but specific rules for keywords, primitive types, declarations, fields, and calls run first.

## Validation

Add a Vitest grammar test that reads `syntaxes/intkernel.tmLanguage.json` and verifies the new scoped regex rules match representative IntKernel snippets. Existing diagnostics tests, compile, package, and local VSIX installation remain the final verification path.
