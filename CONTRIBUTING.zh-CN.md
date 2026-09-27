# 为 CalcKernel 贡献代码

[English](CONTRIBUTING.md)

感谢你关注 CalcKernel。我们欢迎你为 CK 语言、编译器、文档、示例和基准测试贡献内容。

## 开始修改前

- 先搜索已有的 Issue 和 Pull Request。较大的语言或架构变更，建议先开一个 Issue 讨论预期行为，再开始实现。
- 保持改动聚焦，并说明它解决了什么用户问题。
- 使用项目的规范名称 **CK / CalcKernel**、`.ck` 源文件扩展名和 `ckc` 编译器命令。
- 不要在提交或 Issue 中包含凭据、个人数据或特定机器生成的构建产物。

## 代码目录

- 编译器实现：`src/frontend/`、`src/ir/`、`src/optimizer/`、`src/backend/` 和 `src/cli/`。
- 可运行的语言示例：对应的 `examples/` 子目录。
- 集成测试：参见 [`tests/README.md`](tests/README.md) 中按职责划分的目录；共享辅助代码放在 `tests/support/`。
- 用户文档：`docs/` 及其简体中文镜像 `docs/zh-CN/`。
修改语言、CLI、MIR、ABI、兼容性或发布行为时，请同步更新相关测试和面向用户的文档。`docs/` 下正式文档的变更必须在 `docs/` 和 `docs/zh-CN/` 中使用相同的相对路径提供英文与简体中文版本。

## 构建和检查

默认功能集不需要项目指定的原生 LLVM 工具链：

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
```

原生编译器功能需要 `native/llvm/manifest.toml` 指定的 LLVM 前缀。请先按 [`README.md`](README.md) 的源码构建说明准备环境，再运行：

```sh
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
```

请在 Pull Request 中列出你运行过的命令及结果。如果当前环境缺少原生构建依赖，请说明原因，并运行不依赖该工具链的检查。

## Pull Request 流程

1. 从 `main` 创建分支，并围绕一个主题完成改动。
2. 为行为变更添加或更新测试；文档所描述的契约发生变化时，同步更新中英文文档。
3. 运行相关检查，并检查差异中是否包含构建产物、意外提交的密钥或无关改动。
4. 向 `main` 提交 Pull Request，说明问题、解决方式、兼容性影响和验证结果，并关联相关 Issue。
5. 根据评审意见调整改动，并确保必要检查通过。提交 Pull Request 不代表改动一定会被接受；最终由维护者评审和合并。

不要在公开 Issue 或 Pull Request 中披露漏洞细节，请按 [`SECURITY.zh-CN.md`](SECURITY.zh-CN.md) 中的私密流程报告。
