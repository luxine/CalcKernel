# CalcKernel in Visual Studio Code

[简体中文](../zh-CN/guides/vscode.md)

The CalcKernel extension supports CK 0.14 `.ck` source files. Install the VSIX
that matches the editor host platform and architecture:

| VS Code host | VSIX target |
| --- | --- |
| macOS Apple silicon | `darwin-arm64` |
| macOS Intel | `darwin-x64` |
| Linux ARM64 | `linux-arm64` |
| Linux x64 | `linux-x64` |
| Windows ARM64 | `win32-arm64` |
| Windows x64 | `win32-x64` |

The package name is `calckernel-vscode-0.14.0-<target>.vsix`. Install it from
the Extensions view with **Install from VSIX...**, or use the command line:

```sh
code --install-extension calckernel-vscode-0.14.0-linux-x64.vsix
```

The [VS Code extension workflow](../../.github/workflows/vscode-extension.yml)
uploads one platform-specific VSIX artifact for each of the six targets.
Download the artifact matching the VS Code host, then install its `.vsix` file.

The extension requires VS Code 1.91 or later. It activates when a `.ck` file is
opened and provides:

- CK syntax highlighting, bracket/comment handling, and code snippets.
- Live diagnostics from the CK frontend, including diagnostics for unsaved edits.
- Completion, hover, signature help, go to definition, references, and safe rename.
- Document and workspace symbols, semantic highlighting, folding, selection ranges,
  and document formatting.
- **CK: Check Current File**, **CK: Run Current File**, and **CK: Build Current File**
  commands, plus language-server restart and output commands.

CK has no import or module system, so language navigation and rename follow the
bindings in the CK source documents rather than resolving imports between files.
Workspace symbol search also finds top-level functions and structs in unopened
`.ck` files under the workspace folders. Unsaved text takes priority over the
corresponding disk file. Each search scans at most 512 CK files and 16 MiB of
source, returning at most 1,000 symbols. Generated directories such as
`target`, `build`, and `node_modules`, version-control directories, and symbolic
links inside the scanned directory tree are skipped. A workspace folder opened
through a symbolic link is resolved once as a search root.
On Windows network shares, files whose normalized path cannot be read by the
operating system are omitted from workspace symbol results.

## Compiler paths

The VSIX includes a frontend-only `ckc` for the language server. It runs without
LLVM and provides editor analysis offline. By default, the extension uses this
bundled server, then searches `PATH` if the bundled executable is unavailable.
Set `ck.server.path` to an absolute path to select another `ckc` 0.14.x server.

Run and Build need a separate Native-enabled `ckc` 0.14.x. Set `ck.compiler.path`
to its absolute path, or leave the setting empty to search `PATH`. Build asks
for an artifact kind (`executable`, `dynamic`, `static`, or `object`) and an
output path. These commands save the active file before invoking `ckc`; their
stdout and stderr are shown in the CalcKernel output channel. The compiler must
be built with the Native toolchain for Run and Build.

Example user settings:

```json
{
  "ck.server.path": "/opt/calckernel/bin/ckc",
  "ck.compiler.path": "/opt/calckernel-native/bin/ckc"
}
```

The language server analyzes the editor's current document text and does not
need to save it. The Check, Run, and Build commands require a local, saved `.ck`
file. In a virtual workspace, syntax highlighting and snippets are available;
the language server and compiler commands require a file-backed document. The
extension does not download a compiler or build Rust code when it starts.
