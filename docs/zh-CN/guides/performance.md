# CalcKernel 性能指南

[English](../../guides/performance.md)

CalcKernel 0.14 保留 fail-closed performance report schema 8。正式 release 必须具有固定
x86-64 与 AArch64 worker 的完整 report；本地 build 或 release-candidate identity 不能代签。
Measurement 绑定 candidate SHA、exact 0.12 replay SHA、LLVM/Clang 22.1.8、Rust 1.90.0、
hardware/capability manifest、compiler/oracle/source/recipe digest、training/held-out corpus、
profile shard/final profile、target set、variant object、artifact bytes、sample order 与全部 raw sample。

Ordinary regression 的精确 replay 是 CalcKernel 0.12 commit
`e1bcea461492a5a2619cdb960ea00dd668847f0a`。Clang/Rust PGO oracle 使用与 CK 相同的
training/evaluation split 和 source-level precondition，禁用 fast math/contraction，并通过
differential 与 undefined-behavior audit。Training data 不作为 held-out timing evidence；
correctness 另含 adversarial corpus。

0.10、0.11 和 0.12 回放源码树以校验和固定的产品归档保存在
`benches/baselines/sources/`。Fresh clone 不需要旧仓库历史：preparer 会校验每个归档及其
固定输入 digest，然后重建临时 baseline tree，以检查批准的 adapter 和源码 diff。Replay
report 仍记录 baseline manifest 中原始源码 commit identity。

## 跨语言对比

独立的[跨语言 benchmark](../../../benches/cross-language/README.md) 使用相同的确定性输入，对比
仓库内 CalcKernel、C++、Rust、JavaScript、Java 与 NumPy 内核。Runner 在计时前验证输出 hash，
并记录源码 hash、工具版本、编译参数、样本顺序和原始测量值。可从全新 compiler
checkout 按该 benchmark README 中列出的依赖运行。

## Sampling protocol

全部 timed channel 使用相同 source mode、input、batch、process 与 CPU policy。Dynamic load、
symbol lookup 与 dispatch resolution 在 steady-state timing 前完成，report 证明 resolver 只执行
一次。Channel 使用固定 rotating warm-up/sample schedule，保留实际 order/sample，采用 upper
median，并执行闭合 stability rule。Stability failure 使 evidence 无效，不允许任意重跑或删 case。
缺少、unknown、extra 或不匹配的 report field、digest、stream、tier、capability 都使 checker 失败。

累积的 0.12 vector/domain replay 使用
`interleaved-upper-median-three-channel-v3`。三条通道使用同一数据工作区，避免分配位置差异伪装成
代码性能差异。每个保留行交错执行七轮 candidate/C/Rust，再保留
各 channel 的 upper median。仅 `slp_quad` 在逐行 common-mode 归一化后执行未改变的 16/20
稳定性门槛；throughput 仍只使用原始保留耗时。

Hand-written oracle 使用 architecture-specific baseline flag，禁用 fast math/contraction，且
不得使用 CK baseline profile 不具备的 CPU feature。它们获得 source language 可表达的全部
等价 precondition，并须在固定 declared valid domain 上通过 differential 与 undefined-behavior
audit。缺失、无效或测量后排除 competitor 都会使 gate 失败。

## 累积 release gate

- 0.14 ordinary no-PGO baseline/native 相对 exact 0.12 replay：geometric-mean slowdown 不超过
  2%，单项不超过 5%。
- PGO use 相对相同 0.14 ordinary CPU policy：geometric-mean improvement 至少 5%，held-out
  单项 slowdown 不超过 3%；固定 instrumentation corpus 上 generation execution 不超过 ordinary 5x。
- Eligible multiversion dispatch 相对 portable baseline：geometric-mean improvement 至少 8%，
  单项 slowdown 不超过 3%；dispatch 至少达到独立加载的同字节 artifact 中 resolver 实际选中
  hidden member direct call geometric mean 的 98%，单项最多慢 5%。
  ELF collector 从 `.dynsym` 读取 public entry，并从 private `.ck_dispatch_slot` section 读取
  实际发布 pointer，因此 shipped product 无需保留完整 local symbol table 也能维持该证明。
- Combined PGO+multiversion 相对较快的对应 PGO-only/multiversion-only channel，geometric
  mean 最多慢 2%，单项最多慢 5%。
- Combined CK 至少达到较快等价 Clang/Rust PGO geometric mean 的 95%，每个 accepted
  kernel 至少达到 90%。
- PGO/multiversion/combined source-to-object geometric-mean ratio 不超过 ordinary 的
  1.5x/2.5x/3.5x，单项不超过 2x/3x/4x；artifact aggregate 不超过 1.25x/2x/2x，
  单项不超过 1.5x/2.5x/2.5x；distributed `ckc` archive 相对 exact 0.12 最多增长 15%。
  Source-to-object 样本使用已终止子进程的 user+system CPU time，排除托管 worker 被调度
  移出的时间，同时不移除任何编译器工作。
- 保留全部 0.12 累积 gate：Native 至少达到 pinned Clang geometric mean 的 95%，单项最多
  慢 10%，checked proof loop 至少达到 unchecked 的 97%，optimizer
  latency 保持既有 suite 2x、单项 3x 上限。
- 每个架构与 safety mode 上，vector kernel 至少达到各 kernel 中较快有效 C/Rust SIMD oracle
  geometric mean 的 95%，每个 kernel 至少达到自身 oracle 的 90%；domain-fact suite 至少
  超过较快 generic Clang/Rust oracle geometric mean 的 5%。
- Unchanged scalar corpus 相对 independently replayed 0.11 的 geometric mean 最多慢 3%，
  单项最多慢 8%；Native object size 相对同一固定 replay 的 aggregate 增长不超过 35%，
  单项不超过 2.5x；baseline O3 source-to-object compile ratio 的 geometric mean 不超过
  1.5x，单项不超过 2x。

Runtime throughput、generation overhead、source-to-object time、artifact/compiler archive size、
memory、cold/warm execution 与 cache behavior 是分离指标。任何 threshold 都不能削弱 diagnostic、
evaluation order、modular integer、strict float、checked first-error、print/effect order、semantic
MIR、public ABI 或 contract domain。

## 有界 checked kernel 诊断

Linux/AArch64 CI 启用 `CKC_OBSERVE_CHECKED_RUNTIME=1`，在**原始** checked
`specialized_length` gate 调用外保留可选观测。原有 timer、kernel 调用循环、corpus 和
sampler 保持不变，仍由它们决定报告。每个 measurement evidence 目录保存
`checked-runtime-observations.jsonl`：library hash、实际 entry/input/output 地址、
input/result digest、process map，以及全部 429 次原始调用（9 次 warmup 和
20 × 7 × 3 次正式调用）。每次调用前后记录 wall/thread CPU clock、user/system CPU
记账、page fault 和 context switch。记录空间预先分配，采样后才写出；观测失败不能
替换原有结果。不可用指标是 null，不是零。

`python3 scripts/check-runtime-observations.py target/ckc-perf/results-baseline.json`
核对保留的 library byte，并从该报告的旁路记录逐项还原全部 sample 和 median。
这仅验证证据一致性，不作 release acceptance。外层快照还包含结果 hash 与边界工作，
不是原子快照，也可能扰动进程状态。不完整旁路记录属于无效证据；历史缺失观测无法恢复。

仅在显式启用 workflow 的 `performance_diagnostics` input 时，原有 gate 结束后，
Linux/AArch64 CI 在独立对照中加载经 hash 验证的 checked
`specialized_length` CK/C/Rust 指令体，比较原始地址与三个固定复制布局。复制区域只读可执行，
不会同时可写可执行，并须保持各原始通道的正常结果和错误前缀行为。所有布局共享同一
input/output 工作区并执行固定完整采样顺序。全部 raw row、映射地址、CPU affinity、资源快照
和可用的仅用户态硬件计数保存于 `target/performance-diagnostics/checked-aarch64-layout`。

这是代码放置位置的受控干预，不重新生成或替代 release evidence，不恢复历史映射，不改变
任何 gate，也不自动认定根因。硬件计数区间包含时钟边界工作；不可用或复用的计数不能当作零。
指令体发生变化时，明确报告为不适用于本次有界对照。

## 命令与证据

昂贵的稳定 worker 测量前先运行本地 schema/checker/correctness check：

通用 harness 入口为 `cargo bench --bench ckc_perf`，输出
`build/perf/latest.summary.json` 与 `build/perf/latest.summary.md`。Native 与 PGO
测量再增加下面所示的 feature 和 task selector。

```sh
cargo test --locked --test performance -- --nocapture
python3 -m unittest discover -s tests/performance -p '*_test.py'
python3 scripts/prepare-performance-replay.py --baseline 0.12 \
  --out target/performance-runtime-replay-v012
python3 scripts/prepare-performance-replay.py --baseline 0.11 \
  --out target/performance-runtime-replay-v011
python3 scripts/prepare-performance-replay.py --baseline 0.10 \
  --out target/performance-runtime-replay
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
cp target/ckc-perf/results.json target/ckc-perf/results-baseline.json
python3 scripts/check-native-performance.py target/ckc-perf/results-baseline.json
cargo bench --features native-toolchain --bench pgo_perf -- \
  --task collect --out target/ckc-perf/v0.13-results.json
python3 scripts/check-native-performance.py target/ckc-perf/v0.13-results.json
```

Native 命令要求固定路径 `CKC_LLVM_PREFIX`、`CKC_CLANG_ORACLE`、
`CKC_CANDIDATE_COMPILER`、`CKC_V012_RUNTIME_BUNDLE`、
`CKC_V011_RUNTIME_BUNDLE` 与 `CKC_V010_RUNTIME_BUNDLE`。两个 report 必须由
同一个 worker 生成并检查；复制或跨 worker 的 schema-7 report 不能作为 release evidence。

Report 在独立 checker 读取前 canonicalize 并 hash；benchmark 本身不能宣称通过。Diagnostic
只检查实际 report/artifact，不重建或重新计时 required gate。修改 source、corpus、profile、
target/capability、oracle precondition、threshold、statistic、exclusion 或 checker 均属于需评审
contract change。

## WebAssembly 运行时观测

独立的 Node/V8 runner 测量 WebAssembly 产物，与上述 Native 发布门槛分开。
先构建一次 `ckc`，再用固定输入测量同一批 CK 示例的 O0 和 O3。Runner 接受
`--wasm-features baseline|simd128`，默认 `baseline`；分别记录两个 profile 时使用独立输出目录：

```sh
cargo build --release --locked --bin ckc
node benches/wasm/bench.mjs --ckc target/release/ckc --wasm-features baseline \
  --out build/wasm-perf/baseline --samples 20 --warmup 10 --batch 100 --size 1024
node benches/wasm/bench.mjs --ckc target/release/ckc --wasm-features simd128 \
  --out build/wasm-perf/simd128 --samples 20 --warmup 10 --batch 100 --size 1024
CKC=target/release/ckc node --test benches/wasm/bench.test.mjs examples/wasm/host/*.test.mjs
```

Runner 将 `wasm-runtime-report.json` 和生成的模块写入所选 `--out` 目录（默认
`build/wasm-perf`）；该目录应保持为 ignored build output。它在计时前校验输出，并记录准确的源码、编译器、runner 与产物身份，以及原始样本和
宿主/runtime 信息。Report 还记录请求的 feature profile、规范化 profile digest，以及每个产物的
`ck.wasm.target` metadata；计时前会核对请求值、digest 与实际 metadata 是否一致。O3 的
`f64_map`、`i32_map`、`u32_compare_select`、`i32_to_f64`、`u32_to_f64`、
`u32_alias_map`、`u32_reduce_sum` 和 `u32_reduce_product` case 在选用 `simd128` 时测试
经过独立验证的 SIMD128 路径；baseline profile 与 O0 artifact 提供标量对照。评估别名回退
还须运行单独的重叠与地址边界测试。计时结果应连同正确性和指令形态一并保存。
`u32_cursor_copy` 测量经过检查的 O3 地址游标，`u32_field_offset` 测量有证明支持的
memarg 字段位移。`u32_fill` 测量受运行时守卫保护的 Bulk Memory 填充路径；
`pricing_batch` 使用同一个持久 arena 比较一调用处理多条记录与逐条调用。
评估这些路径时，应与相同 v0.15 profile 下的 O0 比较，并保留发射的指令形态。
P8 编译器可作历史对照，但它使用旧 schema-1 baseline，其 profile digest 和允许特性不同。

CK 发射、模块编译、实例化、预热、稳定内核调用、宿主数据准备与读取分别记录。
`--emission-samples` 重复编译器发射并保存每次原始耗时；兼容字段 `ck_emission` 为其中位数。
产物记录包含总字节数、各 section payload 字节数、code 字节数、函数数和 local 数，
以便同时评估直接二进制输出与代码增长。比较新旧编译器时，应交替运行同一源码、profile 名称
和优化级别，并明确标注 profile schema 的差异。Bulk copy/fill 还需测量短/长区间、重叠和陷阱回退；批处理 case 分别报告逻辑行数、
实际 JS 到 Wasm 的调用次数与 runner 的 `--batch` 重复次数。

模块编译时间是每个产物在该 Node 进程内的首次编译时间，并非浏览器冷启动
测量。每轮端到端样本只涵盖已实例化模块上的准备、调用和读取，不包含模块编译、
实例化与内存扩展。比较编译器版本时应使用相同参数并保留完整 report；一次本地运行
既不能证明普遍加速，也不构成发布阈值。当前 runtime 通道是 Node/V8，性能结论须标明
这一范围。

PGO 与受限 multiversioning 已在 0.13 交付，0.14 仍需通过这些原样保留的 gate。离线
Auto-Tuning 延期；这个兼容性版本不声称新的 optimizer 加速。indirect-call promotion、
scalable KIR 与 adaptive JIT PGO 仍是未来工作。
