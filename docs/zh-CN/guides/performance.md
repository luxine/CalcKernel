# CalcKernel 0.13 性能指南

[English](../../guides/performance.md)

CalcKernel 0.13 使用 fail-closed performance report schema 8。正式 release 必须具有固定
x86-64 与 AArch64 worker 的完整 report；本地 build 或 release-candidate identity 不能代签。
Measurement 绑定 candidate SHA、exact 0.12 replay SHA、LLVM/Clang 22.1.8、Rust 1.90.0、
hardware/capability manifest、compiler/oracle/source/recipe digest、training/held-out corpus、
profile shard/final profile、target set、variant object、artifact bytes、sample order 与全部 raw sample。

Ordinary regression 的精确 replay 是 CalcKernel 0.12 commit
`c70681e578a14ceea0b2bf0d730661140514793e`。Clang/Rust PGO oracle 使用与 CK 相同的
training/evaluation split 和 source-level precondition，禁用 fast math/contraction，并通过
differential 与 undefined-behavior audit。Training data 不作为 held-out timing evidence；
correctness 另含 adversarial corpus。

## Sampling protocol

全部 timed channel 使用相同 source mode、input、batch、process 与 CPU policy。Dynamic load、
symbol lookup 与 dispatch resolution 在 steady-state timing 前完成，report 证明 resolver 只执行
一次。Channel 使用固定 rotating warm-up/sample schedule，保留实际 order/sample，采用 upper
median，并执行闭合 stability rule。Stability failure 使 evidence 无效，不允许任意重跑或删 case。
缺少、unknown、extra 或不匹配的 report field、digest、stream、tier、capability 都使 checker 失败。

累积的 0.12 vector/domain replay 使用
`interleaved-upper-median-three-channel-v2`。每个保留行交错执行七轮 candidate/C/Rust，再保留
各 channel 的 upper median。仅 `slp_quad` 在逐行 common-mode 归一化后执行未改变的 16/20
稳定性门槛；throughput 仍只使用原始保留耗时。

## 累积 release gate

- 0.13 ordinary no-PGO baseline/native 相对 exact 0.12 replay：geometric-mean slowdown 不超过
  2%，单项不超过 5%。
- PGO use 相对相同 0.13 ordinary CPU policy：geometric-mean improvement 至少 5%，held-out
  单项 slowdown 不超过 3%；固定 instrumentation corpus 上 generation execution 不超过 ordinary 5x。
- Eligible multiversion dispatch 相对 portable baseline：geometric-mean improvement 至少 8%，
  单项 slowdown 不超过 3%；dispatch 至少达到 selected-direct geometric mean 的 98%，单项
  最多慢 5%。
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
  慢 10%，checked proof loop 至少达到 unchecked 的 97%，vector/domain gate 保持，optimizer
  latency 保持既有 suite 2x、单项 3x 上限。

Runtime throughput、generation overhead、source-to-object time、artifact/compiler archive size、
memory、cold/warm execution 与 cache behavior 是分离指标。任何 threshold 都不能削弱 diagnostic、
evaluation order、modular integer、strict float、checked first-error、print/effect order、semantic
MIR、public ABI 或 contract domain。

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

PGO 与受限 multiversioning 只有在这些 gate 通过后才随 0.13 交付。Auto-Tuning remains 0.14；
indirect-call promotion、scalable KIR 与 adaptive JIT PGO 仍是未来工作。
