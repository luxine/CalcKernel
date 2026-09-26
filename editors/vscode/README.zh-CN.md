# CalcKernel VS Code 扩展

[English](README.md)

本扩展识别 CK / CalcKernel 的 `.ck` 文件，提供语法高亮、代码片段，以及由当前 Rust `ckc` 编译器生成的实时诊断。语言服务会检查尚未保存的修改。

## 安装

安装与操作系统和处理器架构对应的 VSIX。安装包内含不启用 LLVM 的前端 `ckc` 语言服务，因此高亮和诊断可离线使用。手动配置的服务端须为 0.14.x。

## 设置与命令

- `ck.server.path`：语言服务使用的兼容 `ckc` 绝对路径。留空时依次使用安装包内的程序和 PATH。
- `ck.compiler.path`：运行与构建使用的 Native 版 `ckc` 绝对路径。留空时使用 PATH。
- 命令面板提供 `CK: Check Current File`、`CK: Run Current File`、`CK: Build Current File`、`CK: Restart Language Server` 与 `CK: Show Output`。

运行和构建要求本地 `.ck` 文件，并在调用编译器前保存文件。语言服务可直接检查未保存文本。虚拟工作区保留静态高亮和片段；编译器命令要求本地文件。

扩展激活时不会下载编译器，也不会自动构建 Rust 代码。
