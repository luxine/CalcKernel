# CK 0.13 实施计划自审

日期：2026-09-02
对象：`specs/0.13/implementation/00`、`01`–`11`、`99`
结论：**通过；两轮依赖、仓库契合与验收可执行性自审后无阻断项。**

## 第一轮：设计覆盖与依赖闭环

- [x] 五个 release deliverables 都有唯一主 owner：profile wire/CLI（01）、collection（02–03）、
  profile-guided optimization（04–06）、multiversion/dispatch/artifact（07–09）、release/performance/CI
  （10–11），且最终由 99 累计复核。
- [x] 两轮审查修订被显式落实：profile identity 使用 executable/library topology而非物理 kind；
  O2 ordinary structural passes完全 profile-blind，late layout之后只有 closed target repair allowlist；
  full flush suffix固定为 canonical identity bytes的完整 SHA-256。
- [x] raw-shard-only merge、saturation/edge equation、histogram bucket全域下界、directory identity/
  no-follow、sticky flush、multiversion object rejection、separate modules/no LTO、cache bundle atomicity均有
  task、局部 acceptance 和 final acceptance三层 owner。
- [x] 阶段依赖无环：wire -> KIR topology -> runtime -> analysis -> O2/O3 -> variant plan -> dispatch ->
  artifact/cache -> release identity -> performance/CI。后阶段不反向定义前阶段 schema。
- [x] 普通 off路径、Native ABI 1/Runtime ABI 2、strict-f64、checked first-error、effect/print、sanitizer/
  C/Wasm/JIT exclusions贯穿阶段和总验收，没有用 profile observation替代 static safety proof。

## 第一轮发现与修订

1. **发现：** generation lifecycle和真正 PGO final optimization跨阶段 03/04，若阶段 03 把
   `pgo build`称为完整优化会形成循环前置。
   **修订：** 阶段 03只验收真实 instrumentation/child/merge/transaction，并明确 final use可暂走未加权
   普通 O3骨架；阶段 04接入 profile application并重跑 convenience-path handoff，阶段 06再要求 final
   artifact真实经过 O3 PGO pipeline；最终 99只接受真正 profile-guided final artifact。
2. **发现：** variant generation、runtime selection和multi-object packaging落在三个既有仓库边界，
   合并为一个阶段会让 feature checker、detector和cache失败互相遮蔽。
   **修订：** 拆为 07 planner/KIR bundle、08 dispatcher/runtime、09 artifact/cache/CLI，且每阶段明确
   不能代签下一阶段。
3. **发现：** profile runtime若塞入既有五个 ordinary runtime object数组，会改变普通 artifact identity
   并可能泄漏 collector。
   **修订：** 阶段 03要求独立 profile-runtime object/hash/identity；阶段 08 dispatch runtime同样独立，
   阶段 09按 mode精确选择 named support objects。

## 第二轮：仓库契合、TDD 与命令可执行性

- [x] 新 workload profile放 `src/profile/`，不会与现有 `src/ir/kir/profile.rs::KirTargetProfile`
  混名；site/effect/mapping接现有 KIR model/build/print/verify；proposal/checker复用 optimizer analysis/
  kir_passes/transaction/audit边界。
- [x] O2/O3 Native工作接现有 `native/bridge/ckc_llvm.*` 和 `src/backend/llvm`；named objects接现有
  artifact/lld/archive；cache 4接现有 `src/cli/cache`；CLI output复用 `OutputTransaction`而非另起写盘协议。
- [x] 每阶段 task都有 observable RED -> minimal GREEN -> refactor顺序、精确文件与明确“不做”；每份
  acceptance都有实际命令、非零 filter要求、结构断言和动态 evidence位置。
- [x] default-feature阶段与 Native-only阶段分开；阶段 03后固定 LLVM 22.1.8 prefix不可 skip；阶段 11
  将非本机真实路径交给六 host而不是用本机 fixture冒充。
- [x] benchmark schema固定为8、replay固定 exact SHA；training/held-out/adversarial、oracle precondition、
  resolver boundary、所有设计 threshold与0.12累计门槛完整进入独立 checker和CI owner。
- [x] exact-SHA证据不回写被测提交；远程长任务间隔查询；最终不merge/tag/release，符合交付授权。

## 第二轮发现与修订

1. **发现：** 仅写 `cargo test --test native` 会掩盖 filter拼写错误或零测试。
   **修订：** 每阶段 acceptance 明确所有 filter必须非零并记录 count；最终仍跑完整 default/all-feature。
2. **发现：** 计划中的 stage audit target目录由实现生成，脚本不会自动创建。
   **修订：** 各 task把 real fixture artifact生成列为 TDD输出，acceptance bundle统一位于已忽略
   `target/acceptance/v0.13/`；执行时先构建fixture再调用audit，不允许空目录代签。
3. **发现：** physical artifact kind既不能进入 profile identity又必须防 cache alias。
   **修订：** 阶段 01只保留semantic topology；阶段 09强制把physical kind和topology compatibility共同
   写入cache/artifact key，并增加dynamic/static/object跨包装profile兼容与executable mismatch测试。

## 最终判定

没有未分配设计义务、循环依赖、与当前仓库冲突的模块边界、零测试验收漏洞或隐含降门槛。
计划可以提交并按 01–11 顺序完全行内执行；若实现暴露真实规范反例，必须按总控阻断流程复诊，
不得通过修改验收标准绕过。
