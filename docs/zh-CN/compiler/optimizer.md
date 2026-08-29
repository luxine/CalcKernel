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

## Pipeline

- O0：构造并验证 mode-specific KIR，不运行可选 transform。
- O1：`cfg-canonicalize`、`sccp-range`、proof-carrying `check-elimination`、
  `dead-code-elimination`、`cleanup`。
- O2：增加 `effect-aware-inline`、`memory-ssa-refine`、`gvn`、
  `load-forwarding`、`dead-store-elimination`，然后重跑 range/check cleanup。
- O3：增加 `natural-loop-analysis`、保守 `licm`、induction analysis、post-loop
  range/check elimination、DCE 与 cleanup。

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
