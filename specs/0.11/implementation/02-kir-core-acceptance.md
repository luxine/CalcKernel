# 阶段 02 验收：KIR 核心

## 必须通过

1. `cargo test --locked --test ir kir_ -- --nocapture`
2. `cargo test --locked --test ir mir_ -- --nocapture`
3. `cargo test --locked --test optimizer optimizer_should_preserve -- --nocapture`
4. `cargo test --locked --test backend -- --nocapture`
5. `cargo fmt --check`
6. `cargo clippy --all-targets --locked -- -D warnings`
7. `git diff --check`

## 结构断言

- O0 KIR 对所有有效 CFG 通过 validator；每种定向 mutation 被拒绝且错误稳定。
- checked 与 unchecked 的唯一差异来自 `KirBuildConfig` 规定的 guard/effect，不来自
  backend 私有补丁。
- artifact 裁剪发生在 effect/SSA 构造前；不可达 runtime call 不进入 KIR。
- KIR printer 确定且不改变 `emit-mir` bytes。
- 本阶段没有任何 guard elimination。

## 完成证据

执行时追加 SHA、test count 与代表性 checked/unchecked KIR snapshot 摘要。
