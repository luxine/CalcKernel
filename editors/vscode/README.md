# CalcKernel for Visual Studio Code

[简体中文](README.zh-CN.md)

This extension recognizes CK / CalcKernel `.ck` files. It provides syntax highlighting, snippets, and live diagnostics from the current Rust `ckc` compiler. The language server works on unsaved edits.

Language features include completion, hover, signature help, go to definition, references, safe rename, document and workspace symbols, semantic highlighting, folding, selection ranges, and document formatting. Navigation and rename follow bindings in one CK source file; CK has no imports. Formatting follows the editor's indentation settings, preserves line comments, and leaves incomplete source unchanged.

## Install

Install a platform-specific VSIX for your operating system and CPU architecture. The package includes a frontend-only `ckc` language server, so highlighting and diagnostics work offline without LLVM. A manually configured server must be version 0.14.x.

## Settings and commands

- `ck.server.path`: absolute path to a compatible `ckc` for language features. Empty uses the bundled binary, then PATH.
- `ck.compiler.path`: absolute path to a Native-enabled `ckc` for Run and Build. Empty uses PATH.
- `CK: Check Current File`, `CK: Run Current File`, `CK: Build Current File`, `CK: Restart Language Server`, and `CK: Show Output` appear in the Command Palette.

Run and Build require a file-backed `.ck` document and save it before invoking the compiler. The language server uses unsaved text for diagnostics. Virtual workspaces retain static highlighting and snippets; compiler commands require local files.

The extension does not download compilers or build Rust code at activation time.
