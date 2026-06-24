# Changelog

## Unreleased

- Improve TextMate highlighting for local variables, parameters, struct fields, member access, function calls, and type references.
- Add compiler-aware semantic highlighting.
- Add hover information for IntKernel symbols.
- Add AST-aware completions for document-local symbols and struct fields.
- Add Go to Definition support for local symbols, functions, structs, and fields.
- Add document symbols / Outline support.
- Share compiler analysis across diagnostics and editor providers.

## 0.1.0

- Add `.ik` language registration.
- Add TextMate syntax highlighting.
- Add snippets for common IntKernel constructs.
- Add static keyword, type, and template completions.
- Add compiler-backed diagnostics through the local `intkernel` package.

## Verification

- Automated tests pass with `pnpm test`.
- Extension bundle builds with `pnpm compile`.
- VSIX package builds with `pnpm package`.
- Current IntKernel examples pass compiler diagnostics smoke verification.
