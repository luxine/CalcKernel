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

执行时追加 SHA、SCC/alias/mutation test count 和 CK2016 正反例摘要。
