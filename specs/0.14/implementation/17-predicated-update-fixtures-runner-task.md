# 阶段 17 任务：冻结 Floyd Assets 与 Native Runner

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

实现 Predicated-Update Performance Contract 1 的不可变 source、三份 input、
schema-1 tune manifest 与四个 runner protocol。该阶段只证明生成器、oracle、
profile training 和 native timing 回执正确，不收集或接受性能报告。

## 仓库落点与接口

- 新增以下 recipe-owned 文件，字节必须与
  `specs/0.14/predicated-update-performance-1.md` 第 2–4 节一致：
  - `benches/fixtures/tune/predicated_update.ck`
  - `benches/fixtures/tune/predicated-update-training.tsv`
  - `benches/fixtures/tune/predicated-update-validation.tsv`
  - `benches/fixtures/tune/predicated-update-release.tsv`
  - `benches/tune/workloads/predicated-update.cktune.toml`
- 修改 `benches/tune/runner.rs`，在现有协议外增加且只增加：
  - `--ck-predicated-tune`
  - `--ck-predicated-profile <lib> <flush> 128 113`
  - `--ck-predicated-oracle <training|validation|release-held-out> <n> <seed>`
  - `--ck-predicated-perf <lib> <split> <n> <seed> <iterations>`
- 生成器实现 Contract 1 的 SplitMix64、row-major strict-f64、零对角、ring
  connectivity、缺边正无穷；所有 `n/seed/len` 用 checked u32/u64/usize
  算术，单次 matrix 不超过 1 GiB。
- dynamic symbol 类型固定为
  `unsafe extern "C" fn(*mut f64, u32, u32)`：传入 `distance.data`、
  `distance.len=n*n`、`n`。每次 timed call 使用 fresh matrix；加载、分配、
  clone、symbol lookup、digest 在 timer 外，timer 内只有 `iterations` 次函数调用。
- profile protocol 调用 kernel 后恰好一次调用 header 提供的 flush symbol；
  tune protocol 继续产生 exact `CKTUNE/1`，direct oracle/perf 产生 Contract 1
  固定回执。
- 扩展 `tests/performance/tune_contract.rs`、
  `tests/performance/tune_oracles.rs` 与
  `tests/performance/tune_gate_test.py` 的 asset/runner 静态契约。

## TDD 顺序

1. 添加 RED `predicated_update_assets_should_match_frozen_bytes_and_digests`，
   检查五个文件、LF/final LF、source/manifest exact content、N/seed/split、
   recipe 列表以及两个 CKTUNE expected digest。
2. 添加 generator golden RED：固定前 16 个 matrix cell bits、三 split complete
   output digest分别为
   `d6105453012eedb8a8db812555f116dd69ca6a8e0242faf81f10038947581608`、
   `e21128a8623d0c072111b02c8a3f1ce3309d12f8bf0b3beddc1e0f6342dfc6c8`、
   `4d9a1612967ec78ffb3d0ecc035929bb8217cefc13dfeb3fb2e21989186b055d`。
3. 用 apply_patch 创建 exact source/TSV/manifest；实现一个共享
   `PredicatedMatrix` 生成器供四协议调用。运行 asset/golden tests 转绿。
4. 添加 protocol parsing RED：缺参数、额外参数、非 canonical decimal、错误
   split/N/seed、零 iterations、overflow、超过 1 GiB、tune map 中 release row
   全部失败且不输出成功 prefix。
5. 实现 oracle，执行 scalar Floyd，输出固定 digest；实现 profile protocol，
   验证 symbol、kernel digest、flush status 和 sole shard。
6. 实现 performance protocol与 timer boundary；添加 fake library/counter fixture
   证明每 invocation 恰好 requested iterations、fresh input、completed 等于
   iterations、digest 匹配。
7. 在有 Native toolchain 时构建真实 CK Floyd dynamic library，运行三个 oracle、
   profile 与最小 perf smoke；运行阶段命令并记录 stage-17 evidence。

## 阶段命令

```sh
cargo test --locked --test performance predicated_update_assets_ -- --nocapture
cargo test --locked --test performance predicated_update_generator_ -- --nocapture
cargo test --locked --test performance predicated_update_runner_ -- --nocapture
cargo test --all-features --locked --test performance predicated_update_native_runner_ -- --nocapture
python3 -B -m unittest discover -s tests/performance -p 'tune_gate_test.py'
```

## 边界

- 不在 runner 中选择优化、计算 acceptance ratio 或删除失败 sample。
- 不把 training/validation 输入用于 release timing，不允许 release row 进入
  CKTIMAP1。
- strict f64 使用原始 bits 做结果摘要；不得容忍 epsilon、NaN canonicalization
  或 fast math。
