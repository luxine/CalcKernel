# 阶段 05 验收：O0/O1

当前复审状态：I17 release 验证缓存已完成本机复验；I18 发现实际 SCCP propagation 缺口，
本阶段验收重新打开，以下历史记录不构成当前完整通过。见
`../review/implementation-blockers-01.md`。

## 必须通过

1. `cargo test --locked --test optimizer kir_o0_ -- --nocapture`
2. `cargo test --locked --test optimizer kir_o1_ -- --nocapture`
3. `cargo test --locked --test optimizer guard_ -- --nocapture`
4. `cargo test --locked --test ir proof_ -- --nocapture`
5. `cargo test --locked --test optimizer runtime_print_ -- --nocapture`
6. `cargo fmt --check`
7. `cargo clippy --all-targets --locked -- -D warnings`
8. `git diff --check`
9. `cargo test --release --locked --lib verifier_cache_ -- --nocapture`

## 结构断言

- O0 KIR 与 builder 输出除验证记录外相同。
- O1 pass order 与规范逐项一致，每项后都有 verifier record。
- 正例 guard 带有效 ProofId 消失；每个近邻反例保留并有确定 reason。
- checked 首错、print 和 may-fail mutation 全部拒绝非法 reorder/delete。
- 任一 invalid certificate 使 compilation failure，且 output transaction 未提交。
- Debug/release 均拒绝错误的 no-change 声明：即使 pass 声称未改动，IR、proof、guard
  rewrite 或 contract fact 的故障注入仍触发独立核验失败。

## 完成证据

执行时追加 SHA、每类 eliminated/retained guard 数与 mutation 结果。

## 执行记录（2026-08-29）

- 实现提交：`18bcf353cabd7f11734fcc9bcb17763d8eef81ef`
- O0：2 个 validator-only 用例通过；合法输入 KIR 保持不变，非法输入无 artifact。
- O1：固定 `cfg-canonicalize -> sccp-range -> check-elimination ->
  dead-code-elimination -> cleanup` 次序通过，五项 record 均由 verifier 标记为已验证。
- guard：常量安全溢出与支配的契约 slice 边界各删除 1 个；未知标量相邻例保留 1 个，
  reason 固定为 `retained: scalar safety is unknown`。
- mutation：将 `GuardSafety.condition_instruction` 改为不存在的 ID 后，独立 checker 拒绝，
  output transaction 无 artifact。
- 有序效果：`print_i32` 在 O1 DCE 后仍存在，cleanup 后 effect order 为 `[0]`。
- 验收命令：本文件“必须通过”第 1–8 项全部通过；另执行 `cargo test --locked`，
  默认特性全仓 308 个测试通过。
- 说明：额外探测的 `cargo test --locked --all-targets` 会执行 benchmark binary；既有
  `ckc_perf` 的 `emit-llvm-o3` 明确要求 `native-toolchain` feature，因此该非阶段门禁命令
  未记为通过，原生 feature 验证留在阶段 09。

## I18 常量传播事务的局部复验（2026-08-30，尚非本阶段完整通过）

- 本节随 `optimizer(stage-05): verify and apply integer constant propagation` 提交，
  parent 为 `fb020f3894e501ccb69a52364d097c3b349d208b`；完整未完成项见 I18 复审记录。
- `cargo +1.90.0 test --locked --test optimizer kir_o1_sccp_ -- --nocapture`：7/7；
  `cargo +1.90.0 test --locked --lib constant_rewrite_ -- --nocapture`：7/7。
- `cargo +1.90.0 test --locked`：360 项；`cargo +1.90.0 test --locked --all-features`：
  481 项（Native 92、CLI 21），顺序执行且全部 exit 0。新增 C 数值对照覆盖 O0–O3 和
  checked/unchecked，检验相同/不同 phi、整数比较、wrap 与 checked 失败时结果槽不变。
- Release `cargo +1.90.0 test --release --locked --lib`：12/12，包含 7 项新事务验证与
  5 项 I17 验证缓存故障注入。fmt、all-feature Clippy、diff check 全部通过。
- 首次并行运行 default/all-feature Cargo integration tests 时，共享
  `target/debug/ckc` 被 default build 覆盖，11 项 Native CLI 以缺少 feature 失败；
  保留失败日志，核对 `CARGO_BIN_EXE_ckc` 与真实 verbose identity 后改为顺序运行，
  未修改产品代码或测试判定以规避失败。
- Rust 1.90.0、LLVM/Clang 22.1.8、AArch64 macOS baseline CPU 下原 performance gate
  exit 0：unchecked Clang mean `0.9999` / V0.10 ratio `1.0009`，checked
  `1.0033` / `0.9951`，proof throughput `0.9809`，optimizer suite-median `1.1409`；
  所有 individual gate 通过。该证据不替代 I14 的远程同 worker 诊断。
- 默认/全特性日志 SHA-256 分别为
  `d181d436f9595ec8ef6980d80f947926f205bf65267cec3343e0e7f1cb68a42e`、
  `02f01a3f1274e478622c8a8b54a6f7389221e1d36485916f00c86d99ef9dad9b`；
  schema-5 performance report 为
  `36706df975d353e125bcdd7bfebf13067008e484e9e818b63e3829f3d18a6833`。
- 自审确认这一事务不移动 effect、不删除 guard、不使用 branch/contract 局部事实作为
  无条件常量。仍需完成路径/契约范围传播、条件边与 phi 本体改写、证据失效的后续验收；
  不以本节替代阶段 05 的最终完整签收。

## I18 范围证书与检查消费者的局部复验（2026-08-30，阶段仍未完整通过）

- 本节对应 `optimizer(stage-05): consume scoped range certificates for guard elimination`，
  parent 为 `ac6b29de94d6c566626ee670ebf272887f381e34`。
- 新增路径/入口契约驱动的真实 comparison rewrite；支配范围证明溢出、非零除数、
  有符号除法和定长 slice 索引安全。范围不能证明的另一分支、零、`MIN / -1`、
  `index == len` 均保留检查。每个新增优化正例先观察 guard/compare 未消除的 red。
- `cargo +1.90.0 test --locked --test optimizer kir_o1_sccp_ -- --nocapture`：10/10；
  同 target 的 `guard_`：11/11。`cargo +1.90.0 test --locked --lib`：16/16，
  包括 4 个新 scope/type/contract 故障注入、原 7 个事务测试和 5 个 release-cache 测试。
- O2 重复常量 GVN 的 red 复现 `constant instruction is missing`；按 live proof DAG
  保护后续变换依赖，O2/O3 复验通过。未使用常量仍由 DCE 删除，不靠保留全图通过验证。
- 新 C 可执行对照在 O0–O3 全部通过，覆盖 `n + 8`、除零、`MIN / -1`、slice 第 7/8 项、
  status code 和失败时结果槽不变。i32/i64 边界字面量另以 O1 artifact 严格等于 O0
  验证分析域错误的保守处理，不改写原始 checked negation。
- 顺序执行 `cargo +1.90.0 test --locked` 与
  `cargo +1.90.0 test --all-features --locked`：373/494 项全部通过（Native 92、CLI 21）。
  日志 SHA-256 分别为
  `38916e26f6130db30f03e4fc53d55f9b14818ab75d2ac707b9bec26b68c4f9bd`、
  `debe09497860414a354e68aab3cb9d6663dd507291a058dcc8f31c9c3f899fd5`。
- 最终 fmt、all-feature Clippy、diff check、release library 16/16 均 exit 0。
  Rust 1.90.0 / LLVM+Clang 22.1.8 / AArch64 macOS baseline CPU 的完整原 performance
  gate exit 0：unchecked Clang mean `0.9970` / V0.10 ratio `1.0039`，checked
  `0.9954` / `1.0030`，proof throughput `0.9985`，optimizer suite-median `1.0617`；
  所有 individual gate 通过，没有调整门槛、语料或 baseline。最终 schema-5 report
  SHA-256 为 `a377ee72396780dd73460b4c3151f98422bb1a8e5a5cfbe49ac0e022080ce288`，
  performance log 为 `09aafea0c8c99831364a719674da2a416fbb64f968a0527192957ec7768a990a`。
- 自审范围为 scalar producer/checker、证书 DAG 投影、guard transaction、后续 scalar/GVN/
  LICM/DCE preservation、对应正反例。阶段 05 的 sparse worklist、条件边剪枝、phi 本体
  改写与 CFG 相关的证据失效/重建尚未完成；阶段 07/I14/远程同 SHA 总验收仍保持打开。

## I18 稀疏工作队列的局部复验（2026-08-30，阶段仍未完整通过）

- 本节对应 `optimizer(stage-05): propagate scalar ranges with a sparse worklist`，parent
  为 `a46717ccf4254f291cfdbdadecf88ed31547c652`。
- 两项实际 red：反向块布局在固定线性预算下为 0/39 项改写；后置比较定义的合法 CFG
  未触发两个路径比较改写。相应 green 为 39/39 和 2/2，全部闭合证书经独立核验再应用。
- 新稀疏回归 3/3，含后到范围的算术消费者、相邻非恒定范围与证书确定性；原预算撤回、
  certificate mutation、checked/floating/effect 与跨阶段 preservation 测试未减少。
- 顺序全量执行 `cargo +1.90.0 test --locked`（376 项）和
  `cargo +1.90.0 test --locked --all-features`（497 项，Native 92/CLI 21），均 exit 0。
  `cargo +1.90.0 test --release --locked --lib` 为 19/19；all-feature Clippy、fmt 与
  diff check 全部 exit 0。日志 SHA-256 分别为
  `3b9b5ecaffd793c2ab6c8904a69f73aa124f3eaa86d3f7d902a581d4299366d4`、
  `16de00ce70d6e938333b864f2bbdfba26512d4cabcf88d11e53d901b484b137e`、
  `5e86847f53ca9ff70986861b023fbe7ca687bbeaf2e0ccfee85e480f0a802eab`。
- 行内自审核查使用关系完整性、队列去重、范围变化的消费者唤醒、未改变范围时的临时
  proof 截断与固定预算原子撤回，未发现本批改动的新阻断项。条件边剪枝、phi 本体改写、
  CFG 证据失效/重建及阶段 07 仍未验收，不把工作队列等同于完整 SCCP。
- 本批首次原 performance gate **exit 1**：checked proof throughput `0.9508447 < 0.97`，
  不能记为通过。Clang mean 为 unchecked `1.0007` / checked `0.9994`；V0.10 ratio 为
  `1.0002` / `0.9990`；optimizer median `1.2176`，individual 均通过。失败 report
  SHA-256 为 `3d7aaef31a7d022ff016b343cf0132cc28c2ffa33e82e8eea80fc14025cb759b`，
  benchmark log 为 `2a5c1fa3c5b07b06f16488c616490468db756d98f62a26c04678868c971e1e71`。
  已保留原始样本；详见 I19，同 SHA 全门槛仍待正式通过，不用对象一致性替代性能验收。

## I18 CFG/phi 与 region 清理的局部复验（2026-08-30，阶段仍未完整通过）

- 本节对应 `optimizer(stage-05): canonicalize CFG and materialize constant phis`，parent
  为 `10d6bba15a9b6d69e0bc963985101630b77d7db8`。
- 实际 red/green 覆盖常量 phi 定义、常量分支后的再次传播、不可达 unsafe call/print、
  空跳转块转发与未使用 slice region。空 if 从 4 块变为 2 块，标量/memory 边参数修复；
  非局部 SSA 定义、存活契约绑定、可达 checked/print/memory 效果保留。
- `cargo +1.90.0 test --locked --test optimizer kir_o1_cfg_ -- --nocapture`：6/6，
  包括存活 contract instance/fact scope 重绑和死路径无效证据先拒绝的回归。
  库测试及 release 库测试为 24/24，其中新增 phi fault/budget/preservation 用例 5 项。
- 新 C 对照实际编译运行 O0–O3 的 checked/unchecked 模式，核对 phi 参数交换、store
  顺序、常量分支与返回值。首次 harness 错把 bounds-checked ABI 当作普通返回值而编译
  失败；按现有任一 checked mode 都使用 status ABI 的契约纠正测试配置，产品 ABI 未改动。
- 顺序执行 `cargo +1.90.0 test --locked` 与
  `cargo +1.90.0 test --all-features --locked`，391/512 项全部通过（Native 92、CLI 21）。
  all-feature Clippy、fmt 和 diff check 通过。阶段 05 布尔 Copy/phi、checked 下游传播
  核查与最终验收仍待完成，阶段 07 及 I14/I19 不以本节代签。
- 默认/全特性/Release library 日志 SHA-256 分别为
  `e5f6862929fa32b6515ed6be468a0b9c6c1fc7087e61b885db1b8e750de938b2`、
  `1960179bef059d7c1e41735c90747da0f159f3036d906b75ef349d834732ff22`、
  `357eb17a45adf620a22eb5d2b48b0faa9d83666a4e64526c9a8763ec4386c08e`。
- 本批全部修改后的首次原 performance gate exit 0（Rust 1.90.0 / LLVM+Clang 22.1.8 /
  AArch64 macOS / baseline CPU）：unchecked Clang mean `0.9993` / V0.10 ratio
  `1.0015`，checked `1.0122` / `0.9863`，proof throughput `1.0372`，optimizer median
  `1.2962`；全部 individual gate 通过，Dijkstra 为 `980583 / 350000 ns`。未修改任何
  门槛、语料、baseline 或采样协议；本次通过不解释或覆盖先前 I19 的失败。
  Report SHA-256 `17ab9887d31bf5081b274f2b42173459f852b868a54b176ab1913cf4f3c80c13`，
  benchmark log `a0b31920ade972469c9053c1494f9f53c25f6b198117754b40c7006e77a53a9b`。
- 行内自审核查整批写入原子性、每条边的 phi 与 Memory SSA 参数替换、非局部定义保留、
  CFG 导入重建发生在持久 guard proof 之前、DCE 的 region 引用闭包。未发现本批新增
  阻断项；没有用本机单轮 performance PASS 代替远程同 SHA 全部 CI。
