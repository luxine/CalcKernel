# CalcKernel 0.11 Fact-Driven Optimizer

[简体中文](../zh-CN/compiler/optimizer.md)

`--opt-level 0|1|2|3` and `-O0`–`-O3` select one target-neutral KIR pipeline
shared by C, WebAssembly, and Native LLVM. Semantic MIR is validated and kept as
the stable source-order boundary; it is not a second optimized product path.

## Evidence model

KIR contains scalar SSA, explicit control flow and guards, region Memory SSA,
ordered failure/print effects, effect summaries, facts, and proof certificates.
Facts are either `Proven` by compiler analysis or `TrustedContract` at a
dominating unsafe-function instance. Unknown, over-budget, or unclassifiable
analysis yields conservative state, never an optimistic assumption.

Every guard elimination carries a `ProofId`. A small verifier checks the closed
certificate against the current CFG, SSA definitions, dominance, Memory SSA,
effect order, and contract-instance scope without asking the optimizing analysis
to approve its own result. CFG, inlining, or loop changes explicitly invalidate
or remap affected evidence. Invalid or stale evidence is a compiler error.

Verification-cache reuse compares the complete KIR, proofs, guard-rewrite records,
and contract facts with the last verified state in both debug and release builds.
A pass's `changed = false` declaration alone never authorizes reuse.

## Pipelines

- O0 constructs and verifies mode-specific KIR and runs no optional transform.
- O1 runs `cfg-canonicalize`, `sccp-range`, proof-carrying
  `check-elimination`, `dead-code-elimination`, and `cleanup`.
- O2 adds `effect-aware-inline`, `memory-ssa-refine`, `gvn`,
  `load-forwarding`, `dead-store-elimination`, then reruns range/check cleanup.
- O3 adds `natural-loop-analysis`, conservative `licm`, induction analysis,
  post-loop range/check elimination, DCE, and cleanup.

Scalar range analysis is demand-driven per function. A guard-free function has
no safety-check consumer, so the named pass and its verifier record remain in
the pipeline without constructing an unused product-domain result.

KIR inspection uses `emit-kir`, `--print-facts`,
`--print-effect-summaries`, and `--explain-optimization`. Output is
deterministic and distinguishes trusted from proven evidence.

## Required preservation

Unchecked integer operations retain modular semantics. Checked operations keep
their possible-failure effect until proven safe. Strict `f64` does not use
fast-math and preserves NaN, infinity, signed zero, and operand order.
`slice<T>` guards under `--bounds checked` may disappear only when index/range
facts prove the selected check.
Raw-pointer validity and the truth of `slice(data, len)` remain caller duties.

The shared alias service combines region origin, symbolic sub-slice intervals,
access width, `noalias`, and alignment. Memory optimizations consume the same
query and Memory SSA versions. Interprocedural summaries are solved over
call-graph SCCs and retain reads/writes, runtime print, possible checked failure,
unsafe calls, and conservative `readwrite all` state.

Possible checked failures and runtime prints are ordered effects. No pass may
move another failure or observable operation across them without a proof that
the move is unobservable. Contract facts never weaken source semantics: a false
unsafe precondition is already undefined at entry, while safe functions gain no
new undefined behavior from failed analysis.

## Backend facts and performance

Only verified pairwise no-alias, alignment, range, and effect facts reach C or
LLVM. C emits portable hints only when their complete preconditions hold. Native
performs a pre-LLVM fact audit and rejects injected or stale metadata.

Performance gates compare identical algorithms, safety modes, data, hardware,
CPU policy, and strict semantics. Native must meet the pinned Clang thresholds,
0.11 may regress no more than the recorded limits from exact 0.10, canonical
checked proof loops must approach unchecked throughput, and KIR optimization
latency is bounded against the 0.10 MIR optimizer. Thresholds never authorize
weaker semantics or invalid contract-domain inputs.
