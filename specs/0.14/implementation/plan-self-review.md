# CK 0.14 实施计划自审

审查对象是 `specs/0.14/implementation/` 的总控、19 组 task/acceptance 与唯一总验收。
阶段 01–11 是当前分支已经落地、但必须由最终 SHA 回归的累计基础；阶段 12–19 是本轮严格执行链。
本次拆分和自审完全行内完成，未使用子代理，也未降低通过六轮对抗性审查的产品或性能门槛。

## 第一轮：设计覆盖与依赖闭环

累计基础映射：

- CKTUNE01/inspection：阶段 01；manifest/snapshot：阶段 02；frontier/search：阶段 03。
- nonpublishable trial/source-aware replay：阶段 04；runner/process：阶段 05；measurement/selection：阶段 06。
- publication journal：阶段 07；cache/CLI：阶段 08；release identity/docs：阶段 09。
- schema 9/collector/checker/archive：阶段 10；原十作业 exact-SHA topology：阶段 11。

本轮兑现映射：

- Profile Runtime 的 MSVC 原子与 Linux/Darwin durable publish：阶段 12。
- host-resolved Native artifact path 与 LLVM void call：阶段 13。
- predicated same-place update 的 discovery、Memory SSA、alias/effect/checked 合法性：阶段 14。
- vector compare/select/unmasked store 物化、独立 checker 与 LLVM lowering：阶段 15。
- single-choice tune integration、source-aware attestation 与 exact replay：阶段 16。
- 冻结 Floyd source/inputs/manifest、SplitMix64 generator 与四协议 runner：阶段 17。
- 独立 Contract 1 collector、closed report、mutation suite 与唯一 checker：阶段 18。
- 阶段 01–19 全量回归、十作业 exact-SHA CI 与最终交付：阶段 19 和总验收。

依赖顺序闭合：平台与 bridge 根因先修复，之后建立候选合法性，再物化与独立复核，然后才允许 tuner
选择和签发 attestation；冻结正确性资产与 runner 先于 collector/checker；最后才接入 CI 与签署最终 SHA。
任何阶段都不能靠后续 measurement 建立静态 proof，也不能让旧 schema 9 代签新的 Contract 1。

## 第一轮发现与修订

1. 原阶段 10–11 的措辞容易被误读为已经拥有最终签署权。已标明它们只是累计基础，阶段 19 必须在最终
   SHA 上重放，并由 `99-final-acceptance.md` 唯一签署。
2. 把平台修复和优化实现混在同阶段会掩盖跨 host 根因。已拆为阶段 12–13，冻结 Runtime ABI 2、Bridge ABI 4
   与公开状态码，并分别验收 profile durable publish、artifact path 和 void call。
3. predicated-update 若一次完成 discovery/lowering/tuning，会让错误 Memory SSA 或不可达向量体逃过审查。
   已拆成阶段 14 discovery、15 materialization/checker、16 tune/attestation 三个有单向依赖的阶段。
4. 仅扩展 schema 9 会允许通用 case 或复合 plan 贡献 5% 收益。阶段 17–18 保持独立 Contract 1，要求 exactly
   one selected choice/unit/site，并以 source-aware facts 证明 minimum、guards 与真实 vector chunk。

## 第二轮：仓库契合与接口复核

- Profile Runtime 继续沿用 `native/profile_runtime/{common,platform,include}` 与 provenance；没有创建第二套 runtime。
- optimizer 继续扩展 `analysis/vectorize.rs`、KIR vector materializer、独立 `vectorize_check.rs` 与已有 LoopSimd
  payload；KIR 3、CKTUNE01、Manifest Schema 1 不加字段。
- Native lowering 只改现有 bridge/KIR LLVM seam；CLI 测试改用已有 `NativeArtifactPaths` 权威。
- tune attestation 使用已有 tune/replay 测试 driver；performance 使用已有 `tune_perf` bench、runner、contracts
  driver和 Python gate 目录，不新建重复入口。
- CI 保持 quality、native-integration、六 native-host、两 performance 共十 job；新增门禁进入原 required job，
  不使用 `continue-on-error`、optional selector 或第十一个旁路 job。
- 动态证据只写 ignored `target/acceptance/v0.14/`、`target/ckc-perf/` 或 CI artifact，避免候选 SHA 自引用。

## 第二轮发现与修订

1. Linux cache namespace 曾把 XDG base 与 CK 实际子目录混为一谈。阶段 18 现在明确
   `XDG_CACHE_HOME=E/cache/<command>`，实际闭合根为 `cache/<command>/ckc`。
2. publication lock 曾被错误要求出现在 replay。阶段 18 只要求 pgoTuned 发布产生 persistent locks，replay
   只验证 common-role artifacts 与 attestation byte equality。
3. candidate SHA、可执行文件和 profile directory 曾使用不匹配的 identity 类型。计划现在区分 canonical
   Git-SHA text、regular-file `FileIdentity` 与目录 `DirectoryEvidence`。
4. 测试计划曾引用不存在的 source-checker driver。阶段 16 改为扩展现有 `tests/tune/replay.rs`；阶段 14–15
   使用已有 optimizer driver，阶段 17–18 使用已有 performance driver。

## TDD、命令与门槛复核

- 每个本轮 task 都指定真实仓库落点、冻结接口、先 RED 后 GREEN 的顺序、精确 selector 和阶段 evidence 目录。
- 阶段 12–18 的 acceptance 将 contract/mutation 正确性与需要 stable CPU 的真实性能分开；本机能力不足不能
  伪签或跳过远程 tier，但不阻止完成可在本地真实验证的实现。
- Contract 1 保留固定 source/input/manifest digest、四 runner protocol、3/20/3 采样、upper median、16-of-20、
  validation 102/100 和 release 95/100；没有 retry、post-result exclusion 或阈值降级。
- collector 只记录，checker 不计时；checker 先验证同 SHA schema 9，再闭合 recipe、commands、profile、cache、
  locks、decision/attestation、timing 和 regular-file inventory。
- final executable/dynamic library 不携带 runner、tune dispatch runtime 或新 shared dependency；普通未 opt-in 路径
  不访问 tune manifest/cache/decision，也不改变 optimizer 行为。

## 占位符与冲突扫描

- 所有阶段都给出确定文件、责任、失败行为、命令和验收主体；没有待定产品选项或“稍后实现”占位符。
- 阶段 01–11 的已落地契约未被重定义；阶段 12–19 只补齐已审查通过的优化兑现与真实平台阻断。
- `00-master-control.md`、阶段 acceptance 与 `99-final-acceptance.md` 对执行范围、唯一签署权、exact-SHA、
  不合并 main、不创建 tag/Release 的表述一致。

## 最终判定

PASS：19 阶段计划与当前单 crate、optimizer、Native bridge/runtime、tune、performance 和十作业 CI 结构契合；
覆盖六轮对抗性审查关闭的全部问题，依赖与证据链闭合，无阻断项或明显隐患，可以进入文档提交与行内实现。
