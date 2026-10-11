# Loop, Memory and Vectorization

[简体中文](../zh-CN/compiler/generalized-loop-vectorization.md)

Loop SIMD is a property-based transformation over verified KIR. Source function,
variable and fixture names are not legality inputs. Discovery, legality,
profitability, materialization and independent verification have separate roles.

## Loop model

`CanonicalLoopDescriptor` records loop identity, header, preheader, latch, exits,
body blocks, nesting, induction and trip relation. Header value parameters and
Memory SSA parameters describe carried state through entry and backedge operands.
Canonical analysis remains distinct from transformation legality: an analyzable
loop can contain a recurrence, an ordered effect, an unknown memory relation or
an operation unavailable on the target.

Irreducible control, unsupported exits, incomplete carried-state reconstruction
and exhausted analysis budgets retain scalar execution. Analysis records are
valid only for the KIR snapshot from which they were reconstructed.

## Memory access model

The shared affine access analysis describes source and root memory regions, base,
read/write kind, element type and size, alignment, slice range and index form.
An affine form uses deterministic SSA coefficient terms and a constant. Outer
induction values and invariant expressions can be leaves relative to an inner
loop. Broadcast, unit stride and constant stride describe properties; recognizing
a property does not imply that a vector lowering exists for it.

The affine record can represent more coefficient terms than the current generic
WASM f64 materializer accepts. That path still requires each access to project
exactly to the loop induction plus one invariant offset, or to one invariant
broadcast. It also requires the supported operation and loop-carried-state
shape. Full affine analysis is therefore not arbitrary affine lowering or a
polyhedral optimizer.

Mathematical affine equality and machine index arithmetic are distinct. A modular
index may wrap. Such a summary cannot prove disjointness, distance or injectivity
by itself. The checker must establish the required nonwrapping span or preserve
the exact machine index expression. Compatibility projections onto one induction
and one invariant must be exact before consumers use them.

## Dependence and effects

Memory dependence uses a shared relation over normalized access records and
verified alias information. It distinguishes read-after-write, write-after-read
and write-after-write, same-iteration relationships, carried relationships and
unknown relationships. Read/read pairs do not reorder an observable write.
Same-location reads and writes can be lane-local only when the access is
injective across iterations. A lone invariant store is not independent merely
because no second instruction exists.

Unknown alias, range or dependence remains conservative. Profile observations
can rank candidates, estimate trip counts and influence cost; they cannot prove
memory safety, alignment, effect freedom or absence of dependence.

## Candidates, plans and cost

`VectorizationCandidate` combines a canonical loop, supported operation mappings,
memory access properties, VF, UF, trip threshold and predicates. Target capability
filters lane types and operations. Enumeration is bounded, with stable ordering
and tie-breaking; cost and growth budgets choose among legal candidates.

`VectorizationPlan` records the immutable KIR/profile/evidence identity, scalar
operation to vector lane mapping, memory and broadcast groups, runtime predicates,
epilogue, cost, growth and proof roots. Existing modular integer reductions and
pure comparison/selection use the same plan family. Strict floating reductions
remain unavailable because reassociation would change source rounding.

Trip evidence follows a fixed precedence: exact canonical trip, proven-bounded
trip from a complete, non-exhausted scalar analysis, profile-estimated trip,
then unknown.
Profile estimates guide cost only; they do not establish a bound or any legality
fact. Exact candidates are priced at their actual trip count, complete vector
groups and scalar remainder, subject to the target's minimum vector-group floor.
They use the same closed 20-percent cost requirement as runtime candidates, so
some exact trips below a runtime candidate's conservative threshold can still
qualify, while others remain scalar.

For non-exact candidates, a runtime threshold is admitted only when every tail
at its first eligible vector-group count satisfies the cost requirement. With
proven-bounded or profile-estimated trips, the shared scenario set covers every
point when the interval is shorter than the widest candidate, or its first and
last residue windows of at most 64 iterations each, for at most 128 scenarios.
For unknown trips, let `H` be the smallest minimum threshold among legal
candidates and `W` the widest candidate width, capped at 64. The shared
scenarios are `max(H - 1, 0)` through `H + W - 1`, so there are at most 65.
The explanation uses `H` as its representative; a candidate whose own threshold
is above a scenario is scored as scalar fallback with zero vector groups and
zero scalar-tail work. All Loop SIMD and loop-SLP alternatives in the scope use
the same finite scenario set. Their aggregated cost is a bounded ranking
heuristic, not a probability estimate or a worst-case normalized ratio.

Each scenario accounts for vector groups, actual scalar tail, predicates and
the scalar path when the trip is below a candidate's threshold. For the
persistent modular-add reduction path, the preheader `Compare`, `Branch` and
`Splat` work is charged on both paths; the vector-only `Fold`, scalar-seed
`Add` and finalizer `Branch` are charged only after vector execution. In the
current WASM profile these parts cost 3 and 10 units respectively. More
generally, the scalar fallback retains only work performed by its actual
preheader. Cost records use checked arithmetic and the closed `u32`
representation; out-of-range trips or fields reject the candidate rather than
saturating into a favorable result.

Structural growth ceilings remain hard validation limits. Ranking separately
penalizes added KIR units and a deterministic mapped-operation expansion proxy,
including memory groups such as stores. For Loop SIMD the proxy counts mapped
vector operations and memory/broadcast groups; SLP counts its vector, setup and
extract instructions. It is an expansion count, not peak register pressure.
The penalty is `(ceil(added KIR units / 4) + ceil(expansion items / 4)) ×
target Branch cost`; when the profile has no Branch cost, the closed one-unit
fallback is used. It is added to every finite scenario for ranking only. Loop
SIMD and SLP use the same scenarios, scalar denominator, Branch unit and
comparator. Ratios use `u128` cross-products. The final stable order is the
existing x86 widening-cast frontend penalty, aggregate score ratio, runtime-only
ratio, fewer added KIR units, smaller expansion proxy, smaller UF, smaller VF,
then candidate key.
Scenario aggregate totals use saturating `u64` static-cost units and can cap at
`u64::MAX` for pathological inputs. `u128` protects only the cross-products
used to compare the stored ratios; it does not restore capped aggregate totals.
This score is therefore a deterministic, capped static-cost ranking signal,
not an exact percentage of uncapped total cost or a measured runtime percentage.
`insufficient-benefit` identifies the chunk cost floor; nonwinners receive
`code-growth` or `tail-cost` only when that factor explains the loss, and
otherwise use `higher-cost-loop-alternative`. Candidate pricing failures also
use stable `tail-cost` or `target-cost` reasons.

For the supported WASM affine domain, contiguous loads/stores and invariant
readonly broadcasts are combined with supported strict lane operations. Their
legality depends on complete addresses, per-write independence and operation
closure. An invariant load may be shared across interleaved chunks only when
its value is stable and independent of every write. Floating operations remain
separate, in source order within each lane; no FMA or fast math is introduced.

## Runtime predicates and epilogue

Runtime versioning checks the complete fast-path precondition before entering
the optimized loop. Address/range arithmetic is total and checked for overflow;
predicate failure enters the original scalar path. Trip threshold, slice-span,
alias, alignment and divisibility predicates retain a closed bounded schema.
Alias interval disjointness does not prove slice or physical memory coverage.
Supported unknown-alias f64 loops combine the required alias and complete span
guards within the same predicate budget; larger bundles retain scalar execution.

The vector body covers complete chunks. An ordered scalar epilogue covers the
remaining iterations, including small and nondivisible trip counts. The
independent checker verifies partitioning, exit state and exact fallback identity.

## Transaction and independent checker

The proposer builds a trial without mutating the accepted KIR. The checker
reconstructs source loop, operations, addresses, alias roots and guard premises
from the original KIR. It does not trust candidate `safe`, `noalias`, cost or
dependence assertions. Pure dependence arithmetic can be shared because its
inputs are reconstructed separately on both sides.

The checker validates materialized operations, lane mapping, memory coverage,
predicates, target legality, scalar remainder, untouched functions, cost and
growth. The trial preserves the original contract facts, eliminated guards,
evidence generation and existing proof records; its appended proofs must match
the plan's complete proof roots. Commit atomically updates KIR and evidence only
after these checks.
Rejection or exhaustion preserves the original scalar behavior.

## Exposure and backend boundary

Invariant branch exposure, pure decision-tree recognition and boundary peeling
can reveal candidate loop structure. Their CFG and speculation proofs remain
distinct from SIMD operation and memory proofs. A boundary producer is allowed
to be specialized; a producer's suggested footprint is not proof of its safety.

The affine checker reconstructs modular source indices and independently proves
that each access is contained in the guarded span. This proof uses affine
coefficients, trip bounds and total range predicates, rather than a prescribed
number or arrangement of loads. Count multiplication must be exact in the
guarded domain before it can discharge a nonwrapping premise.

Pure tree exposure certifies mutually exclusive leaves and their continuation
before shared memory/dependence checks interpret them as one logical write.
The tree's control topology and speculative lane expressions still have a
separate checker because scalar KIR has no general select operation. Invariant
branch exposure identifies roles from operations and data flow, then presents an
ordinary loop to the vectorizer.

The optimizer chooses the transformation. WASM and LLVM backends lower verified
vector operations and enforce instruction, lane, feature and encoding legality.
They do not infer high-level loop safety from workload identities. C and WASM
`baseline` retain their scalar vector-lowering policy.

The generic stencil legality and span checker does not make footprint discovery
generic: the current stencil source exposure still recognizes a bounded 3x3
interior after boundary peeling. Likewise, normalization and decision-tree
support begin with their own bounded source/CFG exposure; the tree checker is
not a general scalar-select or if-conversion facility.

## Identity, budgets and limitations

The published 0.15.2 compiler used vector cost/proof/budget identities 1/1/1;
the current candidate uses 3/2/3, including the proof-identity change from the
published release. The cost identity covers candidate admission/ranking, the
proof identity covers legality evidence, and the budget identity covers bounded
analysis work. The budget includes descriptor-root reconstruction, which uses
an indexed worklist and charges index construction and each visited edge against
`16 * kir_function_units + 64`, capped at 8192 function units. Native cache keys
include the cost, proof and complete deterministic budget identities, so older
optimization evidence cannot be reused under changed rules. KIR v3, public CLI,
source syntax and Native/WASM ABI remain unchanged.

Affine analysis bounds work by KIR size, caps coefficient terms, and stops
dependence enumeration on exhaustion. Optional dead-scalar recognition is bounded
to 128 body instructions, 256 values and 512 work steps; exhaustion retains the
source computations. Candidate width/interleave search remains
closed; this is not offline auto-tuning.

The supported lowering domain remains narrower than affine analysis. Arbitrary
gather/scatter, unsupported stride/lane/cast, floating reductions, checked SIMD,
complex recurrences and effectful branches retain scalar execution. Boundary
envelope discovery and control exposure retain narrower, independently checked
contracts. General affine containment does not itself discover arbitrary
multidimensional footprints or restructure arbitrary control. Capability expansion
must preserve correctness, fallback and bounded work before changing those
contracts.
