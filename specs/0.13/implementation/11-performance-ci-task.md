# 阶段 11 任务：schema 8 PGO/multiversion 性能、size/time 与十作业 CI

## 目标

实现并执行 fail-closed benchmark schema 8：固定 training/held-out/adversarial corpus、exact 0.12 replay、
等价 Clang/Rust PGO oracle、ordinary/PGO/multiversion/combined/selected-direct channels、generation 开销、
artifact/compiler/archive size 与 source-to-object time，并在不增加 bootstrap matrix 的前提下扩展现有
exact-SHA 十作业 CI。

## 仓库落点

- 新增 `benches/pgo_perf.rs`、`benches/cases/pgo-cases.tsv`、training/held-out/adversarial fixtures、
  oracle adapters/manifests；修改 `Cargo.toml`、`benches/{ckc_perf.rs,runtime_replay.rs,summary-schema.md}`。
- 修改 `scripts/{prepare-performance-replay.py,check-native-performance.py,
  diagnose-native-performance.sh,audit-performance-oracles.py}`，必要时新增 deterministic trainer/
  capability/feature-containment helpers。
- 修改 `tests/performance/**`、`tests/performance.rs`、`tests/contracts/ci.rs`。
- 修改 `.github/workflows/ci.yml` 与 bootstrap action cache/evidence payload；保持十个 required jobs。

## TDD 顺序

1. 写 schema 8 RED：candidate/replay/toolchain/source/training shards/final profile/target-set/variant object/
   sample order/hardware/capability/recipe/adapters 全部 exact bytes+SHA；missing/unknown/extra/mismatch fail。
2. 写 workload split RED：training 与 held-out 固定分离，correctness 还含 adversarial；timed PGO 只用
   held-out。同 source/mode/input/batch，动态加载/symbol resolve/detector resolve 在 steady timing 前。
3. 写 sampling RED：保留既有 warmup、rotating order、upper median、stability、fail-fast、等价规则；
   resolver untimed record证明只运行一次，stability fail 是 invalid evidence而非任意重跑许可。
4. 写 replay/oracle RED：exact 0.12 commit `d8380507...`、LLVM/Clang 22.1.8、Rust 1.90.0；Clang/
   Rust PGO 使用相同 train/eval 与 safety/strict-float precondition，source/recipe/binary/UB audit完整。
5. 写 ordinary regression RED：0.13 no-PGO baseline/native 相对 exact 0.12 replay geo slowdown <=2%、
   individual <=5%，并保留全部 cumulative 0.12 hand-SIMD/domain/scalar/size/time gates。
6. 写 PGO RED：PGO use 对同 0.13 CPU policy geo improvement >=5%，held-out individual slowdown <=3%；
   generation execution <=5x ordinary；所有训练 shard/profile digest可复现。
7. 写 multiversion/dispatch RED：eligible suite dispatch 对 portable baseline geo >=8%、individual slowdown
   <=3%；steady dispatch >= selected-direct geo 98%、individual slowdown <=5%，resolver once。
8. 写 combined/oracle RED：combined 相对 faster PGO-only/multiversion-only geo slowdown <=2%、individual
   <=5%；CK >= Clang/Rust PGO oracle geo 95%、accepted kernel individual >=90%。
9. 写 compile/size RED：PGO/multi/combined source-to-object geo <=1.5/2.5/3.5，individual <=2/3/4；
   artifact aggregate <=1.25/2/2，individual <=1.5/2.5/2.5；distributed ckc archive <=0.12 +15%。
10. 建立 x86-64/AArch64 代表 suite，覆盖 branch/layout、call/constant/length、trip/unroll/SIMD、memory/
    compute bound、v3/v4/SVE/SVE2 eligible；先 semantic/differential/UB/feature audit，后计时且不可测后排除。
11. 更新 CI payload：quality 承担 schema/unit/mutation/docs/cache；Native integration 真走 generate/merge/
    use/final audit；六 host 走 ABI/fallback/detector/object；两 performance worker 发布 capability manifest
    并执行完整 gate。quality 使用 `tests/oracles/typescript` 内 provenance/commit/tree/source-manifest
    固定、lockfile 完整约束的测试专用 TypeScript oracle，在校验后本地构建并执行既有 live
    C/WASM/CLI/fixture differential gate；不得依赖同 owner 私有仓库凭据或以移除 oracle 代替修复。
    required tier 缺失必须失败，不能 skip。
12. 本地先跑 schema/checker/correctness；稳定 worker执行昂贵 benchmark。失败先诊断实现/测量/环境，
    不改变 threshold/statistics/corpus/oracle。feature branch push 后显式 dispatch，间隔查询远程状态。

## 执行策略

- 每份 report 先 canonicalize/hash，再由独立 Python checker读取；benchmark 本身不能宣布通过。
- diagnostic 只检查本轮实际 artifact/report，不能重建、重新计时或替代 required gate。
- exact-SHA CI 运行期间若本地提交任何变更，旧 run 作废并对新 SHA 重跑。

## RED/GREEN 证据

保存 schema mutation、correctness/profile/capability/artifact digests、完整 sample 与 checker output 到
`target/acceptance/v0.13/stage-11/` 和 CI artifact；不得把大报告或动态 run id 提交到仓库。
