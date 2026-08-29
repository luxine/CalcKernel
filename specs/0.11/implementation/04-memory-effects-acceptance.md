# 阶段 04 验收：Memory SSA、alias 与效果

## 必须通过

1. `cargo test --locked --test ir memory_ -- --nocapture`
2. `cargo test --locked --test optimizer alias_ -- --nocapture`
3. `cargo test --locked --test optimizer effect_ -- --nocapture`
4. `cargo test --locked --test frontend ck2016_ -- --nocapture`
5. `cargo test --locked --test optimizer runtime_print_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- mutation tests 拒绝错误 partition、memory phi 与 call effect mapping。
- pairwise `noalias(a,b)` 在存在第三 alias root 时不会生成 full-parameter fact。
- `effects none` 允许 private local memory，但拒绝任何外部可达 read/write。
- print/may-fail/unsafe 不会被 effects clause 隐藏或重排。
- unknown/over-budget 结果严格为保守 summary。

## 完成证据

- 日期：2026-08-29
- 实现提交：`e3f0658`
- `cargo test --locked --test ir memory_ -- --nocapture`：8 passed，0 failed/ignored。
- `cargo test --locked --test optimizer alias_ -- --nocapture`：8 passed，0 failed/ignored。
- `cargo test --locked --test optimizer effect_ -- --nocapture`：6 passed，0 failed/ignored。
- `cargo test --locked --test frontend ck2016_ -- --nocapture`：5 passed，0 failed/ignored。
- `cargo test --locked --test optimizer runtime_print_ -- --nocapture`：3 passed。
- `cargo fmt --check`、`cargo clippy --all-targets --locked -- -D warnings`、
  `git diff --check`：全部通过。
- 补充全仓验证：`cargo test --locked` 共 300 passed，0 failed/ignored；固定 LLVM 环境下
  `cargo clippy --locked --all-targets --all-features -- -D warnings` 通过。
- alias 覆盖 root/copy/sub-slice、零长度、确定 sibling byte interval、pairwise
  `noalias(a,b)` 与第三 root 反例；两根完全分离时生成两条 Memory SSA 版本链，第三根
  可能别名时按连通分量回退 conservative partition。
- Memory SSA 覆盖 load/store/call/join/loop；effect-aware call 只更新 parameter-mapped
  partition，unknown/multi-partition call 保守合并。交换 memory-phi 参数的 mutation 被
  verifier 以 partition mismatch 拒绝。
- 共享 SCC solver 覆盖 direct/transitive/recursive 三类 component、参数回映射、未知
  callee 与 `max_steps=0` 预算 fallback；fallback 精确为 `readwrite all + may_fail +
  runtime_effect + unsafe_calls`，不读取墙钟。
- CK2016 正例覆盖精确 read/write/readwrite ceiling、`effects none` 无 externally
  reachable memory、sub-slice 与 transitive callee 回映射；反例覆盖 underdeclared
  read/write/readwrite、raw pointer/all。runtime print、may-fail 与 unsafe-call 标志独立
  保留，不受 memory ceiling 隐藏。
