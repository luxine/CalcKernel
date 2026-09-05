# CalcKernel 0.13 Fact-Driven Optimizer

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
For a changed module, exact unchanged functions may reuse their prior structural
verdict while changed functions are fully checked and module-global identity plus
all fact/proof/rewrite validation still runs. Immutable profile validation and
CFG-only dominance results are memoized; dominance cache hits debit the same
deterministic analysis budget. Candidate-free discovery bypasses only speculative
state allocation, never a candidate, checker, certificate digest, or final verifier.
No-op frontiers reuse discovery-only loop descriptors only across passes that
preserve induction structure and only when the cached function identity matches.

## Pipelines

- O0 constructs and verifies mode-specific KIR and runs no optional transform.
- O1 runs `cfg-canonicalize`, `sccp-range`, proof-carrying
  `check-elimination`, `dead-code-elimination`, and `cleanup`.
- O2 adds `effect-aware-inline`, `memory-ssa-refine`, `gvn`,
  `load-forwarding`, `dead-store-elimination`, then reruns range/check cleanup.
- O3 first normalizes loops and runs the bounded specialization frontier, then
  adds `natural-loop-analysis`, legality/dependence analysis, conservative
  `licm`, `induction-simplify`, post-loop range/check elimination, the mutually
  exclusive Loop SIMD/unroll/loop-SLP frontier, residual straight-line SLP,
  DCE, and cleanup.

## Workload-profile authority

CK workload profile data is immutable non-proof input. It can rank candidates
and estimate work, but cannot establish range, alias, alignment, effect, bounds,
dominance, or checked-failure safety. A profile mapping survives a CFG rewrite only
through a closed record rechecked without calling the proposer; unknown,
saturated, inconsistent, overflowed, or low-confidence observations retain the
ordinary baseline.

O2 runs the full ordinary machine pipeline first. Profile-on and profile-off are
byte-identical immediately before `CkLateProfileLayout`; that late pass may only
change block/trace ordering and required target repairs from the closed allowlist.
It supplies no LLVM profile metadata and cannot alter non-terminator instructions.

O3 starts each inlining, value/length specialization, unroll, SLP, and Loop SIMD
proposal from the same immutable pre-state. An independent checker recomputes
legality, proof dependencies, profile benefit, static cost, growth, profile
mapping, and shared budget. A transaction publishes the candidate module,
proof/fact state, mapping, and audit ledger together or rolls them all back;
rejected proposals and exhausted searches do not refund budget.

Multiversion planning also starts baseline and every enhanced variant from the
same pre-state. Eligible exported roots need the closed minimum profile benefit;
each target variant reruns the normal verifier, fact audit, target-feature audit,
and object audit. Cross-variant LTO is forbidden, so an enhanced assumption
cannot strengthen baseline or a sibling variant. The baseline-safe dispatcher
selects a verified compatible variant without changing public semantics.

Every KIR module carries a canonical `KirTargetProfile`. Inspection, portable
C, WebAssembly, Native library, and Native executable profiles identify their
consumer, target, CPU policy, operation availability and exact fixed-width
costs. Missing, zero, stale, or target-mismatched answers reject optimization;
the optimizer never substitutes host folklore. The profile digest, cost/proof
schema identities, and optimizer budgets are object-affecting cache inputs.
C and WebAssembly profiles disable Vector KIR in 0.13.

Specialization, unroll, SLP, and Loop SIMD use one verified transactional state:
the complete candidate module, proof/fact state, and audit-budget delta are
prepared without mutating the accepted pre-state. A separate checker validates
the exact rewrite, semantics, proof roots, target legality, cost, growth, and
budget charge. Acceptance atomically swaps module and audit state; ordinary
rejection or exhaustion leaves both byte-for-byte unchanged. Candidate keys,
tie-breaking, fallback reasons, and `--explain-optimization` output are stable.

Specialization is internal and bounded by callee/clone/module limits. It only
uses verified constant arguments and separately scoped trusted-contract facts;
recursive SCCs, indirect calls, checked or sanitizer modes, and observable
effect changes are rejected. Clone identity is deterministic and never exported.

Loop SIMD accepts canonical single-latch loops whose accesses, dependence graph,
strict operation semantics, and target profile all close. It supports direct
load/store maps, strict `f64` arithmetic including unary negate and divide,
supported integer-to-`f64` casts, pure compare/select diamonds, and unchecked
modular integer add/multiply reductions. It emits a fixed-width vector body plus
an ordered scalar epilogue. Unknown aliasing may produce one total overflow-safe
non-overlap predicate guarding a byte-for-byte scalar fallback; more complex
predicates remain scalar. Checked or sanitizer modes, floating/checked reductions,
scans, gather/scatter, vector calls, masked memory, shuffles, and unsupported
alignment/operations remain scalar.

Unroll considers factors 2 and 4, preserving exact trip partition and scalar
remainder semantics. SLP packs only isomorphic, independent, adjacent scalar
operations in source order; it cannot invent shuffle or masked-memory support.
Loop SIMD, loop SLP, and unroll are priced over the same immutable loop scope and
only one winner commits. A vector candidate must beat the scalar cost by at least
20% at its conservative trip threshold; exact shorter trips stay scalar. The
aggregate O3 growth ceiling and proposer/checker work budgets apply across all
0.13 speculative transforms, including rejected alternatives and clones.

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

LICM resolves loop-invariant operands through all phi/Copy inputs. Each transient
source-identity claim is independently checked before rewriting an operand;
mixed incoming values are not invariant. This permits actual invariant integer
expressions, not only constants, to move. Moved instructions keep their ValueIds
and dependency order. Live proof producers, memory operations, calls, print,
checked arithmetic and strict floating arithmetic stay in place. Integer division
and remainder are never speculated: an unchecked operation can still trap on a
path that originally executed zero iterations. LICM search has a fixed per-function
KIR budget; exhaustion restores that function's pre-pass state and reports the
conservative reason. No partial operand rewrite or movement survives that fallback.

Induction discovery checks every entry and latch, requiring the same initial value
and recurrence on all incoming paths. Transparent values and invariant bounds are
traced through real SSA arguments and copies, never inferred from source variable
names. Mixed steps and intervening assignments remain conservative. A scalar loop
invariant certificate must name the transfer result actually passed on every
backedge, not an unused operation with convenient arithmetic.
Strict same-type bounds mark ascending `+1` and descending `-1` recurrences
wrap-safe on their taken edge; non-strict bounds and larger steps do not inherit
that claim.

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

Induction simplification coalesces equal integer loop-carried values. A closed
equality certificate lists the simultaneous SSA equalities and exact producers;
the independent checker validates all incoming phi edges, copies, constants and
matching add/sub transfers with the same overflow semantics. Different initial
values or an unmatched latch prevent the rewrite. The pass replaces redundant
block parameters with same-ValueId copies and removes their incoming scalar
arguments; Memory SSA, calls, stores and guards keep their order and identity.
Unused modular recurrences can then disappear, while checked failures still need
their own guard proof. Live phi certificates are protected. Certificates, rewrite
bindings and fresh instruction IDs are checked before any mutation. Candidate
search has a fixed per-function KIR-size budget; exhaustion discards that function's
pending rewrites and increments `induction_budget_fallbacks` deterministically.

Natural-loop analysis uses the same fixed KIR-size budget for its dominator
matrix/iteration and subsequent loop-graph and SSA-forwarding work. Exhaustion
discards all partial loop/induction results for that function. The structural
verifier still computes complete dominance; an analysis fallback never skips
verification. Dominator iteration follows block IDs, independent of storage
order. Removing dominance backedges must leave an acyclic graph; residual
cyclic components identify irreducible control flow even inside a natural outer
loop. Such functions conservatively skip LICM and induction simplification.
Self-latches do not pull preheaders into the loop body. `--explain-optimization`
reports per-function/pass `fixed-kir-budget-exhausted` or
`irreducible-control-flow` reasons, including induction-search exhaustion.

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
CPU policy, training/evaluation split, and strict semantics. Schema 8 compares
0.13 ordinary/PGO/multiversion/combined channels with pinned Clang/Rust PGO and
hand-written SIMD oracles, and replays exact 0.12 commit
`c70681e70f050a8782373af13f58d7803cae1fbf`. Correctness, optimization time,
generation overhead, artifact size, compiler archive size, and cache behavior
have separate gates. PGO and bounded multiversioning ship in 0.13. Auto-Tuning
remains 0.14; indirect calls, scalable KIR, and adaptive JIT PGO remain future.
Thresholds never authorize weaker semantics or invalid contract-domain inputs.
