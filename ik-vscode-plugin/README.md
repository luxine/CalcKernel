# IntKernel for Visual Studio Code

IntKernel for Visual Studio Code adds editor support for `.ik` source files. It combines TextMate grammar highlighting with compiler-aware language features, so IntKernel programs are easier to read, navigate, and maintain inside VS Code.

## Features

- Automatic language detection for `.ik` files.
- Syntax highlighting for IntKernel keywords, declarations, primitive types, pointer types, comments, booleans, numbers, operators, variables, and fields.
- Semantic highlighting for structs, functions, parameters, local variables, fields, and references.
- Compiler-backed diagnostics from the local IntKernel compiler package.
- Hover information for variable types, field types, function signatures, and struct declarations.
- Completion suggestions for keywords, snippets, primitive types, pointer types, declarations, document symbols, and struct fields.
- Field completion after direct and indexed receivers, such as `config.` and `configs[0].`.
- Go to Definition for structs, functions, fields, parameters, and local variables.
- Document symbols and Outline support for source navigation.
- Language configuration for line comments, bracket pairing, and auto-closing delimiters.

## Installation

### Install from a VSIX

If you received a packaged `.vsix` file, install it from the command line:

```sh
code --install-extension ik-vscode-plugin-0.1.0.vsix
```

Or install it inside VS Code:

1. Open the Extensions view.
2. Select `...` in the Extensions view title bar.
3. Choose `Install from VSIX...`.
4. Select the packaged `ik-vscode-plugin-0.1.0.vsix` file.

### Install from Marketplace

After the extension is published, search for `IntKernel` in the VS Code Extensions view and install it like any other Marketplace extension.

## Getting Started

Open any `.ik` file in VS Code. The editor should select the `IntKernel` language mode automatically.

Useful checks after installation:

- Open an IntKernel source file and confirm keywords, types, variables, and fields are highlighted.
- Hover over a symbol to inspect its type or signature.
- Type after a struct value, for example `config.`, to see field completions.
- Use `Go to Definition` on functions, structs, fields, parameters, or locals.
- Open the Outline view to navigate top-level declarations and nested symbols.
- Introduce a type error and confirm the Problems panel reports an `intkernel` diagnostic.

## Requirements

- Visual Studio Code `1.90.0` or newer.
- No external language server is required. The extension bundles its VS Code integration and uses the local IntKernel compiler package during build.

## Known Limitations

- Language features are based on the current IntKernel compiler API and may not cover syntax that the compiler does not yet expose.
- Semantic colors depend on the active VS Code color theme. Some themes display variables, fields, and parameters more distinctly than others.
- This version focuses on single-file editor intelligence. Cross-file project indexing is planned for a later release.

## Troubleshooting

If `.ik` files are not detected automatically, use the language selector in the lower-right corner of VS Code and choose `IntKernel`.

If semantic colors look too subtle, try a theme with strong semantic token support or enable semantic highlighting in VS Code settings:

```json
{
  "editor.semanticHighlighting.enabled": true
}
```

If diagnostics appear stale after editing, save the file or reload the VS Code window. Diagnostics are recomputed from the extension's compiler-backed analysis cache.

## Release Notes

Release notes are included with each published version.

## License

MIT
