# CalcKernel 0.13 Fact-Driven Optimizer

[English](../../compiler/optimizer.md)

`--opt-level 0|1|2|3` 与 `-O0`–`-O3` 选择 C、WebAssembly、Native LLVM 共用的单一
target-neutral KIR pipeline。Semantic MIR 仍是稳定 source-order boundary，但不是第二套
optimized product path。

## Evidence model

KIR 包含 scalar SSA、显式 control/guard、region Memory SSA、ordered failure/print effect、
effect summary、fact 与 proof certificate。Fact 来自 compiler `Proven` analysis，或来自支配
unsafe function instance 的 `TrustedContract`。未知、超预算或无法分类只会得到保守状态。

每次 guard elimination 都携带 `ProofId`。小型 verifier 独立检查当前 CFG、SSA、dominance、
Memory SSA、effect order 与 contract-instance scope，不让优化 analysis 自己批准结论。
CFG/inlining/loop 改动必须显式 invalidate 或 remap evidence；stale evidence 是 compiler error。

Debug 与 release 构建复用 verification cache 前，都将完整 KIR、proof、guard rewrite
record 与 contract fact 和上一已验证状态逐项比较。仅有 pass 的 `changed = false` 声明
不能授权复用验证结果。
对于 changed module，精确未变化的 function 可以复用既有 structural verdict，但 changed
function 仍完整检查，并继续执行 module-global identity 与全部 fact/proof/rewrite validation。
Immutable profile validation 与仅由 CFG 决定的 dominance result 可以缓存；dominance cache hit
仍扣除相同的确定性 analysis budget。Candidate-free discovery 只省略 speculative state
allocation，绝不省略 candidate、checker、certificate digest 或 final verifier。No-op frontier
只有跨越保持 induction structure 的 pass 且缓存 function identity 精确匹配时，才复用
discovery-only loop descriptor。

## Pipeline

- O0：构造并验证 mode-specific KIR，不运行可选 transform。
- O1：`cfg-canonicalize`、`sccp-range`、proof-carrying `check-elimination`、
  `dead-code-elimination`、`cleanup`。
- O2：增加 `effect-aware-inline`、`memory-ssa-refine`、`gvn`、
  `load-forwarding`、`dead-store-elimination`，然后重跑 range/check cleanup。
- O3：先规范化循环并运行有界 specialization frontier，再增加
  `natural-loop-analysis`、legality/dependence analysis、保守 `licm`、
  `induction-simplify`、post-loop range/check elimination、互斥的 Loop SIMD/unroll/
  loop-SLP frontier、residual straight-line SLP、DCE 与 cleanup。

## Workload profile 权限

CK workload profile 是 immutable non-proof input，只能排序 candidate 与估算 work，不能建立
range、alias、alignment、effect、bounds、dominance 或 checked-failure safety。CFG rewrite 后
的 profile mapping 只有在闭合 record 被不调用 proposer 的 checker 重新核验时才保留；unknown、
saturated、inconsistent、overflowed 或 low-confidence observation 均回退 ordinary baseline。

O2 先运行完整 ordinary machine pipeline。Profile-on/off 在 `CkLateProfileLayout` 前逐字节
一致；该 late pass 只能改变 block/trace order，以及执行闭合 allowlist 中的 required target
repair。它不提供 LLVM profile metadata，也不能改变 non-terminator instruction。

O3 让 inline、value/length specialization、unroll、SLP、Loop SIMD 的每项 proposal 从 same
immutable pre-state 开始。独立 checker 重算 legality、proof dependency、profile benefit、
static cost、growth、profile mapping 与 shared budget。一个 transaction 同时发布 candidate
module、proof/fact state、mapping 与 audit ledger，或全部回滚；被拒 proposal 与耗尽搜索不退款。

Multiversion planning 同样让 baseline 与全部 enhanced variant 从 same pre-state 开始。
Eligible exported root 必须达到闭合的 profile benefit 下限；每个 target variant 都重跑 normal
verifier、fact audit、target-feature audit 与 object audit。Cross-variant LTO 禁止，因此 enhanced
assumption 不能强化 baseline 或 sibling variant。baseline-safe dispatcher 只选择已验证的兼容
variant，不改变 public semantics。

每个 KIR module 都携带规范化 `KirTargetProfile`。Inspection、portable C、WebAssembly、
Native library 与 Native executable profile 明确 consumer、target、CPU policy、operation
availability 和 fixed-width 精确 cost。缺失、零值、过期或 target 不匹配的答案会拒绝优化；
优化器不以 host 常识代替 profile。Profile digest、cost/proof schema identity 与 optimizer
budget 都进入 object/cache identity。0.13 的 C/WebAssembly profile 禁用 Vector KIR。

Specialization、unroll、SLP 与 Loop SIMD 共用 verified transactional state：完整 candidate
module、proof/fact state 和 audit-budget delta 在不修改 accepted pre-state 的情况下生成。
独立 checker 核验精确改写、语义、proof root、target legality、cost、growth 与 budget charge。
接受时原子交换 module/audit state；普通拒绝或预算耗尽时二者逐字节不变。Candidate key、
tie-break、fallback reason 与 `--explain-optimization` 输出均稳定。

Specialization 是 internal 且受 callee/clone/module 上限约束。它只使用已验证 constant argument
与单独 scoped trusted-contract fact；recursive SCC、indirect call、checked/sanitizer mode 以及
observable effect 变化都拒绝。Clone identity 确定且永不 export。

Loop SIMD 只接受 access、dependence graph、strict operation semantics 与 target profile 全部
闭合的 canonical single-latch loop。支持直接 load/store map、包含 unary negate/divide 的
strict `f64` 算术、受支持的 integer-to-`f64` cast、pure compare/select diamond，以及 unchecked
modular integer add/multiply reduction。结果为 fixed-width vector body 加保持顺序的 scalar
epilogue。Alias 未知时可生成一个 total、overflow-safe non-overlap predicate，保护逐字节一致
的 scalar fallback；更复杂 predicate 保持 scalar。Checked/sanitizer mode、floating/checked
reduction、scan、gather/scatter、vector call、masked memory、shuffle 及不支持的
alignment/operation 都保持 scalar。

Unroll 只考虑 factor 2/4，并保持精确 trip partition 与 scalar remainder 语义。SLP 只按 source
order 打包 isomorphic、independent、adjacent scalar operation，不能发明 shuffle 或 masked
memory。Loop SIMD、loop SLP 与 unroll 在同一不可变 loop scope 上计价，只有一个 winner
提交。Vector candidate 在保守 trip threshold 必须比 scalar cost 至少低 20%；已知更短 trip
保持 scalar。O3 aggregate growth ceiling 与 proposer/checker work budget 覆盖全部 0.13
speculative transform，包括被拒绝的 alternative 与 clone。

整数常量传播也处理无 guard 的函数，实际改写 modular arithmetic、整数 Copy 和比较，
包括所有输入边均为同一常量的 block parameter 的消费者。每次事务先针对不可变的改写前
KIR 检查闭合推导及精确替换值，全部通过后才修改指令。超出确定性的单函数预算时，丢弃
该函数尚未提交的全部提案。该变换不删除 checked operation 或 guard，也不折叠 strict float。

布尔常量经过 Copy、取反、相等/不等比较和所有输入边的 join 传播。整数比较也向同一
布尔工作队列提供结果，因此已经证明的布尔 join 可以驱动后续分支剪枝。检查器将每项
真假结论绑定到真实定义并逐条验证输入边；输入不同或未知时不能替换为常量。

已证明不会失败的 checked 算术也向下游传播值，包括另有 overflow 结果的 checked
指令。原 checked producer 保持不变；其 guard 只能由独立的检查消除事务删除。必然
失败的除法/取模得到带明确 failure 状态的 unknown 标量，不伪造数值常量，也不升级为
分析错误。不会失败的常量取模保留精确的有符号结果。

常量整数与布尔 block parameter 会改写为具有新 instruction identity、保持原 value identity 的
常量指令，并修复全部输入边的标量参数；Memory SSA 参数顺序不变。整批证书检查与新
指令 ID 预留都发生在写入之前。仍被有效证书引用的 block parameter 保留原定义。

在检查消除之前，传播与常量分支剪枝迭代到 CFG 不动点。每次 CFG 改动都验证结构并重建
存活的契约实例，已删除调用不能遗留活动事实。临时标量证书在每次改写前消费，不跨
CFG 改动复用。空跳转块通过参数替换同时转发标量和 memory 参数；定义了跨块 SSA 使用
或契约绑定的块保守保留。转发不移动或删除任何可达效果。DCE 还清除已失去 slice 定义、
未被引用的描述符 region，但不会因此删除 checked 失败。

标量传播使用确定性的 SSA-use 工作队列。范围变化只唤醒消费者，也包括依赖比较边另一
操作数的 block parameter。后到的路径范围会更新已经访问过的 join 及其消费者；范围
不变则不再触发下一轮。每次队列求值都扣除同一固定的单函数预算，耗尽时丢弃该函数
全部尚未提交的 proof 与 rewrite。

整数范围也从入口契约和比较的各条分支边传播，并在 block parameter 处合并所有输入。
证明检查器在真实定义位置或输入边核验每项前提；分支局部证据不能逃逸到前驱或另一条
分支，即使两条分支指向同一目标块。检查消除使用仅保留所需依赖且经过独立核验的范围
证书，证明溢出、非零除数、有符号除法溢出和定长 slice 索引安全。安全性未知时保留
guard，证据格式或推导失效则编译失败。后续常量折叠、GVN、LICM 和 DCE 保留仍被有效
证书引用的指令；无关死指令不会因证明依赖而被保留。

另行运行的完整 scalar product-domain analysis 仍按 safety-check consumer 的需求执行；
无 guard 的函数不会构造无人消费的 range result。

LICM 沿所有 phi/Copy 输入解析循环不变操作数。每个临时来源等值声明在改写操作数前
由独立检查器核验；输入来源不一致时不作为不变量。这使真正的不变整数表达式（不只
常量）能够移出循环。移动保留 ValueId 和依赖顺序。存活证明的 producer、内存操作、
调用、print、checked 算术及 strict 浮点算术保持原位。整数除法和取模不提前执行：
unchecked 运算也可能在原本零次迭代的路径上触发 trap。LICM 搜索采用固定单函数
KIR 预算；耗尽时恢复该函数 pass 前状态并报告保守原因，不保留部分操作数改写或移动。

归纳变量识别检查每条入口和回边，要求所有输入路径具有相同初值和递推关系。透明值与
不变边界沿真实 SSA 参数和 Copy 追踪，不能根据源变量名猜测。步长不同或中间重新赋值
时保持保守。标量循环不变量证书必须指明每条回边实际传递的 transfer 结果，不能借用
一个算术上合适但未参与回传的 operation。
严格同类型边界可标记升序 `+1`、降序 `-1` 递推在 taken edge 上不会回绕；非严格
边界和更大的步长不继承该声明。

guard 检查器不调用循环分析。局部 strict-bound 规则核验真实整数比较、全部 SSA 转发
输入及具体 taken edge：删除该边后，guard 所在位置必须不可达。仅有该边的目标块
支配使用位置并不足够。`i < bound` 与整数类型可逐点证明 `i + 1` 安全；u32 索引只有
在同一 bound 是所访问 slice 的长度，或支配契约证明 bound 不大于该长度时才证明
不越界。此规则不需要推断递推关系或常量循环初值。slice 身份沿真实 SSA 输入确认，
不使用 slot 名。图遍历通过 visited set 保证终止，输入不明确时保留 guard。真正的
循环不变量证书仍必须通过上述独立入口/transfer 检查。

归纳简化合并等值的整数循环携带值。闭合等值证书列出同时成立的 SSA 等式与准确的
producer；独立检查器核验所有 phi 输入边、Copy、常量，以及溢出语义相同的对应
add/sub transfer。初值不同或任一回边不匹配时不改写。pass 将冗余 block parameter
替换成保留原 ValueId 的 Copy，并移除相应输入标量参数；Memory SSA、调用、写入和
guard 的顺序及身份不变。未使用的 modular 递推随后可删除，checked 失败仍须单独
的 guard 证明。存活 phi 证书的依赖受到保护。证书、改写绑定及新指令 ID 均在修改前
核验。候选搜索使用由 KIR 大小决定的固定单函数预算；耗尽时丢弃该函数未提交的
改写，并确定性增加 `induction_budget_fallbacks` 计数。

自然循环分析使用同一固定 KIR 大小预算，覆盖支配关系矩阵/迭代及后续循环图和 SSA
转发遍历。耗尽时丢弃该函数的所有部分循环/归纳结果。结构 verifier 仍计算完整
支配关系，分析回退不会跳过验证。支配迭代按 block ID 进行，不依赖存储顺序。
移除 dominance backedge 后的图必须无环；剩余的循环分量用于识别不可约控制流，
也覆盖自然外循环内的多入口循环。此类函数保守跳过 LICM 和归纳简化。自回边不会
把 preheader 纳入循环体。`--explain-optimization` 按函数/pass 输出
`fixed-kir-budget-exhausted` 或 `irreducible-control-flow` 原因，也包括归纳搜索耗尽。

`emit-kir`、`--print-facts`、`--print-effect-summaries`、
`--explain-optimization` 提供 deterministic KIR inspection，并区分 trusted/proven evidence。

Unchecked integer 保持 modular semantics；checked operation 在证明安全前保留 may-fail
effect；strict `f64` 不启用 fast-math。`slice<T>` guard 只有在 `--bounds checked` 下的
index/range 被证明安全时才能删除；
raw pointer validity 与 `slice(data, len)` 的真实性仍由 caller 负责。

统一 alias service 结合 region origin、symbolic sub-slice interval、access width、`noalias`
与 alignment。Memory optimization 共用该 query 和 Memory SSA version。Call-graph SCC 的
effect summary 保留 read/write、runtime print、possible checked failure、unsafe call 与
conservative `readwrite all`。

Possible checked failure 和 runtime print 是 ordered effect，不能无证明跨越重排。只有经过
验证的 no-alias/alignment/range/effect fact 才能进入 C/LLVM；Native pre-LLVM fact audit 会
拒绝 injected 或 stale metadata。

Performance gate 在相同算法、safety mode、data、hardware、CPU policy 和 strict semantics
下使用 schema 8 比较 0.13 ordinary/PGO/multiversion/combined、固定 Clang/Rust PGO、
hand-written SIMD oracle，并 replay exact 0.12 commit
`1009bae18d1a1ebd37ee9ee095cab9a965e69df8`。Correctness、optimization time、generation
overhead、artifact/compiler archive size 与 cache 各有独立 gate。PGO 与受限 multiversioning
在 0.13 交付；Auto-Tuning remains 0.14，indirect calls、scalable KIR 与 adaptive JIT PGO
仍属未来。阈值不能成为弱化语义或使用 contract domain 外输入的理由。
