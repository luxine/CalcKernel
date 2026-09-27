# CalcKernel

[English](README.md) · [官网](https://calckernel.org/zh-CN/) · [快速开始](https://calckernel.org/zh-CN/docs/getting-started/) · [文档](https://calckernel.org/zh-CN/docs/) · [下载](https://github.com/luxine/CalcKernel/releases/latest) · [VS Code 扩展](https://marketplace.visualstudio.com/items?itemName=Luxine.calckernel-vscode-plugin) · [性能实测](https://calckernel.org/zh-CN/#performance) · [MIT 许可](LICENSE)

CalcKernel（简称 CK）是一门面向数值计算内核的静态类型语言，适合编写可由较大应用调用的专注计算函数。程序以 `.ck` 为扩展名，使用 `ckc` 检查、运行和构建。CK 可以生成本机程序和库，也可以输出 C 源码或 WebAssembly，接入现有系统。

CK 负责计算部分；界面、输入输出和数据存储等周边工作仍由宿主应用处理。类型明确的函数接口和由调用方管理的数据切片，让两者之间的数据边界清楚可见。

最新稳定版为 0.14.0；当前默认分支的开发版本为 `0.15.0-dev.0`。

## CalcKernel 的特点

- **专注数值计算。** 使用带类型的函数、结构体、整数与浮点数、条件和循环表达计算过程。
- **生成本机程序。** 将 CK 程序和库编译为本机机器码；对符合条件的计算，编译器会应用优化。
- **便于接入现有系统。** 可以构建带 C ABI 的本机库，也可以生成 C 源码或 WebAssembly 模块。
- **数据所有权明确。** 应用提供传给 CK 的内存并继续负责管理；Native 与 C 构建还可启用整数溢出和切片边界检查。

## 开始使用

1. 下载适用于你系统的[编译器](https://github.com/luxine/CalcKernel/releases/latest)。
2. 跟随[第一个程序教程](https://calckernel.org/zh-CN/docs/getting-started/)创建并运行 CK 程序。
3. 使用任意文本编辑器，或安装可选的 [Visual Studio Code 扩展](https://marketplace.visualstudio.com/items?itemName=Luxine.calckernel-vscode-plugin)。

扩展提供 CK 语法支持、实时错误提示、代码导航，以及检查、运行和构建当前文件的命令。安装说明见 [VS Code 指南](https://calckernel.org/zh-CN/docs/vscode/)。

## 继续了解

- [语言介绍](https://calckernel.org/zh-CN/docs/language/)
- [完整文档](https://calckernel.org/zh-CN/docs/)
- [性能指南与测试结果](https://calckernel.org/zh-CN/docs/performance/)
- [首页性能图表](https://calckernel.org/zh-CN/#performance)
- [项目源码](https://github.com/luxine/CalcKernel)

实际性能取决于算法、具体实现、所用库和目标机器。链接中的数据对应特定计算任务，不代表对各种语言的普遍排名。

## 从源码构建

默认功能集无需 LLVM，即可构建和测试前端、C 与 WebAssembly 组件：

```sh
cargo test --locked
cargo build --release --locked
```

构建 Native 功能需要准备 [`native/llvm/manifest.toml`](native/llvm/manifest.toml) 指定的固定静态 LLVM 前缀。bootstrap 脚本会在构建前校验 LLVM 源码压缩包的 SHA-256：

```sh
rustc_host="$(rustc -vV | sed -n 's/^host: //p')"
llvm_archive="$PWD/llvm-project-22.1.8.src.tar.xz"
curl -fL 'https://github.com/llvm/llvm-project/releases/download/llvmorg-22.1.8/llvm-project-22.1.8.src.tar.xz' -o "$llvm_archive"
llvm_prefix="$PWD/build/llvm/prefix-$rustc_host-release"
./scripts/bootstrap-llvm.sh --archive "$llvm_archive" \
  --prefix "$llvm_prefix" --target "$rustc_host" --profile release
CKC_LLVM_PREFIX="$llvm_prefix" cargo build --release --features native-toolchain --locked
```

该前缀用于构建 CalcKernel 的 Native 功能；运行发行版程序时不需要 LLVM。Windows MSVC 构建要求见[入门指南](docs/zh-CN/guides/getting-started.md)。

## 社区

欢迎参与项目。提交改动前，请阅读[贡献指南](CONTRIBUTING.zh-CN.md)、[安全策略](SECURITY.zh-CN.md)和[行为准则](CODE_OF_CONDUCT.zh-CN.md)。

## 许可

CalcKernel 使用 [MIT 许可](LICENSE)发布。
