# 阶段 18 任务：Contract 1 Collector、Closed Report 与 Checker

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

实现独立于 schema 9 的 Predicated-Update Performance Contract 1 证据链。
collector 只记录；`scripts/check-v014-predicated-update.py` 是唯一接受者，且先
要求同 SHA schema-9 report 通过。任何缺失、额外、重排、缓存复用、复合 plan、
不可达 vector body、错误阈值或未列文件均 fail-closed。

## 仓库落点与接口

- 修改 `benches/tune_perf.rs`，保留 `--task collect`，新增精确入口：

```text
--task collect-predicated-update
--out target/ckc-perf/v0.14-predicated-update-results.json
```

- 新增 `scripts/measure-v014-predicated-update.py`，责任按函数分离：
  `create_evidence_root`、`capture_file_identity`、`run_evidence_command`、
  `capture_profile_directory`、`capture_cache_scratch`、`collect_build_graph`、
  `collect_correctness`、`collect_timing_split`、`write_canonical_report`。
- 新增 `scripts/check-v014-predicated-update.py`，独立实现 typed digest 与
  closed-key validator：`check_recipe`、`check_command`、`check_directory`、
  `check_cache_scratch`、`check_publication_locks`、`check_profile`、
  `check_decision_and_attestation`、`check_timing_split`、
  `check_evidence_inventory`。不得 import collector。
- 新增 `tests/performance/predicated_update_gate_test.py`，用独立 fixture builder
  构造最小合法 report/evidence，并为每个 top-level object、nested cardinality、
  digest、foreign key、ratio operand、order、receipt、cache/profile/lock/inventory
  变异生成独立 rejection test。
- 扩展 `tests/performance/tune_contract.rs` 与 `tests/contracts/ci.rs`，冻结 recipe
  文件列表、bench task、checker command 和上传路径。

## Collector TDD 顺序

1. 写 RED 验证 CLI 只接受 exact task/out，candidate checkout 必须 clean，
   evidence sibling create-new 且无 symlink；collector 源码不得包含 95/100、
   102/100 或 acceptance 字样。
2. 实现 candidate compiler/runner immutable evidence copies和 Section 5 七命令；
   `argv[0]` relative no-follow resolution必须等字节于顶层 identity。
3. 实现 profile directory retained descriptor：空 before receipt、training 后
   sole shard、merge 直接消费该 shard、final profile inspect identity一致。
4. 实现四个 Linux cache scratch：`XDG_CACHE_HOME=E/cache/<command>`、实际
   `cache/<command>/ckc`、create-new empty before、独占 lock、live after inventory
   与 canonical receipts。
5. 捕获 `pgoTuned` decision/artifacts 的全部 persistent publication lock；replay
   不得伪造 lock。比较 tuned/replay common role bytes与 attestation bytes。
6. 实现 oracle、doubling calibration/confirmation、3 warmup、20 measured、每 row
   每 channel 3 calls/min、candidate-SHA order rotation；保留全部 CommandEvidence
   和 CallReceipt，不做 threshold 判定。
7. 写 canonical JSON 到 final path前，先建立所有 stdout/stderr/receipt identity，
   再验证 evidence-root普通文件 inventory 自洽；报告写入外部指定 path，不让
   自身形成递归 identity。

## Checker TDD 顺序

1. 实现 exact JSON parser：拒绝 duplicate/unknown/missing key、非 U64 integer、
   非 canonical path/digest/order；先调用 schema-9 checker 并比较 SHA/compiler/
   toolchain/hardware。
2. 独立重算 recipe、SplitMix64 golden/result digest、Git-SHA Text order、所有
   CommandEvidence argv/env/io/status 和 profile/cache/publication foreign key。
3. 独立解析 CKTUNE01，要求 selected Candidate 恰好一个 PlanChoice，其重建的
   Loop SIMD UnitVariant 恰好一个 target SiteAlternative；比较 attestation ids、
   VF/UF/minimum/pre/post。minimum<=128，且 N/slice facts 让全部 guard 真并执行
   vector chunk。
4. 重算 samples/upper median/16-of-20；用任意精度整数检查 validation
   `tuned*100 <= pgoOnly*102` 与 release `tuned*100 <= pgoOnly*95`。
5. 从所有 evidence FileIdentity 反向建立路径集合，要求等于 live evidence tree
   中全部 regular no-follow file；验证 cache live after、profile sole shard、
   tuned locks同父目录与 exact 40-byte内容。
6. 逐类运行 mutation tests，确认每个变异只因预期 invariant 失败；增加两项
   绕过回归：复合 plan 由其它 choice 提速、minimum=2048 scalar fallback。
7. 在支持 tier 的本机执行一次完整 collect+checker；不支持 tier 时只签署
   contract/mutation，本阶段真实性能保持待 stable host。

## 阶段命令

```sh
cargo test --locked --test performance predicated_update_contract_ -- --nocapture
python3 -B -m unittest discover -s tests/performance -p 'predicated_update_gate_test.py'
cargo test --locked --test contracts ci_v014_predicated_update_ -- --nocapture
cargo bench --features native-toolchain --bench tune_perf -- --task collect-predicated-update --out target/ckc-perf/v0.14-predicated-update-results.json
python3 scripts/check-v014-predicated-update.py target/ckc-perf/v0.14-predicated-update-results.json --schema-nine target/ckc-perf/v0.14-results.json
```

## 边界

- collector 无接受逻辑；checker 不重跑 timing、不删除 case、不选择 rerun。
- 不修改 schema 9、CKTUNE01、Manifest Schema 1 或 ABI。
- 本机缺 x86-64-v4/AArch64-SVE2 只影响真实性能签署，不允许把 required stable
  job 改成 skip；contract/mutation 必须本地通过。
