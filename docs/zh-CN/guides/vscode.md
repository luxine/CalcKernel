# 在 Visual Studio Code 中使用 CalcKernel

[English](../../guides/vscode.md)

CalcKernel 扩展为 `.ck` 源文件提供编辑器支持。扩展的发布周期独立于 CK 0.15.0
编译器。请安装与 VS Code 所在操作系统和处理器架构对应的可用 VSIX：

| VS Code 主机 | VSIX target |
| --- | --- |
| macOS Apple silicon | `darwin-arm64` |
| macOS Intel | `darwin-x64` |
| Linux ARM64 | `linux-arm64` |
| Linux x64 | `linux-x64` |
| Windows ARM64 | `win32-arm64` |
| Windows x64 | `win32-x64` |

扩展 ID 为 `Luxine.calckernel-vscode-plugin`，与现有的 Marketplace 条目一致。
安装包名包含扩展自身版本和目标平台，例如 `calckernel-vscode-plugin-<version>-<target>.vsix`。
可在扩展视图中选择 **Install from VSIX...**，也可以在命令行中指定下载的文件：

```sh
code --install-extension ./path/to/the-downloaded-extension.vsix
```

Marketplace 条目提供已发布的扩展包。构建和打包说明由扩展的公开源码仓库维护。

扩展要求 VS Code 1.91 或更高版本。打开 `.ck` 文件时扩展会自动激活，并提供：

- CK 语法高亮、括号和注释处理、代码片段。
- CK frontend 实时诊断，包括尚未保存的修改。
- 补全、悬停、签名帮助、转到定义、查找引用与安全重命名。
- 文档和工作区符号、语义高亮、代码折叠、选择范围与文档格式化。
- **CK: Check Current File**、**CK: Run Current File**、**CK: Build Current File**，以及
  重启语言服务和打开输出面板的命令。

CK 没有 import 或 module system，因此语言导航与重命名基于 CK 源文档中的绑定关系，
不会解析文件间的导入关系。
工作区符号搜索也会查找工作区目录下尚未打开的 `.ck` 文件中的顶层函数与结构体。
未保存的编辑内容优先于对应的磁盘文件。每次搜索最多扫描 512 个 CK 文件和 16 MiB
源代码，并返回最多 1,000 个符号。`target`、`build`、`node_modules` 等生成目录、
版本控制目录和扫描目录树内的符号链接会跳过。通过符号链接打开的工作区文件夹会被解析为搜索根目录。
在 Windows 网络共享目录中，操作系统无法读取规范化路径的文件不会出现在工作区符号结果中。

## 编译器路径

VSIX 内含一个 frontend-only `ckc` 供语言服务使用。它不需要 LLVM，可离线提供编辑器分析。
内置编译器的版本取决于单独发布的 VSIX，可能早于编译器 0.15.0。默认先使用安装包内的
服务端；若该程序不可用，再从 `PATH` 查找。将 `ck.server.path` 设为绝对路径，可使用
单独安装的 `ckc` 0.15.x 服务端。

Run 和 Build 需要单独安装启用了 Native toolchain 的 `ckc` 0.15.x。将
`ck.compiler.path` 设为其绝对路径；留空时会从 `PATH` 查找。Build 会让你选择产物类型
（`executable`、`dynamic`、`static` 或 `object`）和输出路径。调用 `ckc` 前，这些命令会先
保存当前文件；标准输出和错误输出会显示在 CalcKernel 输出面板中。Run 与 Build 使用的
compiler 必须通过 Native toolchain 构建。

用户设置示例：

```json
{
  "ck.server.path": "/opt/calckernel/bin/ckc",
  "ck.compiler.path": "/opt/calckernel-native/bin/ckc"
}
```

Language server 会分析编辑器当前文档文本，不要求先保存。Check、Run 和 Build 命令要求
本地文件系统中的 `.ck` 文件，并会先保存文件。在虚拟工作区中可使用语法高亮和代码片段；
语言服务和编译器命令要求文档由本地文件支持。扩展启动时不会下载 compiler，也不会构建
Rust 代码。
