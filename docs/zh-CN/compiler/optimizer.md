# CalcKernel 0.11 Fact-Driven Optimizer

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

## Pipeline

- O0：构造并验证 mode-specific KIR，不运行可选 transform。
- O1：`cfg-canonicalize`、`sccp-range`、proof-carrying `check-elimination`、
  `dead-code-elimination`、`cleanup`。
- O2：增加 `effect-aware-inline`、`memory-ssa-refine`、`gvn`、
  `load-forwarding`、`dead-store-elimination`，然后重跑 range/check cleanup。
- O3：增加 `natural-loop-analysis`、保守 `licm`、induction analysis、post-loop
  range/check elimination、DCE 与 cleanup。

整数常量传播也处理无 guard 的函数，实际改写 modular arithmetic、整数 Copy 和比较，
包括所有输入边均为同一常量的 block parameter 的消费者。每次事务先针对不可变的改写前
KIR 检查闭合推导及精确替换值，全部通过后才修改指令。超出确定性的单函数预算时，丢弃
该函数尚未提交的全部提案。该变换不删除 checked operation 或 guard，也不折叠 strict float。

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
下比较 Clang、精确 0.10、checked/unchecked proof loop 与 optimizer latency。阈值不能成为
弱化语义或使用 contract domain 外输入的理由。
