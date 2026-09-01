# 阶段 06 验收：O3 PGO optimizer transaction

## 必须通过

1. `cargo test --locked --test optimizer pgo_ -- --nocapture`
2. `cargo test --locked --test optimizer specialization_ -- --nocapture`
3. `cargo test --locked --test optimizer unroll_ -- --nocapture`
4. `cargo test --locked --test optimizer vector_ -- --nocapture`
5. `cargo test --locked --test optimizer transaction_ -- --nocapture`
6. `cargo test --all-features --locked --test native pgo_o3_ -- --nocapture`
7. `cargo test --all-features --locked --test native pgo_metadata_ -- --nocapture`
8. `cargo test --all-features --locked --test cli pgo_build_final_ -- --nocapture`
9. `cargo test --locked`
10. `cargo fmt --check`
11. `cargo clippy --all-targets --all-features --locked -- -D warnings`
12. `git diff --check`

filter 必须非零；accepted/fallback/mutation/differential 都必须实际执行。

## 结构断言

- PGO candidate 均从同一 verified pre-state 建议，checker 不调用 proposer/analysis implementation，
  并独立复算 profile class bounds、static proof、growth、mapping 与共享 budget。
- guarded value/length path 保留完整 generic fallback；profile 不能改变 strict-f64、checked first-error、
  memory footprint、effect/print order 或 sanitizer contract。
- O3 metadata 只有 exact mapping 后附加，false/stale mapping withholding artifact；O2 仍完全无 metadata。
- rejection/non-winner 不退款，transaction rollback 完整，stable audit/explanation 与 artifact 可复现。

## 完成证据

记录实现 SHA、各 transform test/audit digest、fixed-seed differential、metadata/object audit 和命令结果。
阶段 06 不得把单-target PGO 结果称为 multiversion。
