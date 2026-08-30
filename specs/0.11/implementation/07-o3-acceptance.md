# 阶段 07 验收：O3

当前复审状态：I18 对应的阶段 07 功能缺口已修复并通过完整复验，最新证据见文末。
本阶段通过不替代 I14/I19/I20 性能门槛或同一最终 SHA 的完整 CI；见
`../review/implementation-blockers-01.md`。

## 必须通过

1. `cargo test --locked --test optimizer kir_o3_ -- --nocapture`
2. `cargo test --locked --test optimizer loop_ -- --nocapture`
3. `cargo test --locked --test optimizer generated_loop_ -- --nocapture`
4. `cargo test --locked --test optimizer guard_ -- --nocapture`
5. `cargo test --locked --test ir proof_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`

## 结构断言

- canonical provable slice loops 在 O2/O3 KIR hot loop 内无冗余 bounds guard。
- 近邻反例保留 guard 并给出稳定 conservative reason。
- LICM/induction mutation 不得改变 checked first-error、print order 或 strict f64。
- fixed-seed generated loops 在 O0–O3 observable behavior 一致。
- KIR 中没有 SIMD、unroll、versioning 或 specialization operation。

## 完成证据

执行时追加 SHA、loop fixture seed、guard count 和 verifier mutation count。

## 执行记录（2026-08-29）

- 实现提交：`4fd2ca09c30e022e627e547ae9b1c23c5f1ed4bc`
- O3 pipeline：17 项固定 pass record 全部 `verified=true`。
- loop fixtures：嵌套循环、multiple latch、break/continue 识别为 2 层 natural-loop tree；
  u32 strict-bound `+1` induction 由 entry/backedge SSA 重建。
- LICM：只 hoist 无 Memory SSA、无 ordered effect、无 checked failure 的 modular 纯值；
  checked arithmetic、runtime print、load/store/call 与 strict-f64 均不进入候选集合。
- canonical slice：同一 fixture 的 bounds guard 数为 O1=`1`、O2=`0`、O3=`0`；缺少
  `len <= items.len` 契约的近邻在 O3 仍为 `1`，reason 保持 conservative。
- loop proof：checker 同时复诊 loop header taken edge、phi entry/backedge、step、strict bound
  与 contract affine predicate；证明未开启 loop reasoning 的 O1 不会意外删除。
- generated fixture seed：`0xC0DE`，3 个 break/continue 小循环 × O0–O3 共 12 个 artifact
  全部通过 verifier，未新增 SIMD/unroll/versioning/specialization KIR operation。
- 本文件“必须通过”第 1–8 项全部通过；另执行 `cargo test --locked`，默认特性全仓
  326 个测试通过。

## I18 循环输入/不变量绑定的局部复验（2026-08-30，尚非阶段通过）

- 本节对应 `optimizer(stage-07): bind induction facts to all incoming SSA edges`，parent
  为 `cf9dc0af5dc9110b550a7ab080528ec60080edef`。具体反例见 I18 第六部分，规范与验收
  门槛未改动。
- 实际 red 之一：有 +2/+1 两条 latch 的循环错误报告 step=1。现检查所有入口和回边，
  并以真实 SSA 转发替换 slot 名推断。混合步长/中间重赋值的近邻保持保守，原嵌套和
  一致多 latch 正例、LICM、canonical slice 检查消除仍通过；loop filter 为 8/8。
- 实际 red 之二：真实回边 i+1，却用未参与回传的 i+0 证明 i 始终等于 0，独立 checker
  原来错误接受。现每条 backedge 必须绑定声明 transfer 的正确类型首结果；该错误证书
  拒绝，真实回传 i+0 的合法近邻接受。Debug/release proof filter 均为 6/6。
- 顺序全量执行 `cargo +1.90.0 test --locked` 与
  `cargo +1.90.0 test --locked --all-features`，411/532 项全部通过（Native 92、CLI 21）；
  all-feature Clippy、fmt、diff check 通过。环境为 Rust 1.90.0 / AArch64 macOS /
  LLVM+Clang 22.1.8，沿用阶段 05 的固定 overlay identity。
- 默认、全特性与 release proof 日志 SHA-256 分别为
  `2d6717080d1693324c23e9fdb81e2ddcc559c88d66f13940e8556a8053503bf0`、
  `d577558c42a2ffd2e8fc8171d396c1ceb7f22833f3af1fba5719271a156cc930`、
  `2c7b76eecf4ad4f60cf5bdbe2647e593173b8b269f02b685962f04aab9abd5ac`。
- 本节只验收上述输入绑定修复。实际 induction simplification、guard loop checker 的
  独立性、irreducible/budget 及完整阶段任务仍打开；也未重跑或代签 I14/I19/I20 性能门槛。

## I18 独立 guard 证明的局部复验（2026-08-30，尚非阶段通过）

- 本节对应 `optimizer(stage-07): independently validate strict-bound guard proofs`，parent
  为 `3b25f690bdcfa1b33b26c39c5f1fc812c4ad50ed`；反例、复诊与修复见 I18 第七部分。
- 三项实际 red 已转 green：禁止 checker 调用 optimizing loop analysis；仅重命名 slot
  不改变合法 slice 证明；false/true 两条边都进入循环体时拒绝伪安全证明。最后一项
  变异后的 KIR 仍通过 structural verifier，确实需要独立证据检查。
- 真实路径的 `i < bound`、同类型极值与实际 slice identity 组成局部 guard 前提，不
  再以重新运行归纳分析自证。原 canonical bounds 正例仍为 O1=`1`、O2=`0`、O3=`0`；
  非严格比较、步长 2、中间重赋值和错误 slice 的近邻保留必要检查。
- 默认 optimizer 87 项通过，其中四种整数宽度 × 两个级别 × 六类 strict-bound 情形
  共 48 个组合；错误 slice 同名 KIR 在 O2/O3 均保持保守。C 执行对照覆盖 O0–O3 ×
  四种 safety mode 的极值、零次迭代、checked 首错、此前写入与失败后输出槽完整性。
- 顺序执行 `cargo +1.90.0 test --locked` 与
  `cargo +1.90.0 test --locked --all-features`：417/538 项全部通过（Native 92、CLI 21）。
  `cargo +1.90.0 test --release --locked --test ir proof_ -- --nocapture`：9/9 通过。
  default/all-feature Clippy、fmt、diff check 全通过。环境继续使用 Rust 1.90.0 /
  AArch64 macOS / LLVM+Clang 22.1.8，与阶段 05 同一固定 overlay identity。
- 默认、全特性、release proof 日志 SHA-256 依次为
  `b985e40d9dde4dff1dfe4ba34ce0f5db1041554708c08be8ba411839941fdbc8`、
  `d557fe27d786fc4ce7e412af10e9d0e3cf6a3bc23f0c155479538d78b9f04112`、
  `c782061fc60416b9a7f290689d3feef3a7acd59e54e94f47bba958e4c822672d`。
- 本节关闭 guard checker 独立性及上述局部正确性缺口；actual induction-simplify、
  irreducible/fixed-budget 和完整阶段 07 仍待验收。没有重跑或放宽 I14/I19/I20 性能门槛，
  总验收清单未代签，main 未修改。

## I18 实际归纳简化的局部复验（2026-08-30，尚非阶段通过）

- 本节对应 `optimizer(stage-07): coalesce certified equivalent inductions`，parent 为
  `ece5d129dcd8d4246881a03ead5f6a5ad3d563f4`，实现及反例见 I18 第八部分。
- 实际 red：两个相同初值、相同递推的计数器在 O3 仍各保留一个 Add。现 O2 为 2 个、
  O3 为 1 个，且 `induction-simplify.changed=true`；不是更名或空 pass 记录。
- 独立等值证书核验所有入口/回边与准确 producer；参数改为保留 ValueId 的 Copy，
  输入标量边同步删除，Memory SSA 和 ordered effect 不变。四种整数宽度、双方向、
  checked/modular、多 latch 正例与不同初值/步长/漏更新反例均通过。
- 6 个 debug/release 单元测试覆盖缺失 transfer/definition、错误初值/回边、错误改写目标、
  InstructionId 耗尽、存活证书依赖与预算 0–99 的整函数原子回退。
- 固定 seed `0xC0DE_CAFE_5EED_0110` 原三个程序不变，新增三个嵌套循环、重复计数器、
  break/continue 程序，独立 Rust 参考结果参与 C/WASM/Native 的跨级别/模式对照。
  另有 C 执行测试覆盖整数极值、零次迭代、break 处值差异及 checked 首错前后的写入。
- 顺序全量默认/全特性测试为 429/551 项通过；release `--lib induction_` 为 6/6。
  all-feature Clippy、fmt、diff check 通过。Rust 1.90.0 / AArch64 macOS /
  LLVM+Clang 22.1.8，overlay identity 与前节一致。
- 默认、全特性、release induction 日志 SHA-256 依次为
  `0ee8b370c42fad0f7673c6d650fabffdcacbf0e3dd3eaea11a1bbff2af9b3075`、
  `8ab4b9a6c597a96dce2c88d6d609526550c8f4a502385eafb85ebc55f74f7db1`、
  `b3dc0233360d844ed2289f38d61dbe9920d304d126e365ee537124fd01fd1201`。
- 不可约循环识别、loop-analysis 固定预算及其消费者回退仍需继续实现验收。本节不关闭
  阶段 07 或 I14/I19/I20；无性能复测或门槛变更，main 未修改。

## I18 不可约循环/预算回退的局部复验（2026-08-30，尚非阶段通过）

- 本节对应 `optimizer(stage-07): bound loop analysis and report conservative fallback`，
  parent 为 `6d3ebdac09bd43960fbb351620c45b980196316a`；实际 red/green 见 I18 第九部分。
- 5 个新增 optimizer 测试覆盖多入口循环、自然外循环内隐藏的不可约核心、自回边
  body、嵌套循环预算扫描、不可约 SCC 全预算扫描、块存储顺序及 pipeline 消费者回退。
  实际不可约 pipeline 保留可验证 artifact，LICM/induction 的 changed 都为 false。
- 新 CLI 测试用 40 个非恒定 diamond 触发默认固定预算回退，重复两次 stdout/stderr
  字节一致，包含 `f0 pass=natural-loop-analysis reason=fixed-kir-budget-exhausted`。
  不伪造 guard 身份，不跳过 structural verifier，不使用时间预算。
- 顺序默认/全特性测试 435/557 项全部通过（Native 93、全特性 CLI 22）；release
  `--test optimizer loop_` 为 18/18。all-feature Clippy、fmt、diff check 通过。
  环境仍为 Rust 1.90.0 / AArch64 macOS / 同一 LLVM+Clang 22.1.8 overlay。
- 默认、全特性日志 SHA-256 分别为
  `904dc9e7010a6d1268bedc3ca3d4396c93bd690ca7cca2817ecdaa049c5a6094`、
  `72bc3514b1fd6c25db236095e7b3ccdbe872682283022a604e7cee6eb371f0b2`。
- 剩余阶段 07 的 LICM/完整任务自审与 I14/I19/I20 性能、同一最终 SHA 全 CI 门槛仍打开。
  本节没有性能复测、规范或门槛调整，main 未修改。

## I18 完整阶段复验与自审通过（2026-08-30）

- 实现 SHA：`31e367a2a53912b26939ac64fb989a8a8b7b9f0a`。本节替代上文所有“尚非阶段
  通过”状态，不抹去原反例或先前局部验收的范围。相关修复为 I18 第六至第十部分。
- task 1：17 项 exact O3 pass order 均 verified；nested/multiple-latch/break/continue、
  自回边及不可约内循环测试通过。不可约/固定预算回退均产生完整 unknown 结果和稳定原因。
- task 2：四宽度 × 两种 overflow mode × 八类方向/步长/严格比较共 64 组归纳事实；
  同初值/全部回边、wrap-risk、零次迭代、契约限定 slice bound 的正例和近邻通过。
  既有 widening/narrowing 固定预算、闭合 LoopInvariant 与独立 guard checker 的
  entry/transfer、taken-edge 和 SSA identity 测试全绿。
- task 3：LICM 正例直接断言 Mul 在循环外，不只统计常量；临时 SSA 来源声明由独立
  checker 核验。别名内存、递归 call、print、checked arithmetic、strict float、
  Div/Mod 保持在原控制流。C/WASM 执行对照验证零次迭代、break 绕过、checked 首错、
  之前写入与输出槽；逆序 ID/块存储、存活 producer 和整个函数预算恢复测试通过。
- task 4：等值归纳递推实际 Add 数 O2=`2` / O3=`1`，独立证书覆盖全部输入，
  同 ValueId 参数改写与 scalar-edge 修复原子完成；Memory SSA/guard 身份保持。
  归纳单元测试 6 项、LICM 单元测试 3 项，其中至少 8 个独立负例输入覆盖缺少
  transfer/producer、错误初值/步长/目标/来源、回边变异及 ID 耗尽；IR proof filter 9 项。
- task 5：canonical fixture bounds guard 为 O1=`1`、O2=`0`、O3=`0`。无契约、错误
  index/slice、非严格边界和过大步长的近邻保留必要检查及 conservative explanation。
- task 6：seed `0xC0DE_CAFE_5EED_0110` 的 6 个 generated kernel 在原 C/WASM/Native
  执行器做 O0–O3 supported-mode differential，原三个程序保留；附加 nested/equal-IV/
  break/continue 程序和显式 checked-first-error 执行测试通过。
- 本文件“必须通过”第 1–8 项全部重新执行并通过：optimizer 的 `kir_o3_`/`loop_`/
  `generated_loop_`/`guard_` 过滤器分别为 1/21/1/13 项，IR `proof_` 为 9 项；
  fmt、default Clippy 与 diff check 均 exit 0。额外 all-feature Clippy 通过。
- 顺序默认/全特性全仓测试为 443/565 项通过（Native 93、全特性 CLI 22），release
  `--lib licm_` 为 3/3。环境：Rust 1.90.0 / AArch64 macOS / LLVM+Clang 22.1.8，
  overlay manifest `b8b790dcfdd9652b1634d8d50075b1037298ec7cbcf3e7a5fefabb55d1f84874`。
- 默认、全特性、release LICM 日志 SHA-256 依次为
  `e1fd50ecfa4548d265d0b721d99a035b5c5a88af3bc074468b329220170343c7`、
  `2573ca696f80b011adb6bdd3264e5c0699637366604cc954009ba84de1191690`、
  `a35a0a4375977c008c637788829636ff229f8be6d65ea67ac0d6ec1a821a4e34`。
- 自审结论：命名 pass 有实际语义 transform；独立验证与改写绑定闭合；失败/effect/
  Memory SSA 和 strict-f64 边界未放宽；未引入 SIMD/unroll/versioning/specialization。
  阶段 07 功能验收通过。I14/I19/I20 原性能失败、阶段 11 与总验收仍打开；没有将
  功能测试替代性能结果，没有移动 main，也没有签署尚未通过的最终清单。
