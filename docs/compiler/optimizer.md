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

Integer constant propagation also runs in guard-free functions. It rewrites
modular arithmetic, integer copies and comparisons, including consumers of
constant block parameters whose every incoming edge agrees. Each transaction
checks closed derivations and the exact replacement values against immutable
pre-rewrite KIR before changing any instruction. Exhausting the deterministic
per-function budget discards that function's pending proposals. This transform
does not remove checked operations or guards and does not fold strict floats.

Boolean constants propagate through copies, negation, equality/inequality, and
all-input joins. Integer comparisons feed the same Boolean worklist, so a proven
Boolean join can drive subsequent branch pruning. The checker binds each Boolean
truth value to the actual definition and verifies every incoming edge; differing
or unknown inputs never authorize a constant replacement.

Proven-safe checked arithmetic also supplies values to downstream propagation,
including checked instructions with a separate overflow result. The checked
producer remains intact; its guard can disappear only through the independent
guard-elimination transaction. Guaranteed division/remainder failures yield an
unknown scalar with an explicit failure state, not a numeric constant or an
analysis error. Nonfailing constant remainders retain their exact signed result.

Constant integer and Boolean block parameters become fresh constant instructions
with the same value identities; all incoming scalar arguments are repaired while
Memory SSA arguments retain their order. The entire batch is checked and fresh
instruction identities are reserved before mutation. Parameters referenced by live
certificates are preserved.

Before guard elimination, propagation and constant-edge pruning reach a CFG fixed
point. Each CFG change validates structure and rebuilds live contract imports;
discarded call instances cannot leave active facts behind. Transient scalar proofs
are consumed before each rewrite and never reused across CFG changes. Empty jump
blocks forward both scalar and memory arguments by substitution; blocks defining
nonlocal SSA uses or contract bindings are retained. This forwarding neither moves
nor removes a reachable effect. DCE also removes unreferenced descriptor regions
whose slice definitions have disappeared, without removing checked failures.

Scalar propagation uses a deterministic SSA-use worklist. A changed range queues
only its consumers, including block parameters that depend on a comparison edge's
other operand. Later path refinements update already-visited joins and their
consumers; unchanged ranges do not trigger another round. Every queued evaluation
consumes the same fixed per-function budget, and exhaustion discards all pending
proofs and rewrites for that function.

Integer ranges also flow from entry contracts and individual comparison edges
through all-input block-parameter joins. The proof checker validates each
premise at its actual definition or incoming edge; branch-local evidence cannot
escape to the predecessor or the other arm, even when both arms share a target.
Check elimination consumes projected, independently checked range certificates
for overflow, nonzero divisors, signed division overflow, and fixed-length slice
indices. Unknown safety retains the guard; malformed evidence fails compilation.
Later scalar folding, GVN, LICM, and DCE preserve instructions referenced by live
certificates. Unrelated dead instructions are not retained as proof dependencies.

The separate full scalar product-domain analysis remains demand-driven by
safety-check consumers; guard-free functions do not build unused range results.

Induction discovery checks every entry and latch, requiring the same initial value
and recurrence on all incoming paths. Transparent values and invariant bounds are
traced through real SSA arguments and copies, never inferred from source variable
names. Mixed steps and intervening assignments remain conservative. A scalar loop
invariant certificate must name the transfer result actually passed on every
backedge, not an unused operation with convenient arithmetic.

The guard checker does not call loop analysis. Its local strict-bound rule checks
the actual integer comparison, all SSA forwarding inputs, and the specific taken
edge: deleting that edge must make the guarded use unreachable. Merely dominating
the use with the edge's target block is insufficient. The pointwise facts
`i < bound` and the integer type prove `i + 1` safe; a u32 index is in bounds only
when that same bound is the indexed slice's length or a dominating contract proves
it no greater. This rule needs no inferred recurrence or constant loop start.
Slice identity follows real SSA inputs, not slot names. All graph walks terminate
with visited sets; ambiguous inputs retain the guard. Actual loop-invariant
certificates still require the separate entry/transfer checks above.

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
