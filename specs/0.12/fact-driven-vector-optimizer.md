# CK 0.12 Fact-Driven Vector Optimizer Specification

[简体中文](zh-CN/fact-driven-vector-optimizer.md)

## Status and authority

This is the pre-release design contract for CK 0.12.0. It defines the next
stage of the fact-driven optimizer without claiming that the described behavior
is present in the current 0.11.0 compiler. The released 0.11 language,
observable semantics, CLI behavior, and public Native C ABI remain authoritative
until 0.12.0 is implemented, accepted, and released.

The design is intentionally a single release contract. Implementation staging,
dated reviews, readiness notes, and acceptance evidence belong outside this
document.

## Objective

CK 0.12 turns the verified scalar knowledge introduced in 0.11 into automatic
data-parallel code. The compiler, rather than the programmer, discovers fixed
width SIMD, controlled unrolling, loop versioning, and fact-driven function
specialization. On eligible kernels the resulting Native machine code must
approach audited hand-written C/Rust plus SIMD while preserving the exact CK
safety, floating-point, effect, and ABI semantics.

The release has five connected deliverables:

1. canonical loop form and loop-access/dependence legality;
2. a deterministic target capability and profitability model;
3. proof-carrying Loop and SLP SIMD in KIR;
4. bounded loop unrolling/versioning and fact-driven specialization; and
5. structural, differential, performance, compile-time, and code-size gates.

## Fixed decisions

- SIMD is automatic. CK 0.12 adds no source vector type, intrinsic, pragma, or
  public vector ABI.
- Vector semantics live in KIR, not semantic MIR and not a backend-only side
  plan. KIR is still the only target-neutral optimization IR.
- Vector KIR uses fixed-width vectors and masks. Scalable vectors, SVE, and RVV
  are outside 0.12.
- Existing 'baseline' and 'native' CPU policies both participate. Each artifact
  contains one selected CPU version; runtime dispatch and baseline-plus-feature
  multiversioning remain 0.13 work.
- An accepted Native vector candidate must lower to real SIMD, as confirmed by
  object disassembly. Ineligible candidates remain scalar. C and WebAssembly
  continue to consume verified KIR and receive loop normalization, unrolling,
  and specialization, but 0.12 does not promise explicit C or WebAssembly SIMD.
- O3 owns the new code-duplicating and vector transforms. O0-O2 retain their
  0.11 pass contracts.
- Strict floating point, checked first-error behavior, print/runtime ordering,
  and public ABI behavior are unchanged. There is no fast-math mode.
- Contract-sanitizer builds disable specialization, loop versioning,
  vectorization, and unrolling. They keep the existing scalar pipeline and
  boundary instrumentation.

## Considered architectures

### Chosen: verified target-aware Vector KIR

The Native TargetMachine exposes a normalized capability and cost profile
before KIR optimization. CK uses its own facts, dependence analysis, cost model,
and independent certificate checker to create explicit vector KIR. LLVM lowers
verified vector operations and performs instruction selection, register
allocation, scheduling, and target legalization. LLVM target information can
describe machine capability or cost, but it cannot establish CK alias, range,
effect, bounds, or safety facts.

### Rejected: scalar KIR plus LLVM vectorization hints

Loop metadata alone would be smaller to implement, but LLVM would again own the
decisive legality and transformation. CK could not give the same explanation or
certificate for C/WASM, could not independently reject an invalid vector plan,
and could not reliably exploit CK-only contract facts.

### Rejected: independent backend vectorizers

Separate Native, C, and WebAssembly vectorizers would duplicate dependence,
versioning, and profitability logic and could disagree on safety. Backend
capabilities remain parameterized inputs to one KIR pass manager instead.

## Compilation architecture

The 0.12 flow is:

    source -> checked program -> semantic MIR
           -> consumer/mode-specific scalar KIR
           -> verified 0.11 scalar pipeline
           -> O3 specialization and canonical loop pipeline
           -> verified vector/unroll/SLP KIR when the target supports it
           -> C | WebAssembly | audited Native LLVM

Native commands must create the host TargetMachine and normalized optimization
profile before running the KIR pass manager. No LLVM object is visible inside
the optimizer; the profile is a plain, deterministic CK data structure.

C, WebAssembly, and default inspection profiles report vectorization disabled.
They never receive vector instructions and therefore do not need a hidden
scalarization pipeline. They still use the same KIR representation, verifier,
and all profitable non-vector 0.12 transforms.

"Target-neutral" describes KIR instruction semantics and verification, not a
requirement that every target receive an identical optimized graph. Lane count
and operation selection are parameterized by an immutable target profile, while
the meaning of every selected KIR operation is backend independent.

## Target optimization profile

'KirTargetProfile' schema 1 contains exactly:

- consumer and target identity, represented as either a normalized triple or an
  explicit portable-C/default-inspection pseudo-target;
- layout, represented as either known pointer width and endianness or
  'portable-unknown-layout';
- CPU identity, represented as Native policy ('baseline' or 'native') plus the
  normalized CPU name and complete sorted feature string, or 'not-applicable';
- legal fixed vector widths and legal lane types;
- operation legality for splat, arithmetic, unary, compare, select, cast,
  insert/extract, load, store, and supported integer reductions;
- aligned and unaligned memory legality and integer cost units;
- scalar, vector, mask, insert/extract, branch, and runtime-predicate costs;
- maximum legal interleave factor; and
- a digest covering every field plus the LLVM and bridge identities when they
  produced Native target data.

Operation legality and cost entries are keyed by operation, lane type, lane
count, arithmetic semantics, and memory alignment class where applicable.
Scalar entries are keyed by operation, scalar type, and semantics. The schema
does not contain an unspecified catch-all cost.

Native profiles must have a known layout and Native CPU identity. WebAssembly
uses its fixed WebAssembly layout and a versioned CK scalar cost table. Portable
C and default inspection use 'portable-unknown-layout', 'not-applicable' CPU,
a versioned CK generic scalar cost table, and no legal vector operations.
Unknown layout disables every layout-sensitive transformation, including
address-width predicates; it does not guess the eventual C compiler target.
These non-Native profiles remain deterministic and participate in the same
profile digest and cache identity without depending on LLVM.

Native baseline remains x86-64 with mandatory SSE2 or generic ARMv8-A with
ABI-mandated Advanced SIMD. Native policy may select wider fixed vectors when
the exact host feature set and cost profile justify them. Wider is not
automatically better. A native AArch64 host with SVE still uses a legal fixed
width profile in 0.12.

The pinned LLVM bridge may normalize TargetTransformInfo and legalization
queries into the profile. Each queried operation becomes either 'Legal { cost:
u32 }' or 'Unavailable'; an unavailable, invalid, negative, or unrepresentable
LLVM result becomes 'Unavailable' and disables candidates that require it.
Missing mandatory schema data, contradictory legality/layout data, or a zero
cost for emitted work makes the profile malformed and is a compiler error. Zero
is accepted only for an explicitly cost-free no-op such as a representation-
preserving cast. The same profile digest is part of cache and benchmark
identity.

Native profile construction uses one synthetic LLVM module with the exact
TargetMachine triple and data layout and one internal probe function carrying
that machine's normalized CPU and sorted feature attributes. All costs use
'TCK_RecipThroughput'. The finite query domain is the Cartesian subset actually
representable by KIR schema 2: the five lane types, fixed lane counts 2, 4, 8,
and 16 whose total width does not exceed 512 bits,
closed arithmetic/unary/compare/select/cast/insert/extract/reduction operations,
and power-of-two alignment classes from one byte through the vector byte width.
The bridge uses the operation-specific TTI arithmetic, compare/select, cast,
memory, vector-instruction, reduction, and control-flow queries rather than a
generic guessed cost.

Every operation key in that finite universe is present exactly once as 'Legal'
or 'Unavailable'; an unsupported target width is represented by 'Unavailable',
not by omitting the key. Masks use the same four lane-count candidates. This
probe universe is a schema limit, not a promise that every target supports every
width.

Because 'Mask { lanes }' deliberately has no scalar lane type, mask-only
operation costs use the 'i32' lane tag as a canonical schema sentinel. Every
non-sentinel 'MaskNot' key is 'Unavailable'; the bridge queries legalization and
cost against the actual fixed 'i1' mask type rather than an 'i32' vector.

Every entry also records LLVM's type-legalization part count and legalized type
identity. An invalid part count or scalarized/unsupported legalized form makes
the operation unavailable. The TTI operation cost already models its lowering,
so CK does not multiply it by the part count a second time. An invalid cost, a
valid cost below zero or above 'u32::MAX', and a non-whitelisted zero are
'Unavailable'. Canonical profile bytes use fixed numeric tags, big-endian
integers, length-prefixed UTF-8 strings, sorted feature names, and lexicographic
operation keys; the digest is SHA-256 over a schema tag and those bytes. Profile
construction is repeated in tests and fails if the same TargetMachine identity
does not produce byte-identical output.

## KIR v2 type and instruction model

Semantic MIR remains scalar and source ordered. KIR introduces 'KirValueType':

- 'Scalar(MirType)' for all existing values;
- 'FixedVector { lane, lanes }' where lane is i32, i64, u32, u64, or f64 and
  lanes is a positive u16; and
- 'Mask { lanes }' for lane predicates.

Pointers, slices, structs, void, and source bool are not vector lanes. Masks are
not integers and cannot escape an internal vectorized region or cross the
public ABI.

KIR instruction results and block parameters use 'KirValueType'; function
parameters, return values, calls, and exported storage remain scalar 'MirType'.
Vector and mask values cannot be function arguments/results or cross a block
edge outside their verified vector region.

KIR v2 adds closed instruction families for splat, contiguous load/store,
arithmetic, unary operations, compare, select, supported cast, lane extract,
and exact modular integer reduction. Vector binary operations are the existing
'Add', 'Sub', 'Mul', 'Div', and 'Mod' operations, restricted by the source type,
arithmetic semantics, and target profile. In particular, f64 'Mod' remains
invalid, and integer 'Div'/'Mod' requires both a no-failure proof and an
explicitly legal target operation. Every checked integer vector binary and
checked integer vector negate also carries a no-failure proof; an infallible or
strict-float operation cannot carry one. Vector unary 'Neg' follows the existing
numeric semantics; logical mask 'Not' is the only mask unary operation and can
represent source bool negation after comparison. It does not create a vector
bool lane type or carry a no-failure proof. Supported vector casts are exactly
the existing i32-to-f64 and u32-to-f64 casts. Each vector memory operation
records its region, Memory SSA input/output, lane type/count, byte footprint,
known alignment, and required alignment. There is no gather, scatter, masked
memory, vector call, or shuffle in 0.12. SLP permits only source-order identity
packing and emits no lane permutation.

A vector load/store footprint is exactly the union of the scalar bytes mapped
to its lanes. Neither KIR nor Native lowering may widen it across a slice end,
object boundary, or unmapped page. Prefix/tail handling and SLP packing therefore
never authorize speculative over-read or over-write.

Every consumer-specific optimized KIR module records the profile schema and
digest. The structural and certificate verifiers require the exact matching
profile; the default inspection module binds the target-independent generic
inspection profile rather than a host profile.

Version predicates are high-level, total KIR operations. They can check a trip
threshold, divisibility, target-width address interval non-overlap, or power-of-
two alignment. Predicate evaluation never dereferences memory. Address
addition/multiplication overflow yields false and selects the scalar fallback;
it never wraps into a stronger assumption. Non-overlap uses checked target-width
integer addresses rather than relational comparison of unrelated host-language
pointers. A zero-byte footprint is empty without forming an end address.

The structural verifier rejects mismatched lane counts/types, illegal mask use,
unsupported vector operations, inconsistent memory footprints, stale target
profiles, vector values escaping their region, and vector instructions in a
consumer profile with vectorization disabled.

## Canonical loop form

The existing CFG/SSA KIR remains the representation. 'loop-simplify' creates a
verified canonical form for reducible natural loops:

- one preheader with no loop-side predecessor;
- one canonical latch and one backedge to the header;
- dedicated exit blocks;
- normalized induction start, step, comparison, and trip-count expression;
- loop-closed SSA exit values; and
- explicit 'LoopId' descriptors with parent/depth, blocks, exits, inductions,
  and effect summary.

Loop descriptors are deterministic, non-authoritative analysis results. CFG,
inlining, specialization, unrolling, or vectorization invalidates affected
descriptors and their dependent facts. The pass manager rebuilds dominance,
Memory SSA, contract-instance mapping, and loop descriptors before a consumer
can reuse them.

Irreducible loops, multiple latches that cannot be normalized without changing
effects, non-affine induction, or budget exhaustion remain valid scalar KIR and
receive a stable conservative explanation.

## Loop access and dependence legality

An eligible memory access is contiguous and affine in one canonical induction:

    byte_address(iteration) = base + element_size * (a * iteration + b)

with target-width overflow either statically impossible or covered by a false-
on-overflow version predicate. The first release requires positive unit-stride
('a = 1') vector groups; negative or other affine strides may help prove
disjointness but are not vectorized, gathered, or scattered.

The analysis uses, in order:

1. existing region partitions, Memory SSA, noalias, readonly/writeonly,
   alignment, slice interval, range, and effect facts;
2. exact same-base offset/distance reasoning and conservative integer affine
   dependence tests; and
3. an optional runtime non-overlap/alignment/trip predicate for a cloned fast
   path.

Every potentially loop-carried read/write pair is classified as independent, a
proven supported reduction, dependent, or unknown. A dependent pair blocks
vectorization. Unknown write/write or read/write dependence also blocks it
unless one conjunction of permitted runtime predicates proves the complete
footprints disjoint. Read/read pairs never create an ordering dependence.

Calls, runtime operations, print, volatile-like effects, unknown memory, and
ordered failure block cross-iteration reordering unless an earlier verified
transform removes the call/effect or proves it irrelevant. The vectorizer never
invents noalias from different source names or raw pointers.

## Loop versioning

One loop may have at most two paths:

- one SIMD fast path whose complete assumptions are checked or statically
  proven; and
- the original scalar loop as the unchanged fallback.

The transform keeps the original scalar blocks rather than reconstructing an
equivalent loop. All runtime predicates execute before the first original loop
effect. A loop version contains at most four conjunctive atomic predicates and
no disjunction. A false predicate, address overflow, insufficient trip count,
misalignment, or overlap selects the scalar path.

The fast path requires at least two complete vectors before its scalar
epilogue; the profitability model may raise that threshold. Tail iterations
execute in a scalar epilogue in original order. Version 0.12 does not peel a
scalar alignment prefix: it uses a profile-legal unaligned operation, proves
alignment, adds an alignment predicate, or rejects the candidate. Versioning
cannot suppress an empty loop, move an observable effect, or change which
checked operation reports the first error.

## Proposal and independent verification

Analysis and cost modelling propose transformations; they do not authorize
them. The append-only proof language gains closed steps for:

- canonical loop and exact trip partition;
- induction and affine access mapping;
- static alias/dependence classification;
- runtime predicate completeness and false-on-overflow behavior;
- lane-to-scalar iteration mapping;
- vector operation equivalence and memory footprint;
- reduction associativity under the exact arithmetic semantics;
- scalar fallback identity and epilogue coverage;
- specialization fact scope and clone argument mapping; and
- target-operation legality, cost decomposition, code growth, and budget
  accounting.

A 'VectorizationPlan' records the input LoopId, VF, UF, scalar-to-vector map,
memory groups, optional predicates, epilogue, target-profile digest, estimated
cost, code growth, and proof roots. SLP and specialization use analogous closed
records.

The pass manager keeps two disjoint state layers. 'KirVerifiedProgramState'
owns the module, contract facts, proof arena, eliminated guards, verification
cache, evidence generation, and deterministic IR ID allocators. It is the only
state copied by a speculative transaction and is committed or rolled back as a
whole. 'KirOptimizationAuditState' owns frozen proposer/checker budget ledgers,
the monotonic attempt sequence, accepted/rejected counters, stable explanations,
and budget fallbacks. Audit state is append/debit only and never rolls back with
KIR.

Verification and analysis caches may reduce latency only under exact structural
identities. An immutable target profile memoizes its complete validation result;
copy-on-write mutation clears that result. After a changed pass, every changed
function is fully revalidated, exact unchanged functions reuse their prior
structural verdict, and module-wide function/block/instruction/value/region/
memory identity uniqueness plus the complete fact/proof/rewrite verifier still
run. Dominance reuse is keyed only by the ordered block/successor CFG on which
dominance depends and re-debits the identical deterministic analysis budget.
Discovery-only loop descriptors may omit the cryptographic CFG digest, but any
materialized proposal or certificate recomputes the full digest. If discovery
proves that specialization, vector/SLP, and unroll candidate sets are empty, the
pass manager records verified no-op stages without allocating a speculative
program-state copy. A no-op frontier may reuse the preceding discovery-only loop
descriptors only while no intervening pass changes induction structure and each
cached descriptor set retains the exact function identity; otherwise it
recomputes the analysis. None of these rules changes a candidate, checker,
budget, cost, profitability, or benchmark threshold.

Proposal and checker steps debit the outer audit ledger as they execute; a
rejection, reused specialization, or non-winning frontier candidate receives no
refund. Audit records identify a candidate by its pre-transaction source/KIR
identity, kind, VF, and UF, never by a trial-only ID. Candidates are enumerated
in pipeline stage order with these unique stable keys: specialization uses
caller FunctionId, call InstructionId, callee FunctionId, and canonical fact-set
digest; loop-frontier work uses FunctionId, LoopId, kind rank Loop SIMD/full
unroll/partial unroll, scalar-or-SLP variant rank, then ascending VF/UF; residual
SLP uses FunctionId, BlockId, root InstructionId, then ascending lane count. A
stage completes before the next begins and the shared ledger never resets.
Acceptance swaps only the verified program-state snapshot and appends an
accepted audit record; rejection discards only that snapshot and appends its
stable reason.

The independent checker reads the pre-transform KIR, proposed record, target
profile, facts, and proofs. It does not call the vectorizer, dependence analyzer,
or proposer cost model and does not accept their conclusion as a premise. It
recomputes legality, integer cost totals, profitability thresholds, structural
growth, and budget consumption from the closed record. Only after the checker
accepts the complete proposal is the transformation committed. The post-
transform structural/evidence verifier then runs normally.

Budget exhaustion before commitment discards the whole proposal and preserves
scalar code. A malformed or false certificate, or a post-commit verification
failure, is a compiler error and withholds every artifact.

## Loop SIMD

The Loop vectorizer initially accepts innermost canonical loops with a countable
trip expression, unit-stride memory, target-legal fixed vectors, and no
unresolved ordered effect. It supports lane-wise i32/i64/u32/u64/f64 arithmetic,
compare, supported cast, mask select, contiguous load/store, and splats.

Strict f64 operations remain separate instructions with the same per-element
rounding. FMA contraction and cross-lane reassociation are forbidden. A pure
element-wise f64 loop is eligible; a floating reduction is not.

A single side-effect-free diamond inside an eligible loop may be if-converted
to compare, mask, and select when both arms reconverge immediately, define the
same scalar results, and contain no memory access, guard, call, runtime effect,
or certificate-scoped operation. All other control predication remains scalar.

Unchecked modular integer addition and multiplication reductions may be
vectorized when the target supports the operation and the checker proves the
exact lane partition and horizontal fold. Checked integer reductions remain
scalar in 0.12. Other reductions, scans, recurrences, gather/scatter, complex
predication, and interleaved memory are outside scope.

For checked element-wise operations, the fast path is legal only when existing
facts or permitted version predicates prove that every vector lane cannot fail.
The scalar fallback retains every original guard. Per-lane recovery from a
vector failure is not implemented.

## SLP SIMD

SLP operates on straight-line scalar DAGs after scalar full-unroll opportunities
and before final cleanup. It packs isomorphic, independent operations with the
same lane type and arithmetic semantics. Memory packs must be contiguous and
ordered consistently with Memory SSA. Packing cannot cross a guard, call,
runtime/print effect, unknown write, block boundary, or certificate dependency.

The initial SLP set supports splats, lane-wise arithmetic, comparisons, casts,
selects, and contiguous load/store. It does not perform speculative predication,
arbitrary shuffle synthesis, horizontal f64 operations, or partial vector
calls. A rejected pack leaves all scalar instructions unchanged.

Overlapping residual SLP alternatives with the same function, block, and root
are proposed and independently checked from one immutable pre-state in stable
ascending key order. The winner is the valid alternative with the greatest
absolute modeled cost reduction, then lower transformed cost, smaller code
shape, and stable key. Every other valid alternative is charged and recorded as
a non-winner; exactly one winner may commit. This prevents an earlier narrow
pack from destroying a more profitable wider pack.

## Controlled unrolling

Unrolling is deterministic and cost driven:

- vector interleave/unroll factor is one of 1, 2, or 4 and cannot exceed the
  target profile limit;
- a constant-trip scalar loop may be fully unrolled only when trip count is at
  most 8, the original body contains at most 16 KIR instruction units, and the
  common code-growth budget is satisfied; and
- other scalar partial unrolling uses factor 2 or 4 only when it removes enough
  branch cost by itself or when a trial unroll plus SLP plan meets the
  profitability threshold. An SLP-justified unroll and its pack form one
  independently checked transaction; neither half may be committed alone.

Every scalar full-unroll, scalar partial-unroll, and combined unroll-plus-SLP
proposal must predict at least 10 percent total loop execution-cost reduction
and at least two absolute cost units against the same frozen pre-state. These
requirements are additional to the trip, body, factor, and growth limits above.
Loop SIMD retains its stricter 20 percent threshold. A frontier proposal first
passes its own threshold and independent checker; only accepted proposals enter
the common winner comparison.

The checker proves iteration coverage, order-sensitive effect preservation,
phi/LCSSA mapping, and exact remainder behavior. Unrolling never duplicates a
potentially observable failure or call across a point where the scalar program
could have stopped.

## Fact-driven function specialization

O3 may clone an internal direct-call target for a canonical set of dominating
facts: exact scalar/bool constants, exact slice length, proven alignment,
complete noalias relationships, and readonly/writeonly/effect summaries.
Trusted-contract facts retain their call-instance scope and can specialize only
the dominated instance they authorize.

Export names, signatures, thunks, and public ABI never change. A generic body is
retained. Clone names are internal deterministic digests and cannot be exported.
Recursive SCCs, indirect calls, address-taken functions, runtime calls, and
sanitizer mode are not specialized in 0.12.

Specialization runs after the O1 fact/check prefix and before O2 inlining so a
clone can expose constant folding, check elimination, loop bounds, and later
vectorization. A trial owns a complete copy of 'KirVerifiedProgramState':
module, contract facts, proof arena, eliminated-guard records, verification
cache, evidence generation, and deterministic IR ID allocators. Its work is
charged directly to the outer 'KirOptimizationAuditState'.
It substitutes the scoped facts, creates and redirects the clone, and runs only
bounded clone-local CFG/SCCP/range/check/DCE scalar finalization. Nested
specialization and interprocedural inlining are disabled during the trial.

The specialization checker independently verifies fact scope, argument and ID
mapping, scalar cost reduction, code growth, and both caller/callee budget
charges against the copied pre-state. Acceptance requires the normal 10 percent
and two-unit specialization threshold from this materialized scalar reduction;
predicted but uncommitted vector benefit cannot make an otherwise unprofitable
clone pass. On acceptance the complete verified program-state copy atomically
replaces the pre-state; on rejection only that copy is discarded while audit
charges and the rejection explanation remain. The accepted clone then traverses
normal O2 and the remaining O3 loop/vector pipeline exactly once, with no trial
proof or descriptor migration and no second specialization root.

A specialization clone is never itself a specialization root. Equal canonical
fact sets reuse the same digest-named clone, and the limits count only distinct
fact sets. The pass manager charges trial work even when a clone is reused or
rejected.

One original function has at most three specialized clones and one module at
most 24. The specialization instruction-growth allowance is:

    max(64, min(4096, ceil(pre-specialization module KIR units / 4)))

No clone beyond that shared allowance is committed.

## Deterministic profitability and budgets

Costs are non-negative integer units. The model combines target-profile
operation costs with CK-owned trip ranges, alignment, alias, effects, vector
setup, runtime predicates, scalar epilogue, and code-growth costs. It uses no
wall clock, unordered iteration, machine load, or profile feedback.

For a loop candidate it compares scalar iteration cost with vector body,
predicate, and epilogue cost over an exact or conservative trip estimate. An
unknown trip count adds a runtime threshold equal to at least the computed
break-even and '2 * VF'. A vector loop must predict at least 20 percent execution
cost reduction. An SLP pack or specialization must predict at least 10 percent
local reduction and at least two absolute cost units. Ties select the smaller
code shape, then lower VF/UF, then source/KIR identity order.

Fixed structural limits are:

- at most one SIMD version plus one scalar fallback per loop;
- at most four runtime predicates per loop;
- maximum unroll factor 4;
- transformed loop instruction units no greater than three times the original
  loop units plus 32 control units;
- specialization limits stated above; and
- aggregate post-0.12 O3 KIR instruction units no greater than twice the KIR
  units immediately before specialization.

All 0.12 specialization, Loop SIMD, SLP, versioning, and unroll proposal work in
one function shares '64 * pre_transform_function_kir_units + 128' steps. Their
independent checkers share '96 * pre_transform_function_kir_units + 256' steps.
'pre_transform_function_kir_units' is frozen at that original function's O3
entry; clones use their original function's frozen count. A specialization
trial charges both its caller and original callee budgets; rejection and clone
reuse do not reset either budget. Arithmetic is saturating u32. Exhaustion is a
conservative rejection with a stable reason and no partial mutation.

## O3 pipeline order

O0, O1, and O2 retain the 0.11 sequence. O3 runs:

1. the O1 CFG/SCCP/range/check prefix;
2. fact-driven direct-call specialization with isolated complete-state scalar trial
   finalization, then CFG/SCCP/check refresh;
3. the existing O2 inline, Memory SSA, GVN, forwarding, DSE, propagation, and
   check cleanup;
4. loop-simplify and canonical descriptor verification;
5. natural-loop/induction analysis, LICM, induction simplification, and scalar
   propagation/check cleanup;
6. rebuild canonical loop, dependence, Memory SSA, and cost descriptors and
   freeze one immutable scalar pre-state per innermost loop;
7. from that same pre-state, propose Loop SIMD (including optional versioning
   and target-bounded VF/UF), small constant full-unroll and scalar partial-
   unroll alternatives, each either scalar-only or atomically followed by SLP;
   independently verify
   every proposal, compare accepted alternatives by predicted total cost, code
   shape, lower VF/UF, then KIR identity, and transactionally commit at most one
   winner per loop; non-Native profiles propose only scalar unroll alternatives;
8. rebuild descriptors after the selected loop transactions;
9. residual Native SLP planning outside committed loop regions, independent
   verification, and transactional rewrite;
10. final SCCP where scalar values remain, DCE, Memory SSA cleanup, evidence
    validation, and structural verification.

Each named pass records changed/verified state. Any CFG-changing step explicitly
declares preserved analyses; everything else is invalidated and rebuilt.

Optimization statistics add canonicalized/versioned/vectorized loop counts,
SLP pack and vector-operation counts, scalar epilogues, each unroll factor,
specialized clone counts, rejected-candidate counts by stable reason, and every
analysis-budget fallback. Counts use deterministic KIR identity order.

## Backend contracts

### Native LLVM

Verified fixed vectors lower structurally to LLVM fixed vector types and
operations. Masks lower to the target-legal predicate form. Unaligned access is
emitted only when the profile marks it legal; alignment attributes never exceed
verified alignment. Strict f64 disables contraction/reassociation. CK loop and
vector facts are audited before LLVM optimization just like existing alias,
range, and alignment strengthenings.

The bridge ABI advances because it gains normalized target cost/capability
queries and vector construction operations. LLVM remains pinned to 22.1.8 for
0.12 unless a separate reviewed toolchain change proves equivalent contracts.

### C and WebAssembly

Their 0.12 target profiles disable vector KIR. They continue from verified
scalar KIR and can receive specialization, canonical-loop cleanup, and
controlled scalar unrolling. Generated C remains portable standard C; WASM does
not silently require SIMD128. Adding explicit SIMD for either backend requires a
separate versioned design.

## Explanations, inspection, and fallback

'--explain-optimization' extends its deterministic output with candidate kind,
LoopId or pack/call identity, selected/rejected status, VF/UF, predicates,
estimated scalar/vector cost, code growth, proof roots, and one stable reason.
Required rejection reasons include unsupported consumer/target, sanitizer mode,
irreducible or noncanonical loop, unknown trip, unresolved dependence,
unsupported effect, strict-float reduction, illegal target operation,
profitability threshold, code-size budget, and analysis budget.

'emit-kir' keeps its existing default inspection behavior, which is scalar and
target independent. It gains '--consumer
inspection|c|wasm|native-library|native-executable'; the default is
'inspection'. '--cpu baseline|native' is valid only with the two Native
consumers and defaults there to 'baseline'. Native consumers require a compiler
built with the native-toolchain feature, and 'native-executable' requires the
same valid 'main' entry accepted by executable build/run. This prints the exact
final KIR for that consumer/profile rather than inferring artifact kind from
source contents. 'emit-llvm' uses Native baseline. 'build' uses its selected CPU
policy and 'run' uses native as before.

Unsupported candidates and analysis budgets are normal conservative fallbacks.
Invalid target identity, invalid certificates, stale evidence, or invalid
post-transform KIR are compiler errors and produce no partial output or cache
entry.

## Compatibility, ABI, and cache identity

CK source syntax, type system, semantic MIR, diagnostics, strict f64, checked
status/first-error rules, slice ABI, public symbols, and Native C ABI version 1
remain unchanged. Runtime ABI stays version 2 because no runtime helper is added.

The KIR print/schema identity advances from 'kir-v1' to 'kir-v2'; 0.12 does not
add a persistent serialized KIR artifact cache. The existing Native object/run
cache advances to entry magic 'CKCOBJ02', key schema 3, and manifest schema 3.
Its key and manifest cover the complete target-profile digest, vector
cost-model schema, vector proof schema, and all fixed budget constants in
addition to the existing identities. The private LLVM bridge ABI advances from
2 to 3. Compiler and package version become 0.12.0 only during implementation.
A 0.11 object/cache entry fails the entry or identity check and cannot be
accepted as 0.12.

Vector and specialization clone symbols are internal and excluded from headers,
exports, dynamic symbol tables, and public ABI audits.

## Verification strategy

Acceptance must include all of the following without ignored tests or weakened
thresholds:

1. Every 0.11 language, ABI, CLI, artifact, runtime, sanitizer, differential,
   mutation, performance, and six-host contract remains green.
2. Unit and mutation tests cover loop normalization, LCSSA, trip partition,
   affine overflow, dependence distance, runtime predicate completeness, lane
   maps, masks, vector memory footprints, reductions, fallback identity,
   unroll coverage, clone fact scope, target illegality, stale profile/proof,
   forged cost/growth records, and atomic budget fallback.
3. Generated differential kernels compare O0 scalar semantics with O3 results
   over zero, short, exact-vector, remainder, maximum-safe, overlapping,
   disjoint, aligned, misaligned, checked, and unchecked inputs.
4. Adversarial cases retain scalar execution for irreducible control flow,
   unknown write dependence, calls/effects, strict f64 reductions, possible
   first error, overflowing address predicates, and over-budget modules.
5. KIR and pre-LLVM structural tests prove an accepted vector plan exists and
   contains the expected vector operations. Pinned object disassembly on x86-64
   and AArch64 proves real SIMD instructions, so an LLVM scalar fallback cannot
   falsely satisfy acceptance.
6. Baseline and native CPU policies receive correctness and feature-containment
   tests on every supported host. Native machine code may use only the resolved
   feature string; baseline artifacts must not use optional ISA features.
7. The exact final candidate SHA passes the required ten-job matrix: quality,
   Native integration, six host targets, and x86-64/AArch64 performance.

## Performance and size contract

The strict Native performance report advances from schema 6 to schema 7 and
names candidate version 0.12.0. It pins compiler, source, target profile,
cost-model/proof schema, CK artifacts, oracle artifacts, sampling schedule, and
every source digest. The pinned 0.11 replay compiler is commit
'80c0acf6bb5d65e4d9d40352b9501ea32b79f43d'. Its independently built compiler,
Native artifacts, fixed independent C oracle, recipe, and digests are retained
like the existing 0.10 replay bundle.

The scalar-regression protocol is 'rotating-twelve-channel-v1'. Channel order
is exactly 'candidateNativeUnchecked', 'candidateNativeChecked',
'currentClangUnchecked', 'currentClangChecked',
'replayV011NativeUnchecked', 'replayV011NativeChecked',
'replayV011ClangUnchecked', 'replayV011ClangChecked',
'replayV010NativeUnchecked', 'replayV010NativeChecked',
'replayV010ClangUnchecked', and 'replayV010ClangChecked'. Warm-up has three
rows and sampling has twenty; row r is '[(r + i) % 12 | i in {0, ..., 11}]'. All
streams execute in one process on the same inputs and retain the existing seven
calls per sample and fixed batch identity. Schema 7 records every actual order,
sample, upper median, artifact digest, and result; a missing stream cannot fall
back to historical numbers.

Measurements use portable baseline policy on stable x86-64 and AArch64 workers;
native-policy measurements are diagnostic unless a separately fixed hardware
identity is approved.

Hand-written SIMD C is built by pinned Clang 22.1.8 and hand-written SIMD Rust
by pinned Rust 1.90.0, using architecture-specific baseline flags, disabled fast
math/contraction, and no CPU feature unavailable to the CK baseline profile.
Both implementations must pass differential and undefined-behavior auditing on
the fixed declared valid input domain. Their manifest names every precondition;
an input may be excluded only by that pinned manifest, never after measurement.
A missing or invalid C or Rust artifact fails the gate rather than removing that
competitor.

Vector and domain-fact runtime gates use 'rotating-three-channel-v1' separately
for checked and unchecked CK: candidate CK, pinned C, and pinned Rust. Each run
uses three warm-up rows and twenty sample rows; row r rotates the three channels
by 'r % 3'. All channels run in one process on identical inputs with the same
seven calls per sample, fixed batch identity, and upper-median statistic. Every
actual order and sample is recorded. The generic domain-fact gate substitutes
the pinned generic Clang and Rust artifacts for the hand-SIMD artifacts. Each
dynamic library is opened and its typed kernel entry is resolved exactly once
before correctness, warm-up, or sampled batches. The timed batch performs only
calls through that cached entry; dynamic symbol lookup and per-call string
dispatch are outside the timing region.

The release gates are cumulative:

- all existing 0.11 Native/Clang, 0.11/0.10 replay, checked/unchecked, and
  optimizer-latency limits remain unchanged;
- for each audited vector-eligible kernel, the oracle is the faster valid median
  from its fixed hand-written C-plus-SIMD and Rust-plus-SIMD implementations;
  separately in checked and unchecked mode, CK throughput on each of x86-64 and
  AArch64 is at least 95 percent of the geometric mean of those per-kernel
  oracles, and every kernel reaches at least 90 percent of its oracle;
- on a separate domain-fact suite where CK contracts expose constraints absent
  from the fixed generic source, each per-kernel generic oracle is the faster
  valid median from pinned Clang O3 and Rust O3, and CK exceeds the geometric
  mean of those oracles by at least 5 percent on each architecture;
- hand-written SIMD oracles receive every equivalent precondition expressible
  by their source language and preserve CK strict-float and safety semantics;
- the unchanged scalar regression corpus is no more than 3 percent slower in
  geometric mean and no individual case more than 8 percent slower than an
  independently replayed pinned 0.11 compiler;
- the Native artifact-size suite is no more than 35 percent larger in aggregate
  than the pinned 0.11 compiler, and no individual artifact more than 2.5 times
  larger;
- for baseline O3 source-to-relocatable-object compilation, the geometric mean
  of candidate/replayed-0.11 time ratios is no more than 1.5 and no individual
  ratio is more than 2; and
- KIR analysis/optimization still satisfies the existing suite-median 2x and
  individual 3x limits relative to the fixed 0.10 MIR optimizer.

The artifact-size corpus emits paired baseline Native relocatable objects for
both safety modes from the exact same sources. Size is the exact object byte
length before archive or link; caches, debug sidecars, and distribution
containers are excluded. The source-to-object compile-time corpus uses the same
source/mode pairs, fresh output paths, disabled artifact caches, three warm-up
pairs, and fifteen measured pairs. Candidate-first and replay-first order
alternates, and the upper median is reported. Missing, failed, or mismatched
objects fail both gates.

The vector corpus includes at least contiguous map/zip, strict element-wise f64,
integer transform, exact modular integer reduction where target legal, SLP from
small unrolled bodies, runtime noalias versioning, and specialization that
exposes a fixed slice length. Memory-bound and compute-bound cases are both
required. Changing a source, compiler identity, threshold, statistic, target
profile, or exclusion rule is a reviewed contract change and cannot be done to
make a candidate pass.

## Completion and future boundary

CK 0.12 is complete only when the exact candidate SHA satisfies every semantic,
structural, performance, compile-time, size, cache, and six-host gate above and
the current English/Chinese documentation agrees with the implementation.

The following remain outside 0.12: source SIMD types/intrinsics, fast math,
floating reduction reassociation, gather/scatter, arbitrary shuffle synthesis,
masked fault recovery, complex loop predication, scalable vectors, GPU targets,
cross-compilation, public JIT APIs, profile feedback, runtime CPU dispatch, and
Auto-Tuning. Baseline-plus-feature multiversioning and PGO remain 0.13; offline
Auto-Tuning remains 0.14.
