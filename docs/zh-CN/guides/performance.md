# CalcKernel 0.11 Performance 指南

[English](../../guides/performance.md)

Native runtime contract 同时比较固定 Clang 22.1.8 C oracle，以及精确 commit
`df816502876fba41676f9ebc190e4fadd18cd5a5` 的 CalcKernel 0.10。所有运行使用 O3、
strict floating-point、相同 target/CPU policy/mode、固定输入、warm-up、batch 与 statistic。

Native/Clang geometric-mean throughput 至少 95%，单 case 回退不超过 10%；checked 与
unchecked 在受控 x86-64/AArch64 worker 上以 portable baseline CPU policy 分别验收；
native-CPU 测量只用于调查，不与冻结 baseline 比较。相对记录的 0.10 compiler，0.11 runtime
geometric 回退最多 3%、单 case 最多 8%。Canonical proof-loop checked throughput 至少为
unchecked 的 97%。KIR optimizer 相对 0.10 MIR optimizer 的 suite median ratio 最多 2x、
单 case 最多 3x。

```sh
cargo bench --bench ckc_perf
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
python3 scripts/check-native-performance.py target/ckc-perf/results.json
```

严格 report 是 `target/ckc-perf/results.json`；case manifest 位于
`benches/cases/native-cases.tsv`，schema 位于 `benches/summary-schema.md`。
Normative checker 拒绝非固定 identity 或 investigative CPU policy，并从 sample array
重新计算每个已报告的 upper median。
General compiler-stage summary 仍写入 `build/perf/latest.summary.json` 与
`build/perf/latest.summary.md`。
`benches/baselines/v0_10_compiler.toml` 固定 0.10 commit/compiler/LLVM、target/CPU/mode、
harness/statistics 与每个 source 的 SHA-256 `sourceDigests`。Identity 或 digest 不符必须失败。

冻结的 0.10 benchmark 尚不知道后续 proof-loop 的 slice ABI，因此基线采集只对测量
harness 应用 checksum 固定的 `benches/baselines/v0_10_proof_loop_harness.patch`；编译器仍为
精确固定的 0.10 commit。该补丁提供与 0.11 harness 相同的确定性输入和调用 ABI。

第二个 checksum 固定的 `benches/baselines/v0_10_mir_optimizer_harness.patch` 只测量冻结的
0.10 MIR pass pipeline，解析与 MIR 构造位于计时区外。这与 0.11 仅计 KIR pipeline 的边界
一致，防止把 frontend `check` 时间错误标记为 optimizer 基线。

第三个 checksum 固定的 `benches/baselines/v0_10_linux_cpp_runtime_harness.patch` 在 Cargo
链接未改动的 0.10 编译器前，向选定 C++ 编译器查询静态 `libstdc++.a` 的绝对目录。它只
修复 hosted Ubuntu AArch64 的搜索路径缺口，不改变 CK source、IR、codegen、benchmark
输入或计时边界。

Performance 不允许改变 diagnostic、evaluation order、modular integer/strict floating
semantics、checked first-error、print order、semantic MIR、ABI 或 contract domain。Generated
contract case 只能使用满足声明 domain 的输入；不能为通过候选版本而降低阈值。
