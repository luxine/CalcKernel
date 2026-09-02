# 阶段 10 任务：schema 9 corpus、collector、checker 与 archive

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`; execute inline without subagents.

## 目标

实现冻结的 schema 9 性能证据链：七个预声明 tune case、search/validation/sealed release partitions、CK/C/
Rust 等价 runner 与 oracle、六/三 channel raw sampling、历史 v0.13 replay + fresh schema8 compatibility、资源/
确定性/归档证据，以及唯一 fail-closed checker。

## 仓库落点与接口

- 新增 `[[bench]] tune_perf`、`benches/tune_perf.rs`、`benches/cases/tune-cases.tsv`、
  `benches/tune/{runner.rs,workloads/*.cktune.toml}`、`benches/fixtures/tune/release-held-out.tsv`。
- 新增 `benches/oracles/tune/{manifest.toml,c/tune_oracle.c,rust/tune_oracle.rs}`，扩展现有 PGO/Vector oracle
  audit，使 CK/C/Rust 独立产生 exact `CK-TUNE-RESULT\0` digest。
- 新增 `benches/baselines/v0_13_replay.toml`、`scripts/measure-v014-performance.py`、
  `scripts/package-v014-performance-archive.py`；扩展 `scripts/{prepare-performance-replay,
  check-native-performance,audit-performance-oracles}.py`，保留 schema7/8 checker 兼容入口。
- 新增 `tests/performance/{tune_contract.rs,tune_gate_test.py,tune_oracles.rs}`，由 `tests/performance.rs`
  注册；schema 9 fixture/report mutation 不依赖实际快慢。

## TDD 顺序

1. 写 asset RED：exact 七行 tune-cases、七 manifest、五 eligible+两 domain source、training/held-out/release/
   adversarial partitions、runner/oracle/compiler/license/notices/baseline/schema recipe 全覆盖且 bytes+SHA 固定。
2. 写 correctness RED：manifest search/validation expected digest 与 release digest 从独立 CK/C/Rust canonical
   result bytes 重算；release file/digest 不得进入任何 tune manifest 或 tuning decision input。
3. 写 schema top-level/identity RED：exact 25 keys、candidate/version/SHA、v013 replay commit、toolchains、hardware、
   recipe/binary/evidence-root file identity；missing/unknown/symlink/traversal/duplicate/wrong-root 全失败。
4. 写 sampling RED：main fixed six-channel，validation/domain fixed three-channel；每 split doubling calibration+
   confirmation、3 warmup、20 measured、每 sample 7 equal batches/min/upper median/16 stable，rotation digest 重算。
5. 写 provenance RED：每 channel 的 closed BuildCommand→artifact/decision/profile/source/input foreign-key 完整；
   CK 与 oracle 都显式 unchecked bounds/overflow 且 strict defined inputs，无 CLI default 混用。Oracle
   必须在空环境中显式绑定保留且现场等字节的 Clang linker driver 与 `/usr/bin/ld`，不得依赖 PATH。
6. 写 threshold RED：相对 faster v0.13 ordinary/PGO 的 5% geo、2% selected、<=2% regression；相对 hand SIMD
   98% geo/92% each；domain >8% geo；全部 eligible case 包含 baseline selection，禁止 post-result exclusion。
7. 写 compile/size/resource RED：tune-use 10% geo/20% each，ordinary 3%/8%，artifact 110%，archive 110%，
   standard <=30 min/bounds、RSS <=2x、cache <=4 GiB；Linux direct-child wait4 receipt 两侧同协议。
8. 写 determinism RED：distinct empty cold cache 的 choice/plan/object/link/published identity 一致，warm locked
   inventory 0 compile/measure 且 bytes exact；canonical first cold 决策被 main/validation/size/resource 共用。
9. 写 historical/cumulative RED：exact v0.13 retained checker 在 detached commit 验历史 schema8；candidate 0.14
   fresh 重跑全 schema8 compatibility；两者不能互相代替或重写。
10. 写 archive RED：producer exact invocation，POSIX-pax deterministic gzip 仅含排序的 LICENSE、notices、ckc，
    modes 0644/0644/0755，member/content/compression/static-dependency receipt 完整。
11. 以保留的 native runner 在函数指针循环内部采样并严格校验 `CKPERF/1` 回执，禁止
    Python/FFI 循环开销进入 `elapsedNs`；运行 Python mutation/oracle tests、Rust performance
    contract tests 与 `measure-v014-performance.py --contract-only`；x86-64-v4 精确要求
    AVX-512 F/BW/CD/DQ/VL，缺少 CD 也必须失败。
12. 删除所有不属于 report 的 compile/profile/cache/lock scratch，并由 checker 反向证明
    evidence root 的真实普通文件集合精确等于全部 evidence `FileIdentity` 的闭包。
    支持 tier 上再执行完整 collector+checker，其他 host 不伪造性能通过。

## 实现边界

- collector 只收集 raw evidence，`check-native-performance.py` 是唯一接受者。
- internal `.cktune` 三 invocation measurement 与 external release 七 batch sample 绝不复用或混写。
- 性能 host 仅稳定 Linux x86-64-v4 和 AArch64 SVE2；缺 tier 是 required gate failure。
