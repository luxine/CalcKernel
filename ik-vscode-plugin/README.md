# IntKernel VSCode Extension

VSCode language support for IntKernel `.ik` files.

## Features

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

## Development

Install dependencies:

```sh
pnpm install
```

Compile:

```sh
pnpm compile
```

Run tests:

```sh
pnpm test
```

Package:

```sh
pnpm package
```

The package script compiles and bundles the extension into `dist/extension.cjs`, then creates the VSIX while skipping dependency scanning because runtime dependencies are already bundled.

## Manual Verification

Open this folder in VSCode and launch an Extension Development Host.

Then:

1. Open `/Users/lynn/code/IntKernel/examples/dijkstra.ik`.
2. Confirm the language mode is `IntKernel`.
3. Confirm semantic highlighting distinguishes variables, fields, parameters, functions, and types.
4. Hover `settled_count`, `configs`, `DijkstraConfig`, `matrix_index`, and `node_count`.
5. Type `configs[0].` and confirm field completions include `node_count`, `source`, and `inf`.
6. Use Go to Definition on `matrix_index`, `DijkstraConfig`, `settled_count`, and `node_count`.
7. Confirm Outline displays `DijkstraConfig`, `matrix_index`, `is_unvisited`, `should_relax`, and `dijkstra_matrix`.
8. Introduce a type error such as assigning `true` to an `i64`.
9. Confirm the Problems panel shows an `intkernel` diagnostic.
10. Revert the type error and confirm the diagnostic clears.
