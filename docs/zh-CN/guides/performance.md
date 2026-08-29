# CalcKernel 0.11 Performance 指南

[English](../../guides/performance.md)

Native runtime contract 同时比较固定 Clang 22.1.8 C oracle，以及精确 commit
`df816502876fba41676f9ebc190e4fadd18cd5a5` 的 CalcKernel 0.10。所有运行使用 O3、
strict floating-point、相同 target/CPU policy/mode、固定输入、warm-up、batch 与 statistic。

Native/Clang geometric-mean throughput 至少 95%，单 case 回退不超过 10%；checked 与
unchecked 在受控 x86-64/AArch64 worker 上分别验收。相对记录的 0.10 compiler，0.11 runtime
geometric 回退最多 3%、单 case 最多 8%。Canonical proof-loop checked throughput 至少为
unchecked 的 97%。KIR optimizer 相对 0.10 MIR optimizer 的 suite ratio 最多 2x、单 case
最多 3x。

```sh
cargo bench --bench ckc_perf
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
python3 scripts/check-native-performance.py target/ckc-perf/results.json
```

严格 report 是 `target/ckc-perf/results.json`；case manifest 位于
`benches/cases/native-cases.tsv`，schema 位于 `benches/summary-schema.md`。
General compiler-stage summary 仍写入 `build/perf/latest.summary.json` 与
`build/perf/latest.summary.md`。
`benches/baselines/v0_10_compiler.toml` 固定 0.10 commit/compiler/LLVM、target/CPU/mode、
harness/statistics 与每个 source 的 SHA-256 `sourceDigests`。Identity 或 digest 不符必须失败。

Performance 不允许改变 diagnostic、evaluation order、modular integer/strict floating
semantics、checked first-error、print order、semantic MIR、ABI 或 contract domain。Generated
contract case 只能使用满足声明 domain 的输入；不能为通过候选版本而降低阈值。
