# CalcKernel 0.12 性能指南

[English](../../guides/performance.md)

CalcKernel 0.12 使用严格 performance report schema 7。Release measurement 在稳定 x86-64
与 AArch64 worker 上运行 portable baseline artifact，保持 strict floating-point 与 safety
semantics，并固定 compiler、source、target、规范化 `KirTargetProfile`、cost/proof schema、
optimizer budget、CK/oracle artifact、sampling schedule 与全部 source digest。除非另行评审并
冻结 hardware identity，native-policy result 只作诊断。

Scalar regression baseline 是 commit
`80c0acf6bb5d65e4d9d40352b9501ea32b79f43d` 独立构建的 0.11.0 compiler；其 compiler、
Native artifact、独立 C oracle、recipe 与 digest 和既有 0.10 optimizer replay 一并保留。
C compiler oracle 固定为 Clang 22.1.8，Rust SIMD oracle 固定为 Rust 1.90.0。

## Runtime protocol

Scalar regression 使用 `rotating-twelve-channel-v1`：candidate Native、current Clang、replayed
0.11 Native/Clang 与 replayed 0.10 Native/Clang，各含 checked/unchecked mode。十二条 stream
在同一进程、相同输入上运行；使用三行 rotating warm-up、二十行 rotating sample、每 sample
七次调用、固定 batch identity 与 upper-median statistic。Schema 7 保存每个实际 order、sample、
median、artifact digest 与 result；缺失 stream 不得回退到历史数值。

Vector 与 domain-fact suite 对 checked/unchecked CK 分别使用
`rotating-three-channel-v1`。Vector run 轮换 candidate CK、pinned hand-written C+SIMD 与
pinned hand-written Rust+SIMD；domain-fact run 改用未获得 CK-only contract 的 generic Clang
O3/Rust O3 source。每轮均在同一进程使用相同输入、三行 warm-up、二十行 sample、每 sample
七次调用、固定 batching 与 upper median。

Hand-written oracle 使用 architecture-specific baseline flag，禁用 fast math/contraction，且
不得使用 CK baseline profile 不具备的 CPU feature。它们获得 source language 可表达的全部
等价 precondition，并须在固定 declared valid domain 上通过 differential 与 undefined-behavior
audit。缺失、无效或测量后排除 competitor 都会使 gate 失败。

## 累积 release gate

- 保留既有 Native/current-Clang 与 0.10 replay gate：Native throughput 至少达到 Clang
  geometric mean 的 95%，单项最多慢 10%；checked proof loop 至少达到 unchecked 的 97%；
  KIR optimizer latency 相对固定 0.10 MIR optimizer 的 suite median 不超过 2x、单项不超过 3x。
- 每个架构与 safety mode 上，CK 至少达到各 vector kernel 中较快有效 C/Rust SIMD oracle
  geometric mean 的 95%，每个 kernel 至少达到自身 oracle 的 90%。
- Domain-fact suite 上，CK 在每个架构至少超过较快 generic Clang/Rust oracle geometric
  mean 的 5%。
- Unchanged scalar corpus 相对 independently replayed 0.11 的 geometric mean 最多慢 3%，
  单项最多慢 8%。
- Native object size 相对 replayed 0.11 aggregate 增长不超过 35%，单项不超过 2.5x。
  Baseline O3 source-to-object compile time 的 candidate/replay geometric mean 不超过 1.5，
  单项 ratio 不超过 2。Compile-time 样本使用已终止子进程的 user+system CPU time，因而排除
  托管 worker 被调度移出的时间，同时不移除任何编译器工作。

Runtime throughput、optimizer latency、source-to-object time、object size、memory、cold/warm
run 与 cache behavior 是分离指标。任何 threshold 都不允许削弱 diagnostic、evaluation order、
modular integer、strict float、checked first-error order、print order、semantic MIR、ABI 或
contract domain。

## 执行与证据保留

稳定 worker 测量前先运行本地 schema/unit check：

```sh
cargo bench --bench ckc_perf
cargo test --locked --test performance -- --nocapture
python3 -m unittest discover -s tests/performance -p '*_test.py'
python3 scripts/prepare-performance-replay.py --out target/ckc-perf/v011-replay
python3 scripts/prepare-performance-replay.py --baseline 0.10 --out target/ckc-perf/v010-replay
export CKC_V011_RUNTIME_BUNDLE="$PWD/target/ckc-perf/v011-replay"
export CKC_V010_RUNTIME_BUNDLE="$PWD/target/ckc-perf/v010-replay"
cargo build --release --features native-toolchain --locked
export CKC_CANDIDATE_COMPILER="$PWD/target/release/ckc"
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
python3 scripts/check-native-performance.py target/ckc-perf/results.json
```

通用 compiler-stage benchmark 写入 `build/perf/latest.summary.json` 与
`build/perf/latest.summary.md`；这些 summary 不能替代严格 schema 7 release report。

每份 replay preparation 都需要独立的全新 owned output directory、完整 Git history、Rust 1.90.0 与 pinned
LLVM/Clang 22.1.8。Replay bundle/report 保留 compiler/oracle bytes、recipe/adapter、source
manifest、measurement directory、实际 schedule/sample array、target-profile/artifact digest 与
preparation log。缺失或修改文件、symlink/path escape、identity mismatch、quick run、unknown
field 或 sample 不完整均为 hard error。

Vector corpus 包含 contiguous map/zip、strict element-wise `f64`、integer transform、target-
legal modular integer reduction、SLP、runtime no-alias versioning，以及暴露 fixed slice length
的 specialization，并覆盖 memory/compute-bound case。Size 使用 link 前成对 relocatable object；
compile-time 使用 fresh path、disabled cache、三对交替 warm-up、十五对 measured pair 与 upper
median。

修改 source、compiler identity、threshold、statistic、target profile、oracle precondition 或
exclusion rule 属于需评审的 contract change。PGO remains 0.13。Auto-Tuning remains 0.14。
