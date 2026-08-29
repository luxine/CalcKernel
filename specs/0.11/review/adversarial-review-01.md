# CK 0.11 事实驱动优化器：第一轮对抗性审查

## 审查边界

本轮只审查 `specs/0.11/fact-driven-optimizer.md` 与其简体中文版本是否满足：

- 语言语义、ABI、安全模式与优化事实之间逻辑闭环；
- 能够基于当前 AST → CheckedProgram → MIR → backend 流水线实现；
- 证明、backend 属性和验收门槛具有可判定的责任边界；
- 不把 0.12 及以后 SIMD、特化、PGO、Auto-Tuning 工作倒灌进 0.11。

审查基线是提交 `794250fab28e78c0cf1c944d9eb0f342bb093d9e`。默认
`cargo build --locked` 与 `cargo test --locked` 均通过。

## 结论

**结论：暂不通过，存在 8 个阻断项。**

这些问题不会推翻既定架构，但如果保持原文直接拆解计划，会在实现中留下可导致错误优化、
不一致 artifact 或不可验证验收的空白。修订范围应限于澄清责任边界和补齐必须的语义，不改变
已经确认的 0.11 功能目标。

## 阻断项

### B01：KIR 构建缺少安全模式与 artifact 可达性边界

规范要求 bounds/overflow guard 在 KIR 中显式存在，同时语义 MIR 保持稳定；但当前 guard 是
C/LLVM backend 根据 `overflow_mode`、`bounds_mode` 在 lowering 时生成的，而且 artifact
可达性裁剪发生在 MIR 优化之后，部分命令甚至路径不一致。若不定义模式和 consumer 的进入点，
同一 MIR 无法唯一决定 KIR 中应出现哪些 guard，未被目标 artifact 使用的 runtime call 也可能
污染 effect summary 或错误拒绝 C/WASM artifact。

需要固定为 consumer 与安全模式参数化的流水线，并规定裁剪、KIR 构建、优化和 backend 的顺序；
`emit-kir` 也必须有确定的 root 和模式规则。

### B02：`effects` 上限与完整 effect summary 的关系不闭合

契约语法只能表达 `read`/`write`/`readwrite`，而摘要还包含 runtime print、may-fail 与 unsafe
call。原文“effect ceiling”没有说明 `effects none` 是“无任何效果”还是“无外部可达内存访问”，
也没有定义 local stack、raw pointer、无法映射到命名 slice 的访问如何覆盖。这样既无法稳定产生
`CK2016`，也无法安全映射 LLVM `readonly`/`writeonly`/`memory(...)`。

需要将该子句严格定义为参数映射的外部可达内存访问上限；其他效果始终推导、不得被该语法隐藏；
无法映射的外部内存访问必须落入保守的 `all` 集合。

### B03：成对 `noalias(a, b)` 不能直接推出参数级 `noalias`/`restrict`

原文允许任意成对 `noalias`，又把 LLVM 参数 `noalias` 和 C `restrict` 列为映射目标。只证明
`a` 与 `b` 不重叠，并不能证明 `a` 与第三个 pointer root、返回值、capture 或其他可达内存均
满足参数级承诺。直接放大将产生错误优化。

需要限定：成对事实默认只能形成对应访问的 alias scopes；只有证明参数对所有相关 pointer root
满足 backend 的完整属性语义（包括 capture/return 约束）时，才能使用参数级 `noalias` 或 C
`restrict`。

### B04：证明校验器“独立”的可执行含义不足

原文要求 independent verifier，却没有防止 verifier 直接调用提出变换的同一 range/alias/effect
分析重新询问结论。若分析器与 verifier 共享同一错误路径，`ProofId` 只是标签，不能构成独立防线。

需要定义小型、封闭的 proof certificate 语言，以及不信任优化 pass/analysis 结论的检查责任：
检查 CFG dominance、SSA/Memory SSA、归纳不变量、effect order、事实推导和变换前提；分析结果只能
作为待验证输入。

### B05：unsafe 契约事实的动态作用域与 inlining 规则缺失

`TrustedContract` 只在 unsafe function entry 成立。effect-aware inlining 如果把 callee entry
事实提升成 caller entry 事实，或让事实越过 call-site 的 argument substitution / dominance
范围，就会错误优化 caller 的其他路径。递归调用也不能继承第一次 entry 的实参事实。

需要明确：每次 unsafe call 建立独立的契约事实实例；inline 后只支配对应 clone 区域；事实按该次
实参替换；不能逃逸为 caller-entry 事实；递归边重新建立并验证自己的事实实例。

### B06：LLVM “post-lowering audit”的时间点不明确

LLVM O1–O3 自己可以合法推导新属性和 flags，它们没有 CK `FactId`。若审计发生在 LLVM 优化后，
“任何没有 KIR 来源的属性均拒绝”会误拒 LLVM 自有优化；若发生在 lowering 前则又审不到实际 IR。

需要把 CK 审计固定在“完成 KIR → LLVM lowering 后、调用 LLVM optimization pipeline 前”；
只审计 CK lowering 生成的 strengthening。后续 LLVM 自行推导的内容属于 LLVM 责任域。

### B07：`unsafe main` 与既有 executable entry 契约冲突

规范要求每个 unsafe function 至少一个 `requires`，而当前合法 `main` 必须无参数且返回 `void`
或 `i32`。允许 `unsafe main` 会产生无调用方可承担、难以有意义表达且与 sanitizer entry 边界
重复的可执行入口契约。

需要在 0.11 明确禁止 `main` 使用 `unsafe`、`contract` 或 `effects`，并归入 `CK2014`。

### B08：sanitizer 必须精确实现数学契约和地址区间

契约整数语义是无界数学整数；`noalias` 使用不回绕的数学 byte range。若 sanitizer 使用目标宽度
整数直接计算 affine expression、`address + byte_len` 或指针关系，极值输入可能先发生溢出或宿主
语言 UB，导致错误通过、崩溃或不稳定错误码。

需要要求 sanitizer 使用精确或显式溢出感知的整数算法；任一地址/长度范围无法表示或发生目标地址
宽度回绕都视为契约违反，并保持唯一的 `CKR0007`/246 结果。验收应覆盖极值与回绕案例。

## 非阻断但必须进入计划的观察项

1. 六个 LLVM fact audit 应在现有六个原生 release runner 上分别执行；当前 native target 是 host
   target，不应把验收误写成单机 cross-target 审计。
2. 普通构建下的 false `requires` 是立即 UB，不应通过“期待某个运行结果”测试；应结构化验证事实
   导入，并只在 sanitizer 模式做负向运行测试。
3. exported unsafe header 的规范化注释需要明确 slice ABI 展平后的参数名映射，否则 foreign caller
   无法机械对应 `x.data`/`x.len`。
4. 0.11 尚未发布；本分支只实现并形成候选，不创建 tag、Release，不合并主分支。版本发布动作需由
   所有验收通过后的独立用户指令授权。

## 本轮判定

在 B01–B08 修订并再次对抗性审查前，不进入计划拆分。

## 第 2 步复诊

复诊逐项对照了当前实现，而不是直接接受第一轮结论：

| 项目 | 仓库证据与反证尝试 | 复诊判定 | 最小修订 |
| --- | --- | --- | --- |
| B01 | `lower_and_optimize` 先优化完整 MIR；C/WAT 在其后裁剪，WASM binary、run 和 executable build 路径并不统一；C/LLVM guard 仍在 backend 根据 mode 生成。无法从现状推出唯一 KIR。 | 阻断成立 | 固定 `semantic MIR → consumer roots/prune → mode-specific KIR → KIR pipeline → backend`；inspection 使用 export+entry roots。 |
| B02 | 当前 `MirInstructionEffect` 只有全局粗分类，没有 parameter mapping；规范的新 summary 又多于 effect clause 可表达集合。把 `effects none` 解释成全效果上限会使带 print/may-fail 的声明无可表达覆盖。 | 阻断成立 | 明确 clause 只约束 externally reachable memory；local/private memory 不计；无法映射访问归 `all`；print/may-fail/unsafe 始终推导。 |
| B03 | 现有 backend 尚未生成这些强化属性，因此不存在必须兼容的旧行为；但原规范确实允许从 pairwise relation 误实现为 parameter-wide promise。第三 pointer root 是直接反例。 | 阻断成立 | pairwise 默认只形成精确 access metadata；完整参数级属性必须额外证明全局 backend 前提。 |
| B04 | 当前只有 MIR structural validator，0.11 尚无 proof checker。若计划只实现 `ProofId` 与重新查询 analysis，mutation test 无法区分同源错误。 | 阻断成立 | 规定封闭 certificate、独立 checker 和 analysis-output-untrusted 边界；允许校验 transfer invariant，不要求重复求最优解。 |
| B05 | 当前 inliner 以 MIR function 为单位 clone，未来若直接携带 entry facts，最自然实现会扩大 scope；规范原文没有 call-instance identity。 | 阻断成立 | 引入 call-instance/scoped fact，实参替换与 dominance 限定；递归边单独实例化。 |
| B06 | 当前 Native 顺序明确是 lowering → LLVM verify → LLVM optimize；因此可在 verify 与 optimize 之间做 CK fact audit。LLVM 后续自己推导属性是合理反例。 | 阻断成立且可局部修复 | 精确规定审计时点和 CK-owned/LLVM-owned 边界。 |
| B07 | `create_checked_program` 只接受 non-export、无参、`void/i32` 的 `main`；unsafe 的“至少一个 requires”不能依赖参数时仍可写常量谓词，但没有有价值的 caller boundary，且外部 entry 不存在。 | 阻断成立 | 0.11 直接禁止 unsafe/contracted `main`，保持入口契约单一。 |
| B08 | 目标整数与地址宽度有限，而规范契约明确使用无界数学整数和不回绕 range；直接生成普通 arithmetic 存在可构造极值反例。 | 阻断成立 | 要求 exact limb 或等价 overflow-safe evaluator；range overflow 判 violation；增加极值 mutation/runtime acceptance。 |

复诊也确认四个观察项不需要扩大为阻断：六 runner 可复用现有 CI matrix；普通 immediate-UB
采用结构化测试即可；header 注释映射是确定性 ABI 文本工作；发布动作不属于本分支授权范围。

**复诊结论：B01–B08 均保留，按上表最小范围同步修订英文与简体中文规范。**
