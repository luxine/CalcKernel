# 阶段 06 验收：O2

## 必须通过

1. `cargo test --locked --test optimizer kir_o2_ -- --nocapture`
2. `cargo test --locked --test optimizer unsafe_inline_ -- --nocapture`
3. `cargo test --locked --test optimizer gvn_ -- --nocapture`
4. `cargo test --locked --test optimizer memory_opt_ -- --nocapture`
5. `cargo test --locked --test optimizer runtime_print_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- O2 exact pass order 与规范一致。
- unsafe inline scope mutation 被 verifier 拒绝，递归 edge 使用新实例。
- third-root alias、unknown call 与 memory phi 均阻止不合法 GVN/forward/DSE。
- checked failure 与 print 的数量和顺序跨 inline/cleanup 不变。
- O2 新删 guard 均有通过独立 checker 的 ProofId。

## 完成证据

执行时追加 SHA、inline budget、Memory SSA rewrite 数和负向 case 摘要。
