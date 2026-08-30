# CalcKernel 0.11 Performance 指南

[English](../../guides/performance.md)

Native runtime contract 同时比较固定 Clang 22.1.8 C oracle，以及精确 commit
`df816502876fba41676f9ebc190e4fadd18cd5a5` 的 CalcKernel 0.10。所有运行使用 O3、
strict floating-point、相同 target/CPU policy/mode、固定输入、warm-up、batch 与 statistic。

Native/Clang geometric-mean throughput 至少 95%，单 case 回退不超过 10%；checked 与
unchecked 在 x86-64/AArch64 worker 上以 portable baseline CPU policy 分别验收。每次运行
都由同一 Clang 22.1.8 编译精确 0.10 compiler 生成且摘要固定的 C oracle；native-CPU 测量
不属于这个 baseline 重放协议，不能用于 release 对比。相对精确固定的 0.10 compiler，门禁比较
`(T0.11-Native / Tcurrent-Clang) / (T0.10-Native / T0.10-Clang)`，geometric 回退最多 3%、
单 case 最多 8%。四个计时项在同一 worker 的同一进程中采样，使用独立构建的固定
0.10 Native libraries 和同一冻结 0.10 C oracle。仅靠 Clang 归一化不能消除任意 CPU
代际差异；历史 median 原样保留作 provenance，比较分母取实际同进程重放样本。
两种安全模式与两个编译器版本在相同输入上采用确定性的八通道轮转采样。
Canonical proof-loop checked throughput 至少为 unchecked 的 97%，这个候选版本
原始比率不做归一化。KIR optimizer 相对 0.10 MIR optimizer 的 suite median
ratio 最多 2x、单 case 最多 3x。

```sh
cargo bench --bench ckc_perf
# 先将 CKC_LLVM_PREFIX 和 CKC_CLANG_ORACLE 指向固定 LLVM/Clang 安装。
python3 scripts/prepare-performance-replay.py --out target/ckc-perf/v010-replay
export CKC_V010_RUNTIME_BUNDLE="$PWD/target/ckc-perf/v010-replay"
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
python3 scripts/check-native-performance.py target/ckc-perf/results.json
```

准备流程要求新的输出目录，在自有本地 clone 中构建固定编译器，不修改 main 或既有
baseline worktree。只应用下述四份固定 adapter，校验源码/工具链身份并记录实际编译器
与八份 library 的摘要。同一准备/重放 recipe 可以复用完整 bundle；recipe 改变时选择
新的输出目录。缺少或被修改的重放证据必须报错。
开发流程需要 Python 3.11+、包含固定 commit 的 Git 历史、Rust 1.90.0 与固定的
LLVM/Clang 22.1.8 安装。CI 获取完整历史后再准备本地 clone。Checker 同样要求设置
`CKC_LLVM_PREFIX`，用于核对已安装组件 manifest 的摘要。

Report 必须与相对它定位的 `target/ckc-perf/measurement-<pid>-<timestamp>` 目录一起
保留，同时保留选定 replay bundle 的 `ckc-v010`、八份 library、`replay.tsv` 与
`preparation.log`。候选两种模式与两份 Clang 对照库均在测量前后核对摘要；精确调度
顺序与每模式四组样本全部记录。只移动 report 会丢失验收所需证据。

严格 schema-6 report 是 `target/ckc-perf/results.json`；case manifest 位于
`benches/cases/native-cases.tsv`，schema 位于 `benches/summary-schema.md`。
Normative checker 拒绝非固定 identity 或 investigative CPU policy，逐项核对 schema-2
baseline manifest 中的历史 V0.10 配对 median，核对 replay bundle 与实际测量产物摘要，
并从稳定 sample array 重新计算所有候选/重放 upper median。必须使用精确交错顺序、
三轮 warm-up、二十个样本、每样本七次调用和两千万输入 batch；quick 测量不能通过门禁。
General compiler-stage summary 仍写入 `build/perf/latest.summary.json` 与
`build/perf/latest.summary.md`。
`benches/baselines/v0_10_compiler.toml` 固定 0.10 commit/compiler/LLVM、target/CPU/mode、
配对 Native/Clang median、harness/statistics，以及每个 CK 与冻结 C-oracle source 的
SHA-256 `sourceDigests`。Identity 或 digest 不符必须失败。

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

Checksum 固定的 `benches/baselines/v0_10_clang_cpu_harness.patch` 按架构冻结 portable
Clang reference：x86-64 使用 `-march=x86-64 -mtune=generic`，对应 CK 的 `x86-64`
baseline；AArch64 保持 `-mcpu=generic`，对应 CK 的 generic ARMv8-A baseline。Native-CPU
flag 仍只用于调查，不会被 release gate 接受。

Performance 不允许改变 diagnostic、evaluation order、modular integer/strict floating
semantics、checked first-error、print order、semantic MIR、ABI 或 contract domain。Generated
contract case 只能使用满足声明 domain 的输入；不能为通过候选版本而降低阈值。

CI 在两个架构的完整性能门之前准备 replay bundle，保留首次 report、构建 provenance
和实际测量的非空 library 原件/摘要。失败后的额外诊断记录 CPU identity 并检查这些
同一产物；空的导出 section 不能作为机器码证据。也可以用
workflow-dispatch 的 `performance_diagnostics` 显式开启同样的诊断，无需等待再次失败。
诊断产物不替代原门禁，不授权刷新冻结基线；原 required job 的失败状态保持不变。
