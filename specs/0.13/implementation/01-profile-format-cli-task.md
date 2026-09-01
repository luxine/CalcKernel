# 阶段 01 任务：profile identity、格式、merge/inspect 与 CLI 闭集

## 目标

建立与 LLVM profile 无关的 CK workload profile 基础层：canonical `CkProfileIdentity` schema 1、
`CKPART01`/`CKPROF01` 无歧义编码、受限 parser、raw-shard-only deterministic merge、inspection
schema 1，以及 `ckc pgo merge|inspect` 和所有 PGO flag 的解析/组合验证。此阶段不插桩、不执行
profile-guided 优化。

## 仓库落点

- 新建 `src/profile/{mod.rs,identity.rs,format.rs,merge.rs,inspect.rs}`，由 `src/lib.rs` 导出稳定
  compiler-owned 数据模型与错误类别。
- 修改 `src/cli/{mod.rs,args.rs,commands.rs}`，必要时新增 `src/cli/pgo.rs`；保持 `build-llvm`
  明确拒绝 PGO/multiversion。
- 新建 `tests/profile.rs`、`tests/profile/{format.rs,merge.rs,inspection.rs}`，扩展
  `tests/cli/{commands.rs,kir_inspection.rs}` 与 docs/CLI contract tests。

## TDD 顺序

1. 写 CLI RED：`pgo merge/inspect/build` 的位置参数、`--json`、`--profile-out`、
   `--pgo-generate`/`--pgo-use`、`--cpu multiversion`、O-level、consumer/kind/sanitizer 矩阵；未知、
   重复、互斥、alias output 和 deprecated `build-llvm` 组合必须在创建输出前失败。
2. 写 identity RED：冻结所有 schema/ABI/target/topology/cost/resource 字段，canonical bytes 与完整
   lowercase SHA-256；路径、注释、时间、PID、物理 dynamic/static/object kind 不进入 profile identity。
3. 写格式 golden/mutation RED：big-endian tag/length/order/digest、UTF-8、trailing bytes、unknown/
   duplicate field、checked allocation、512 MiB、site/shard/bucket/candidate 上限全部 fail-closed。
4. 写 merge RED：只接受 completed `.ckprof-part`；final profile、symlink、递归目录、duplicate run/
   content、identity/site collision/不一致、饱和与错误 spanning-tree equation 均拒绝；temporary 只计数。
5. 写 determinism RED：输入路径/目录/枚举/map/shard 顺序不同但 raw shard 集相同，final bytes
   完全相等；final aggregate 不含 UUID、文件名、时间与本机路径。
6. 写 inspect RED：text/JSON 共享同一个 untrusted parser，JSON schema/version/order 固定；完整
   identity、coverage、runs/shards、unknown/saturated/histogram 与 compiler compatibility 可观察。
7. 以最小 canonical writer、cursor parser、checked-length helpers 和稳定错误枚举实现 GREEN；再
   将 CLI 的普通路径与 PGO 路径整理成显式闭集，禁止 silent fallback。

## 实现边界

- site descriptor 的最终生成由阶段 02 完成；阶段 01 用完整测试 fixture 表验证 wire contract。
- 不读取 LLVM `.profraw/.profdata`，不提供 mismatch override、nested merge、隐式归一化或 telemetry。
- `CkProfileIdentity` topology 只有 `native-executable`/`native-library`；物理 artifact kind 由阶段 09
  的 artifact/cache identity 持有。

## RED/GREEN 证据

把首个失败命令、精确失败断言、GREEN 命令/test count 和 canonical fixture SHA 写入
`target/acceptance/v0.13/stage-01/`。fixture 需要代码审查时提交其 canonical bytes/期望 digest，
不得提交运行时 profile 输出。
