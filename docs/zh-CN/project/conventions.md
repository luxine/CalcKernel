# CalcKernel 仓库约定

[English](../../project/conventions.md)

规范语言名称为 CK 与 CalcKernel，source extension 为 `.ck`，原生 compiler 为
`ckc`，Rust package/library 为 `calckernel`。不得引入 `tk`、`tkc`、`.tk`、wrapper
或 package-surface alias。

Rust source 按职责分为 `frontend`、`ir`、`optimizer`、`backend`、`cli`。公共兼容
通过明确 `lib.rs` re-export 提供，不使用宽泛 visibility。优先 small module、typed
data、borrowed input、显式 error propagation 与 deterministic ordered output。

Test 按相同职责分组，共享 harness 放在 `tests/support`；example 按用途分组；
benchmark/fixture 放在 `benches`。`docs` 下每个正式英文 Markdown 在
`docs/zh-CN` 下都有相同 relative path 的简体中文 peer。

持久文档只描述当前 contract。Temporary design、execution plan、review log、
generated output、local worktree 与历史过程叙述不进入 release tree；Git commit 与
published release note 提供历史。
