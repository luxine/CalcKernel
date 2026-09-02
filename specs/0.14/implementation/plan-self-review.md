# CK 0.14 实施计划自审

审查对象是 `specs/0.14/implementation/` 的总控、11 组 task/acceptance 与总验收。自审完全行内完成，
未使用子代理，也未修改已通过的产品门槛。

## 第一轮：设计覆盖与依赖闭环

逐节映射结果：

- CLI/scope/preconditions：阶段 08；版本/ABI：阶段 09。
- Manifest/path/environment/input map：阶段 02；runner/protocol/process/timing：阶段 05。
- legal alternatives/unit/typestate/search：阶段 03–04；measurement/validation/selection：阶段 06。
- CKTUNE01/inspection：阶段 01；source-aware replay/artifact identity：阶段 04。
- cache/interrupted sessions：阶段 08；security/failure：阶段 02、04–08。
- publication journal/overlap/recovery：阶段 07。
- tests/version/docs：阶段 09；schema 9/performance/archive：阶段 10；十作业/release gate：阶段 11 与总验收。

依赖顺序有效：schema model 先于 workload/frontier；frontier 先于 trial；snapshot+trial 先于 runner；完整
runner/frontier/artifact 先于 measurement；publication 只接收 verified decision/output；CLI 最后组装；版本、
性能、CI 在产品路径闭合后执行。

## 第一轮发现与修订

1. 初稿若把 runner 与搜索放在同阶段，会使 process failure、static search 与 measurement state 难以独立验收。
   已拆为阶段 03、05、06，并让阶段 06 只组合已冻结接口。
2. 旧 `OutputTransaction` 只有 best-effort rollback，不能承担多输出 crash consistency。已设独立阶段 07，
   并明确 ordinary transaction 不受影响、tune output 禁止降级复用。
3. 决策内部验证不能证明真实 source/KIR/artifact。已把阶段 01 self-contained checker 与阶段 04
   source-aware checker 分开，总验收同时要求二者。
4. v0.13 仍是候选而本地 main 是其祖先。总控记录了实际 ancestry，允许实现继续，但阶段 11/总验收保留
   accepted-base release blocker，没有用本地 branch 名伪装发布基线。

## 第二轮：仓库契合、TDD 与命令可执行性

- 新 `src/tune/` 符合单 crate 责任布局；optimizer/backend/CLI 只扩展已有 seam，入口 `src/bin/ckc.rs` 保持薄。
- 测试沿用八个 integration driver，新增 `tests/tune.rs` 作为第九个明确责任 driver；Native/platform 与
  performance 仍放现有 driver，不创建散乱的独立测试 crate。
- 每阶段给出 exact files、public seam、先 RED 后 GREEN 的测试名、精确执行命令与阶段 evidence 位置。
- 计划未引入新的 final artifact runtime、LLVM pipeline search、语言语法、静态/object tuning 或隐式训练。
- 长时间真实性能只在两个 required stable Linux tier 执行；本地 contract/mutation/oracle 验收不能冒充性能通过。
- exact-SHA CI 动态结果保存在 ignored evidence/CI artifact，不回写候选造成 SHA 自引用。

## 第二轮发现与修订

1. 仅按技术层拆分会让 plan replay 与 trial publish 权限交叉。阶段 04 现在用
   `NonPublishableTuneTrial` 明确 typestate，并要求 isolated rebuild 全 trial set。
2. 仅检查最终 selection 会漏掉被省略的 compile plan。阶段 03/04/06 分别要求完整 expansion、trials exact
   equality、actual-size finalist 与 required stream matrix。
3. Windows argv、short-name 与 journal primitive 容易被 Unix 单机测试遗漏。相关 task 与 acceptance 已明确
   cfg golden probe、六 host required gate 和 capability hard failure。
4. 性能 collector 可能同时成为裁判。阶段 10 已限定 collector 只写 raw evidence，独立 checker 是唯一接受者。

## 占位符、类型与门槛复核

- 所有 task 都给出实际文件、函数职责、测试名、命令、边界和 evidence；没有未决定的产品选项。
- `TuneDecision`、`CapturedWorkload`、`TunePlan`、`NonPublishableTuneTrial`、`MeasurementScheduler`、
  `TuneOutputSet` 的生产/消费顺序一致，后续阶段未改名复用不同含义。
- quick/standard/thorough、110% size、50/250 ms calibration、3/20/3 internal samples、双 validation、
  97/102/16 thresholds、2250 ms allowance、4 GiB cache、schema 9 全部门槛与 normative attachments 一致。
- 没有删除 corpus、放宽 skip、增加 retry、接受 partial decision、让 profile/measurement 建立 proof，或改变 ABI。

## 最终判定

PASS：计划与当前 v0.13 仓库模块契合，覆盖设计闭环、阶段依赖、正负测试、性能证据和最终交付；没有阻断项。
