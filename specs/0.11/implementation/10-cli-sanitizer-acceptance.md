# 阶段 10 验收：CLI 与 contract sanitizer

## 环境

使用阶段 09 的 pinned LLVM/Clang 环境变量。

## 必须通过

1. `cargo test --locked --test cli kir_ -- --nocapture`
2. `cargo test --locked --test cli argument_ -- --nocapture`
3. `cargo test --all-features --locked --test native contract_sanitizer_ -- --nocapture`
4. `cargo test --all-features --locked --test native run_ -- --nocapture`
5. `cargo test --all-features --locked --test native cache_ -- --nocapture`
6. `cargo test --all-features --locked --test native runtime_ -- --nocapture`
7. `cargo test --locked --test backend header_ -- --nocapture`
8. `cargo fmt --check`
9. `cargo clippy --all-targets --all-features --locked -- -D warnings`
10. `git diff --check`

## 结构断言

- inspection 输出重复运行 byte-identical，且不含绝对路径、地址、时间或 unordered map。
- normal O0–O3 Native IR 无 contract sanitizer guard/runtime symbol。
- 所有 violation 精确输出 `CKR0007: unsafe contract violation\n` 且 status=246。
- 极值/回绕 case 不 crash、不触发 host UB、不错误通过。
- sanitizer 只用于 run/executable；library/emit 命令在写文件前拒绝。
- header comments 可机械映射 flattened slice fields，C ABI declaration shape 不变。

## 完成证据

执行时追加 SHA、runtime/bridge identity、每个 O-level sanitizer matrix 与 exact-byte 摘要。
