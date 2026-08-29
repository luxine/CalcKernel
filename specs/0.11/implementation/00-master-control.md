# CK 0.11 事实驱动优化器实施总控

## 目标与固定输入

本计划实现通过第二轮对抗性审查的以下规范，二者语义等价且同为约束输入：

- `specs/0.11/fact-driven-optimizer.md`
- `specs/0.11/zh-CN/fact-driven-optimizer.md`

实施分支为 `feat/fact-driven-optimizer-0.11`，独立 worktree 为
`.worktrees/fact-driven-optimizer-0.11`。设计基线是 `794250f`，性能与兼容性所用的
固定 0.10 实现基线是 `df816502876fba41676f9ebc190e4fadd18cd5a5`。不得用移动
分支替代该 identity。

本任务只形成可审查的 0.11 候选分支：不得合并 `main`，不得创建或移动 tag，不得
创建 Release。为了运行六 host 验收，可以推送该 feature branch 并用显式
`workflow_dispatch` 执行 CI；这不授权创建 PR 或合并。

## 不可变约束

1. 完全行内实施，不使用子代理。
2. 严格 TDD：每个行为先写最小失败测试并实际观察预期失败，再写最小实现，再重构。
3. 不改变 0.10 已冻结的求值顺序、checked 首错顺序、print 顺序、严格浮点、C/WASM/
   Native C ABI shape、`emit-mir` 文本或 artifact transaction。
4. KIR 是唯一 target-neutral 优化 IR。开发期可以保留 shadow comparison，但最终阶
   段必须删除 backend 直读优化 MIR 的正式路径。
5. 每个命名 KIR pass 后运行独立 verifier；任何 stale fact/proof 都是编译器错误，
   不能 fallback 到未验证机器码。
6. 不实现 0.12+ 的 SIMD、unroll、specialization、PGO、Auto-Tuning 或 fast-math。
7. 不为通过测试而降低语料、门槛或检查强度。只有规范存在真实错误时才能同步修订
   中英文规范、对应阶段文档和测试，并在提交说明中写明反例。
8. 所有生成物进入已忽略的 `target/` 或 `build/`，不提交测量输出、LLVM prefix 或
   agent 工作文件。

## 固定流水线

```text
Source/AST/contracts
  -> CheckedProgram
  -> stable semantic MIR
  -> artifact roots + reachability/capability check
  -> KirBuildConfig(consumer, overflow, bounds, sanitizer)
  -> scalar SSA + region Memory SSA + explicit ordered effects/guards
  -> facts/effects/proof certificates + verifier
  -> O0/O1/O2/O3 KIR pipeline
  -> C | WASM | audited Native LLVM
```

`emit-mir` 停在 semantic MIR。其他生成 artifact 的命令必须通过上图 KIR 路径。

## 阶段顺序

| 阶段 | 交付物 | 前置 |
| --- | --- | --- |
| 01 | unsafe/contract/effects 前端与稳定诊断 | 无 |
| 02 | consumer/mode-specific KIR、SSA、guard、printer、structural verifier | 01 |
| 03 | Fact/Proof certificate、标量抽象域与独立 proof checker | 02 |
| 04 | region/alias/Memory SSA、SCC effect summary、CK2016 | 03 |
| 05 | O0/O1：CFG、SCCP/range、携证检查消除、DCE | 04 |
| 06 | O2：effect-aware inline、GVN、load forwarding、DSE | 05 |
| 07 | O3：loop/induction、LICM、induction simplify | 06 |
| 08 | C 与 WebAssembly 后端迁移到 KIR | 07 |
| 09 | Native LLVM KIR lowering、fact map/audit 与属性白名单 | 08 |
| 10 | CLI inspection、contract sanitizer、run/build/cache/header | 09 |
| 11 | 全仓切换、兼容/生成/性能/六 host CI、0.11 候选文档与版本 | 10 |

必须依次执行。每阶段先完成 task 文档，再逐条通过 acceptance 文档，才能进入下一阶
段。阶段内允许小提交；第 04、07、10 阶段结束建议形成架构检查点提交。

## 阻断处理

遇到失败时先确定是实现缺陷、测试缺陷、环境问题还是规范反例：

- 实现缺陷：保持测试，修复实现；
- 测试缺陷：只有测试与固定规范不一致时修复，并保留能复现原误判的说明；
- 环境问题：记录精确命令、host、toolchain identity，修复可复现环境，不跳过 gate；
- 规范反例：先在 review 文档复诊，再同步修订双语规范、master、相关 task/acceptance，
  不得只改一个语言版本或悄悄降低门槛。

## 提交与证据

计划与规范修订先单独提交。之后每阶段提交信息使用
`optimizer(stage-N): <imperative outcome>`。每个 acceptance 文档底部在执行时追加：

- 验收提交 SHA；
- 执行命令；
- exit status 与关键计数；
- 适用的 target/toolchain identity；
- 若有修订，链接到规范反例与修订提交。

最终以 `99-final-acceptance.md` 为唯一总验收清单。最终提交后确认 `main` 仍停在任务
开始时的 SHA，并等待用户审查。
