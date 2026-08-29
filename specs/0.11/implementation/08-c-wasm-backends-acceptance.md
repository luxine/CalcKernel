# 阶段 08 验收：C 与 WebAssembly KIR 后端

## 必须通过

1. `cargo test --locked --test backend kir_c_ -- --nocapture`
2. `cargo test --locked --test backend kir_wasm_ -- --nocapture`
3. `cargo test --locked --test backend -- --nocapture`
4. `cargo test --locked --test cli commands::cli_should_check_and_emit_portable_outputs -- --exact --nocapture`
5. `cargo test --locked --test performance bench::benchmark_harness_should_cover_compiler_stages_and_backends -- --exact --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- backend 模块不调用 MIR optimizer/range/alias helper，不根据 mode 私自创建 guard。
- C 与 WASM 在 O0–O3 的 supported-mode differential 全绿。
- pairwise noalias third-root case 不产生 C `restrict`。
- canonical checked C loop 的 backend hot loop 与 KIR 一样无冗余 guard。
- 0.10 C/WASM ABI snapshots 和不可达 runtime reachability 行为不漂移。

## 完成证据

执行时追加 SHA、编译器 identities、differential matrix 与 structural guard count。
