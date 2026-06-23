# IntKernel VSCode Extension

VSCode language support for IntKernel `.ik` files.

## Features

- `.ik` file recognition as the `intkernel` language.
- TextMate syntax highlighting for IntKernel keywords, types, comments, numbers, function declarations, and struct declarations.
- Language configuration for `//` comments and auto-closing braces, square brackets, and parentheses.
- Snippets for common declarations and statements.
- Basic keyword, primitive type, pointer type, and declaration completions.
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

1. Open `/Users/lynn/code/IntKernel/examples/pricing.ik`.
2. Confirm the language mode is `IntKernel`.
3. Confirm syntax highlighting is visible.
4. Confirm snippets and completions appear in a `.ik` file.
5. Introduce a type error such as assigning `true` to an `i64`.
6. Confirm the Problems panel shows an `intkernel` diagnostic.
7. Revert the type error and confirm the diagnostic clears.
