# CalcKernel 0.10 Performance Guide

[English](../../guides/performance.md)

Native runtime contract 将 optimized CK 与同一 source 经 C emitter 后由固定 Clang 22.1.8
oracle 编译的结果比较。两者使用 O3、strict floating-point、相同 target/CPU policy、checked
mode、warm-up 与 measurement harness。

所有接受 kernel 的 geometric mean 中，Native throughput 不低于 C oracle 的 95%；任何单个
kernel 的 regression 不得超过 10%。Checked 与 unchecked suite 分别在受控 x86-64/AArch64
worker 上，对 baseline/native CPU policy 独立报告并 gate。Gate 只比较 runtime；compile
latency、cold/warm run、memory 与 artifact size 另行报告。

```sh
cargo bench --bench ckc_perf --features native-toolchain
```

通用 summary 输出为 `build/perf/latest.summary.json` 与 `build/perf/latest.summary.md`；
严格 Native comparison 输出为 `target/ckc-perf/results.json`。

Case 位于 `benches/cases/native-cases.tsv`，source 位于 `benches/fixtures`，machine-readable
contract 见 `benches/summary-schema.md`。Harness 使用 repeated sample/median、batch kernel，记录
compiler/OS/architecture identity，并在计时前拒绝 semantic mismatch。

Performance 不得改变 diagnostic、evaluation order、integer/strict float semantics、checked
first-error order、runtime print order、MIR 或 ABI。Benchmark 或 threshold 变化按 contract
change 审查。
