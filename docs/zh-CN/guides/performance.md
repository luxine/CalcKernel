# CalcKernel 性能指南

[English](../../guides/performance.md)

使用 `cargo bench --bench ckc_perf` 运行 native benchmark harness。Case 当前位于
`bench/perf/cases/native-cases.tsv`，仓库布局任务会把它与 fixture/schema 一起
原子移动到 `benches/` 下，并同时更新本指南。

应分别测量 lexer/parser/checker、MIR lower/optimize、backend emission、toolchain
build time、artifact size 与 runtime。预热外部工具，记录 compiler/OS/architecture/
toolchain version，多次采样并比较 median。Runtime kernel 应 batch work，避免把
FFI call overhead 误当成 compiler performance。

比较 O0–O3 与所有支持的 C mode 组合。Checked mode 会增加 branch/status
propagation，应与 unchecked throughput 分开报告；WASM/LLVM 没有 checked 对照。
任何性能收益都不能改变 diagnostic、evaluation order、数值语义、checked error
order、MIR 或 ABI。
