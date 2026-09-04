# CK 0.13 Profile-Guided Multiversioning Specification

[简体中文](zh-CN/profile-guided-multiversioning.md)

## Status and authority

This is the pre-release design contract for CK 0.13.0. It is based on the CK
0.12 final-candidate tree and does not claim that profile-guided optimization
or runtime CPU dispatch exists in the current compiler. The CK 0.12 language,
observable semantics, CLI, public Native C ABI, safety rules, and optimizer
contract remain authoritative until this specification is implemented,
accepted, and released.

If the 0.12 candidate changes before it lands, this design must be rebased and
reviewed against the changed contract before implementation continues. Reviews,
implementation staging, and acceptance evidence belong outside this document.

## Objective

CK 0.13 adds workload knowledge and portable CPU specialization without making
normal CK development depend on a training step. It combines CK-owned static
facts with bounded execution-frequency evidence, then emits one portable
baseline and only the profitable feature variants of eligible Native kernels.
The resulting artifact selects the best compiler-ranked compatible variant once
at runtime and retains the exact CK safety, strict floating-point, effect, and
ABI semantics.

The release has five connected deliverables:

1. a stable, bounded, CK-owned profile schema and deterministic identity;
2. profile instrumentation, atomic shards, merge, inspection, and use;
3. profile-guided layout, inlining, specialization, loop, and SIMD decisions;
4. bounded baseline-plus-feature Native multiversioning and runtime dispatch;
5. semantic, adversarial, six-host, performance, size, and compile-time gates.

## Fixed decisions

- PGO is optional and off by default. `check`, `run`, ordinary `build`, existing
  test harnesses, and ordinary release builds require no training and collect
  no profile data.
- CK source syntax, the no-argument `main()` contract, the type system, strict
  `f64`, checked first-error behavior, effects, slice semantics, and public ABI
  do not change in 0.13.
- CK owns the public `.ckprof` format, profile identity, confidence rules, and
  KIR optimization decisions. LLVM profile formats are private implementation
  details and are never accepted as CK profile inputs.
- Profile observations establish profitability, never safety. An observed case
  cannot eliminate a check or prove an alias, range, alignment, effect, or
  bounds fact. Speculative fast paths retain verified guards and a generic
  fallback.
- Instrumentation is present only in a profile-generation artifact. A final
  profile-use artifact contains no counters, profile writer, output path, or
  profile collection dependency.
- Final profiles are terminal aggregates. Schema 1 merge accepts completed raw
  shards only, so one recorded run cannot be hidden inside overlapping nested
  aggregates and counted twice.
- `--cpu multiversion` is explicit. It keeps one ABI-compatible baseline and
  emits at most two profitable enhanced variants per eligible root. Ordinary
  `baseline` and `native` policies retain their 0.12 meanings.
- Runtime CPU detection is fail-closed. Unknown, contradictory, unavailable, or
  unsupported feature information selects baseline; it never selects a variant
  optimistically.
- The public address of an exported function remains a stable dispatcher thunk.
  Variant functions and support symbols are hidden and are not public ABI.
- PGO use is available at O2 and O3. O2 consumes frequencies only through a
  CK-owned late machine-layout plan and emits no profile-derived LLVM metadata.
  PGO-influenced inlining,
  specialization, loop cloning, and multiversioning are O3-only. Profile
  generation uses one fixed, versioned instrumentation pipeline.
- Contract-sanitizer mode is incompatible with profile generation, profile use,
  and multiversioning in 0.13. Invalid combinations fail explicitly rather than
  silently changing policy.
- Native is the only 0.13 PGO/multiversion consumer. C, WebAssembly, default KIR
  inspection, public JIT APIs, and cross-compilation gain no hidden profile or
  dispatch behavior.
- There is no runtime adaptive recompilation, source SIMD, fast math, floating
  reassociation, workload invention, or search-based Auto-Tuning. Offline
  Auto-Tuning remains 0.14 work.

## Existing foundations and release boundary

CK 0.13 reuses rather than replaces the 0.11 and 0.12 foundations:

- canonical SSA KIR, Memory SSA, region identity, effect summaries, and the
  independent fact/proof checker;
- scalar range, congruence, known-bit, alias, alignment, slice, and contract
  facts;
- deterministic transactions, audit ledgers, cost units, and bounded rewrite
  budgets;
- canonical loops, dependence analysis, specialization, unrolling, SLP, Loop
  SIMD, scalar fallback, and target optimization profiles; and
- the pinned LLVM 22.1.8 Native bridge, ORC/JITLink runtime, lld artifact path,
  cache identity, and six release hosts.

No 0.12 threshold is weakened to make a 0.13 candidate pass. A PGO or target
variant starts from the same verified pre-transform KIR and must pass the same
structural, proof, effect, failure-order, and backend audits as an ordinary O3
artifact.

CK 0.13 supports direct calls only because the language has no function-pointer
call surface. Indirect-call target profiling and promotion are deliberately not
invented by this release; adding function pointers requires a separate language
and ABI design.

## Considered architectures

### Chosen: CK-owned profiles applied at KIR

CK defines instrumentation sites and `.ckprof`, validates them against the
canonical pre-profile KIR, makes bounded KIR decisions, and emits verified LLVM
attributes and metadata after lowering. This keeps the durable contract
independent of LLVM and allows runtime frequencies to combine with CK-only
range, alias, alignment, slice, and effect information.

### Rejected: expose LLVM raw profiles

Passing through LLVM instrumentation would be faster to prototype, but it
would bind CK users, caches, diagnostics, and compatibility to a pinned LLVM
format. It would also place decisive site mapping and policy outside CK's
independent verifier.

### Rejected: adaptive JIT profiling

Continuous observation and recompilation could adapt to a changing workload,
but it adds warm-up latency, memory use, runtime machinery, nondeterminism, and
deployment differences between executables and libraries. It conflicts with
the zero-extra-dependency static artifact path and remains outside 0.13.

## Compilation architecture

The Native O2/O3 flow becomes:

    source -> checked program -> semantic MIR
           -> consumer/mode-specific scalar KIR
           -> canonical pre-profile KIR and site table
           -> verified static analyses and 0.12 scalar/vector foundations
           -> optional validated CK profile annotations
           -> bounded PGO decisions
           -> optional multiversion plans from one immutable pre-state
           -> independently verified KIR variants
           -> per-variant LLVM modules and PassBuilder pipelines
           -> dispatcher + artifact assembly

The three profile modes are closed:

- `off`: no site table is materialized beyond optional inspection and no
  instrumentation or profile identity affects code generation;
- `generate`: a fixed canonical pipeline creates the site table and inserts
  profile operations after site identity is frozen; and
- `use`: the compiler recreates the same table, validates the complete profile,
  imports counts as non-proof analysis facts, and runs the selected O2/O3 path.

Profile counter operations live in a dedicated KIR effect domain. They cannot
be deleted, duplicated, or moved across the event they count, but they do not
alias CK program memory and do not create false Memory SSA barriers. The
generation pipeline performs only transformations proven to preserve a
one-to-one mapping between a counted event and its canonical site.

## CLI and workflows

Ordinary commands are unchanged:

```text
ckc run app.ck
ckc build app.ck --out app
```

The convenience path for an executable is:

```text
ckc pgo build app.ck --out app [--profile-out app.ckprof]
```

It transactionally builds a temporary instrumented executable, runs its
no-argument `main()` once, validates and merges the completed shard, writes the
final `.ckprof`, and builds the final O3 artifact. The child inherits the
current working directory, standard streams, and environment. Training is real
program execution, not a sandbox; its side effects are the user's explicitly
requested workload side effects. A non-zero exit, signal, missing/empty shard,
write failure, profile error, or final build error leaves no final artifact.

CK 0.13 does not expose `argv` to source `main()`. Consequently the convenience
command does not pretend that command-line inputs reach CK code. The executable
must exercise a representative workload from its own `main()`. Libraries and
multiple workloads use the explicit path and a user-owned host/test harness:

```text
mkdir profiles

ckc build kernels.ck --kind dynamic --out kernels-profiled \
  --pgo-generate profiles/ --cpu multiversion

# One or more host/test runs load kernels-profiled, exercise real workloads,
# quiesce CK calls, and invoke the generated ck_profile_flush_* control entry.

ckc pgo merge profiles/ --out kernels.ckprof

ckc build kernels.ck --kind dynamic --out kernels \
  --pgo-use kernels.ckprof --cpu multiversion -O3
```

Inspection is read-only:

```text
ckc pgo inspect kernels.ckprof
ckc pgo inspect kernels.ckprof --json
```

The CLI contract is:

- `--pgo-generate <directory>` and `--pgo-use <file>` are mutually exclusive;
- profile generation rejects O0/O1 and uses its fixed generation pipeline;
- profile use accepts O2/O3, while `--cpu multiversion` requires O3;
- `--pgo-generate` is accepted only by Native `build` for executable, dynamic,
  and static artifacts. Object generation is rejected because an unlinked
  object has no defined process/library lifetime or flush owner;
- `--pgo-use` is accepted by Native `build` and Native `emit-kir`.
  `--cpu multiversion --kind object` is rejected in 0.13 because
  the dispatcher and variants are separate audited modules and schema 1 does
  not define a multi-object bundle or cross-platform partial-link product.
  Single-version baseline/native profile-use objects remain supported;
- Native executable and Native library are the two profile-topology classes.
  Dynamic, static, and object artifacts share the Native-library topology, so a
  compatible profile generated through a temporary dynamic or static library
  may be used for a baseline/native object. Native `emit-kir` validates the
  topology selected by its `--consumer`, not a physical artifact kind;
- `--cpu` becomes `baseline|native|multiversion` for Native `build` and Native
  `emit-kir`; portable consumers reject `native` and `multiversion` as today;
- `ckc pgo build` accepts only executable input with a valid `main()` and
  defaults to O3; libraries use the explicit workflow;
- `--profile-out` defaults to `<out>.ckprof` and cannot alias the final artifact;
- `<directory>` for `--pgo-generate` must already exist, must be a real
  directory, and must have no symlink or reparse-point component. The compiler
  resolves it against its build-time current directory, normalizes and
  validates the absolute path, captures its platform file identity, and embeds
  that path and identity only in the temporary generation artifact. The
  operational path and directory identity are excluded from profile,
  final-artifact, and cache identity. At runtime the collector reopens the
  directory with
  component-wise no-follow/reparse-point checks and anchors every temporary and
  completed file operation to that verified directory handle; replacement
  between build and execution is rejected. A platform/filesystem without a
  stable directory identity rejects generation;
- generation with `--cpu multiversion` binds the intended target-set identity
  but executes one instrumented baseline implementation; it does not train by
  dispatching among already optimized variants;
- generation artifacts bypass the Native object cache because their output
  directory is operational state, not reproducible final code identity; and
- `build-llvm` remains deprecated and gains no PGO or multiversion behavior.

Existing transactional output rules cover the final artifact, profile, header,
and sidecars as one output set. A pre-commit failure preserves every prior
destination.

## Profile identity

`CkProfileIdentity` schema 1 contains exactly:

- profile format and profile-contract schema identities;
- compiler package version, compiler source identity, and profile runtime
  identity;
- canonical semantic module graph and canonical pre-profile KIR digest;
- complete deterministic profile site-table digest;
- language, public Native ABI, Runtime ABI, KIR, proof, cost-model, target
  profile, LLVM bridge, and cache schema identities;
- target triple, pointer width, endianness, object format, and OS ABI;
- overflow, bounds, strict-float, sanitizer, consumer, and profile-topology
  class (`native-executable` or `native-library`);
- optimization family (`o2` or `o3`) where it changes legal consumers, while
  the fixed generation topology is represented independently;
- CPU policy and the complete ordered multiversion target-set digest; and
- every fixed PGO confidence, profitability, code-growth, and resource limit.

The identity excludes absolute paths, source formatting, comments, timestamps,
process IDs, host names, shard names, environment variables, machine load, and
the physical dynamic/static/object artifact kind. The physical kind remains in
the final artifact transaction and Native object/cache identity, but it cannot
split profiles whose canonical Native-library KIR and site topology are equal.
Formatting-only and comment-only source changes can reuse a profile when they
produce byte-identical canonical semantic/KIR/site identities. Any semantic,
module-graph, site, compiler-contract, ABI, target-set, safety, or schema change
invalidates it.

Merge and use compare every field and report the first stable field path plus
the expected and observed digest. There is no `ignore mismatch` flag. Profiles
from baseline and multiversion target sets, different targets, different safety
modes, or a changed compiler contract cannot be combined even if their counters
look similar.

## `.ckprof` and shard format

The final file magic is `CKPROF01`; a completed shard uses `CKPART01`. Both use
canonical big-endian integers, fixed numeric tags, length-prefixed UTF-8,
lexicographically sorted records, and SHA-256 over a domain tag plus all
preceding canonical bytes. Unknown tags, duplicate fields, trailing bytes, a
bad digest, or non-canonical ordering are errors.

A final profile contains:

1. the complete `CkProfileIdentity`;
2. the canonical site descriptor table;
3. saturated aggregate counter records;
4. completed-run and merged-shard counts;
5. overflow and incomplete-observation flags; and
6. the final content digest.

A site ID is the first 128 bits of SHA-256 over the canonical function identity,
KIR location, site kind, and kind-specific descriptor. The full descriptor and
site-table digest remain authoritative. Two different descriptors with the
same 128-bit ID are a hard collision error, never an instruction to merge.

The closed site kinds for 0.13 are:

- function entry;
- selected CFG edge, with uninstrumented edge counts reconstructed only from a
  verified spanning-tree equation;
- loop trip-count histogram;
- slice-length histogram at a selected call, loop, or versioning decision; and
- candidate-constant hit/miss at an existing comparison or direct call.

There is no arbitrary value, memory-address, string, byte-array, file-content,
or indirect-call-target record. Candidate-constant records refer to a bounded
ordinal in the canonical site table; they do not copy an arbitrary runtime
integer into the profile.

Counters are saturated `u64`. Length and trip values are `u32` and use exactly
16 buckets: `0`, `1`, `2`, `3..4`, `5..8`, `9..16`, `17..32`, `33..64`,
`65..128`, `129..256`, `257..512`, `513..1024`, `1025..2048`,
`2049..4096`, `4097..65536`, and `65537..u32::MAX`. A constant site has at most
eight candidates plus one `other` counter.

Resource limits are part of schema 1: at most 1,048,576 sites per module, 4,096
input shards per merge, 16 buckets per histogram, 8 candidate constants per
site, and 512 MiB per input or final profile. All size arithmetic is checked
before allocation.

A loop observation greater than `u32::MAX` is placed in the final bucket and
sets a trip-saturation flag. It is not truncated into a smaller bucket.

A saturated counter or histogram is never used to establish confidence or
profitability. The affected site is `unknown`; if a saturated value participates
in a spanning-tree equation, every reconstructed edge depending on it is also
`unknown`. An unsaturated equation that does not reconstruct to one
non-negative, internally consistent count is a malformed shard/profile. These
rules are checked again after merge and before profile application.

## Instrumentation and collection runtime

The canonical profile topology is frozen before instrumentation. Function
entries, a deterministic minimal CFG edge set, loop exits, selected slice
lengths, and pre-existing constant candidates receive explicit profile
operations. Critical edges are split deterministically before IDs are assigned.
The profile topology does not depend on hash-map order, wall time, target load,
or an LLVM optimization decision.

The embedded generation runtime uses process-local lock-free 64-bit counters
where the target guarantees them. Updates are relaxed and detect wrap; a wrap
sets an overflow bit and the serialized value becomes saturated. A target
without the required atomic primitive rejects generation rather than emitting
racy instrumentation. Counter storage is private, non-exported, and separate
from CK-visible memory.

The compiler-private initialization guard is emitted as a non-inlinable helper.
Instrumented function entries may call that compact helper, but LLVM must not
expand the directory/identity/runtime initialization argument setup into each
inlined function or loop site. This keeps one-time initialization semantics
without multiplying cold initialization machinery through hot instrumented
paths.

An instrumented executable's compiler-owned entry wrapper writes one shard
after `main()` returns normally and before the process returns to the OS. The
automatic workflow accepts that shard only when the child itself exits zero.
An abnormal termination may leave a temporary file but cannot damage a
completed shard, and automatic mode fails on the abnormal child.

An instrumented static or dynamic library instead adds one
instrumentation-only C control entry named
`ck_profile_flush_<full-profile-identity-hex>() -> i32` to its
generated header. `full-profile-identity-hex` is the 64 lowercase hexadecimal
characters of SHA-256 over the canonical serialized `CkProfileIdentity`, not
the serialized identity text itself. The entry is also added, where applicable,
to its temporary export/import table. The host must quiesce
calls into that library and invoke the entry at its host-defined shutdown
boundary, before unloading a dynamic library or discarding the final linked
static-library state.
The first call snapshots the counters and writes exactly one shard; later calls
are idempotent and return the same success or failure status. Library unload
hooks and `DllMain` perform no profile I/O, so a write failure is synchronously
observable by the host. Return zero means a validated completed shard was
published; a stable nonzero instrumentation status means no completed shard was
published by that library instance. The control entry, its symbol, and its
runtime are
absent from every ordinary or profile-use artifact and are outside public
Native ABI versioning.

Concurrent flush calls after host quiescence serialize through a private atomic
state machine; exactly one call performs publication and every caller observes
the same terminal status. Calling flush while any thread can still enter or
execute CK code violates the temporary instrumentation API precondition and is
diagnosed where the host test seam can observe it; it is never made data-race
safe by copying counters concurrently.

Shard publication uses a unique temporary file in the selected directory,
flushes and validates the bytes, then atomically renames it to a completed
`.ckprof-part` name without overwriting an existing entry. Concurrent processes
never update the same file. Directory merge ignores recognized temporary names
and reports their count.

The runtime never sends telemetry or opens a network connection. A profile can
still reveal aggregate control-flow behavior and must be treated as a build
artifact with access appropriate to the workload. No raw workload data is
intentionally recorded.

## Merge, inspection, and weighting semantics

`ckc pgo merge` accepts completed `.ckprof-part` shards only. A final `.ckprof`
is a terminal aggregate and is rejected as merge input in schema 1. The command
scans explicit shard files and one level of explicit directories, sorts inputs
by canonical content identity, validates all bytes and identities, rejects
symlinks and duplicate run IDs, then performs saturating addition. A directory
is not followed recursively. A duplicate input is an error rather than an
accidental double weight. Reweighting requires retaining and selecting the raw
shards again; nested or overlapping aggregate profiles cannot silently count a
run twice.

Counts are summed exactly. There is no hidden per-run normalization or inferred
importance: a workload executed ten times contributes ten executions. Users
represent a workload mixture by the inputs and repetition counts they choose.
The merged output excludes shard UUIDs, file names, times, and input order; the
same validated shard set produces byte-identical `.ckprof` bytes.

`inspect` uses the same untrusted-input parser and reports identity, site
coverage, run/shard counts, saturated sites, histograms, hotness summaries, and
compatibility with the current compiler. JSON has a versioned deterministic
schema. Inspection never makes an incompatible profile usable.

## Confidence and hotness model

Profile counts are exact observations of the recorded runs but not universal
truth. Schema 1 uses fixed integer confidence rules:

- a decision site requires at least 128 observations before it can guide code
  duplication, cold marking, or a strong LLVM likelihood;
- a branch or constant is dominant at 90 percent or more observations;
- one trip/length bucket is dominant at 85 percent or more observations;
- a block is cold only when its function has at least 128 entries and the block
  executes no more than 1 percent of those entries; and
- zero observations never prove unreachable behavior or authorize deletion.

Estimated dynamic work uses saturated `u128` arithmetic over entry/edge/loop
counts and the immutable target-profile static cost units. PGO-hot roots are
selected in descending work, then stable function identity order, until they
cover 90 percent of estimated module work. A selected root must itself account
for at least 1 percent of module work unless it is the only eligible root.

Sites below confidence retain the ordinary static optimizer decision. Changing
any threshold advances the profile-contract and cache identities.

Every profile-weighted proposal exposes a closed set of observed outcome
classes and immutable integer target-cost formulas. `N` is the checked sum of
all class counts. For an exact branch/value class, the checker computes the
integer cost difference between the unchanged baseline and the guarded selected
path, including the full generic fallback on misses. For a histogram bucket,
the checker must prove a signed lower bound for
`baseline_cost(v) - selected_cost(v)` over every `v` in the bucket using the
closed target formulas; if it cannot prove the bound, that bucket contributes
no PGO authority and the proposal falls back to the static decision. No sampled
or representative value is invented.

The conservative net benefit is the checked signed-magnitude sum of
`class_count*lower_bound_difference`, minus `N*guard_cost`. A proposal must
remain profitable under that lower bound and every existing static/growth gate.
All magnitudes use checked `u128`; overflow, an indeterminate sign, a tie, or
fractional ambiguity chooses the baseline. Ratios use checked cross
multiplication. The independent checker recomputes every class bound and total
from profile records and target formulas rather than trusting proposal totals.
If a saturated site contributes to a function's dynamic-work estimate, that
function is not eligible as a PGO-hot root.

## PGO-guided optimizations

At O2, validated counts may affect only non-duplicating decisions:

- late machine-block order without duplicating bodies or changing the semantic
  machine CFG;
- function and hot/cold section order; and
- required terminator inversion/fallthrough repair, target branch relaxation,
  and alignment padding caused by the accepted order.

The O2 phase boundary is closed. Profile-on and profile-off builds lower the
same semantic and structural KIR; O2 profile analysis remains an unlowered
sidecar until the late boundary. Both modes run the complete default O2 LLVM IR
pipeline plus every ordinary IR preparation, instruction-selection, scheduling,
outlining, splitting, merging, tail-duplication, and other machine-structure
pass profile-blind. No
profile summary, entry count, branch weight, hot/cold attribute, CFG successor
order, or other profile-derived LLVM input is present before that boundary.

The bridge snapshots and verifies the resulting machine CFG, block bodies, and
symbol map, then applies one CK-owned `CkLateProfileLayout` plan. That pass may
only permute existing machine blocks/functions/sections and repair terminators
or fallthroughs required by the permutation. It cannot duplicate or delete a
body, change a non-terminator instruction, outline, split, merge, reschedule, or
alter a call target. After it, only target-mandated branch relaxation, offset/
fixup assignment, alignment padding, and object emission run; none receives
profile data. Any unmapped block stays in its ordinary order. A verifier
independently compares the pre/post snapshots and rejects a plan outside this
closed delta. This structural boundary, not LLVM pass names, defines O2
permission.

Each target owns a closed post-layout repair allowlist. If CFI, unwind, LOH,
security, bundle, or other target state would require a repair outside that
allowlist, the layout proposal is rejected and ordinary order is retained.
AArch64 reruns its required branch relaxation after an accepted reorder. This
conservative target fallback is a normal explanation, not permission to extend
the allowlist implicitly.

At O3, the same data can additionally affect existing verified transformations:

- choose unroll and vector/interleave factors from the observed trip histogram;
- adjust the existing bounded direct-call inliner cost, preferring hot callees
  and refusing cold size growth;
- raise or lower the runtime vector break-even threshold without removing the
  scalar fallback;
- select a short-slice guarded path when one length bucket is dominant;
- create at most one guarded PGO candidate per value site when an existing
  source/KIR constant is dominant;
- rank existing fact-driven specialization, Loop SIMD, SLP, and versioning
  candidates by estimated dynamic rather than purely local static benefit; and
- decide which eligible roots justify CPU variants.

A PGO specialization reuses the 0.12 specialization transaction, proof checker,
maximum three distinct clones per original, and aggregate code-growth budget.
It adds a runtime equality/range/length guard and calls the unchanged generic
body when the observation does not match. Recursive SCCs, exported-body cloning
outside a dispatcher, ordered effects, possible checked first failure,
sanitizer mode, and unsupported consumers remain conservative exclusions.

Profile weights cannot enable fast math, contraction, reassociation, speculative
memory access, widened footprint, unchecked pointer arithmetic, effect motion,
or guard elimination. Static CK proof remains the only authority for those
operations.

## PGO pipeline order

The O3 profile-use pipeline runs:

1. construct and validate the target set and canonical pre-profile KIR;
2. recreate the complete site table and validate the profile identity;
3. attach immutable non-proof profile analysis and compute confidence/work;
4. run the existing O1 prefix and profile-weighted direct-call specialization;
5. run the existing O2 inlining, Memory SSA, GVN, forwarding, DSE, and cleanup,
   using profile-aware but bounded inlining costs;
6. canonicalize loops and rebuild dominance, effect, range, Memory SSA, and
   dependence analyses;
7. propose PGO length/value fast paths and the existing unroll/SLP/Loop SIMD
   alternatives from one immutable scalar pre-state;
8. independently verify every proposal and transactionally select the lowest
   estimated dynamic cost under existing safety and new growth budgets;
9. freeze the verified baseline module and propose target variants from that
   same logical pre-state, never from another variant;
10. lower each accepted module separately, attach checked frequency metadata,
    run the matching LLVM pipeline, audit features, and assemble dispatch; and
11. run final structural, proof, profile-mapping, symbol, artifact, cache, and
    determinism validation before committing outputs.

Any CFG-changing step invalidates profile mappings derived from the old graph.
Counts are transferred only through a closed, independently checked mapping
record; otherwise the affected site becomes unknown rather than guessed.

## Multiversion target sets

`KirMultiversionTargetSet` schema 1 contains a baseline target profile and an
ordered, closed list of enhanced feature profiles. Every entry records target
triple, CPU/feature string, data layout, KIR operation/cost profile, LLVM/bridge
identity, runtime detection predicate, and SHA-256 digest. Each variant uses the
same public ABI and source safety modes.

The initial target table is deliberately feature-level rather than a guessed
microarchitecture database:

- x86-64 Linux, Darwin, and Windows: ABI baseline, `x86-64-v3`, and
  `x86-64-v4`; v3 requires the complete v3 feature set and OS AVX/YMM state,
  while v4 additionally requires the complete v4 AVX-512 set and OS
  opmask/ZMM state;
- AArch64 Linux: ABI Armv8-A Advanced-SIMD baseline, SVE, and SVE2; SVE2 implies
  SVE, and the OS must advertise usable SVE state; and
- AArch64 Darwin and Windows: ABI baseline only in schema 1 because 0.13 does
  not own a reviewed portable SVE feature/state query for those OS ABIs.

SVE and SVE2 profiles still expose only the fixed-width vector KIR operations
defined by 0.12/0.13. LLVM may legally lower those internal fixed operations
with SVE instructions, but 0.13 adds no scalable KIR value or public ABI.

Baseline-only target sets are valid and produce a stable
`no-compatible-enhanced-tier` explanation. `--cpu native` remains the way to
request one exact local Apple/other CPU model. The x86 level feature lists and
AArch64 HWCAP mappings are compiler-owned canonical tables pinned with LLVM
22.1.8; a table change advances the target-set schema.

An enhanced feature level is not automatically faster. The compiler builds its
target cost profile, proposes only legal transformations, and ranks variants
per root by predicted cost. A root may prefer v3 over v4 or baseline over both.
Runtime dispatch follows that per-root order rather than the numeric feature
level.

## Multiversion eligibility and budgets

An eligible multiversion root is an exported CK function or executable entry
whose reachable optimized body is non-recursive, Native-supported, and has at
least one target-dependent plan predicted to reduce execution cost by both 10
percent and two absolute target cost units. Hidden direct helpers may be cloned
inside that root's variant or inlined, but are not independently exported.

With a valid profile, only PGO-hot eligible roots are proposed. Without PGO,
`--cpu multiversion` uses the ordinary static target costs and stable root order.
In either case:

- one root has exactly one baseline and zero to two accepted enhanced variants;
- every enhanced variant starts from the same verified logical KIR pre-state;
- each variant has its own target-profile digest, proof roots, costs, code size,
  and feature audit;
- total additional multiversion KIR units cannot exceed the complete post-O3
  baseline module units, so final module KIR is at most twice baseline;
- PGO specialization still shares, rather than resets, all 0.12 clone and
  transaction budgets; and
- budget exhaustion or insufficient benefit keeps baseline and records a
  stable conservative reason.

Candidate ordering is total: estimated dynamic cost, smaller code size, fewer
required features, target-tier identity, then root/function identity. Rejected
trials do not refund audit budgets.

## Runtime dispatch and public ABI

For an accepted exported root, the original public symbol names a small
baseline-safe dispatcher thunk. The thunk preserves the exact platform C ABI,
checked status/result-slot behavior, slice flattening, alignment, unwind policy,
and symbol visibility. Each implementation symbol contains a content digest
and is hidden from headers, export tables, and ordinary symbol lookup.

The first call obtains one process-local normalized capability bitset, ranks
that root's variants, and publishes the chosen function pointer with
acquire/release atomics. Concurrent first calls may compute the same answer but
only publish a compatible verified pointer. Later calls perform one atomic load
and indirect tail call; they do not repeat CPUID/HWCAP queries. The public
function address remains the thunk before and after resolution.

x86-64 detection uses compiler-owned CPUID and XGETBV checks and requires both
hardware bits and OS register-state support. AArch64 Linux uses the initial
auxiliary-vector HWCAP/HWCAP2 state captured without parsing mutable text.
Unsupported OS/architecture pairs expose baseline only. Query failure,
heterogeneous uncertainty, malformed state, or an unknown future feature bit
selects baseline.

Production artifacts provide no environment variable or public API that can
force unsupported features. Tests may link a private detector seam into test
fixtures only. Static archives namespace private support symbols by target-set
digest; dynamic libraries and executables hide them. Resolver and thunk code is
compiled for baseline and is audited to contain no optional instructions.

## Native LLVM and artifact contracts

Baseline, each enhanced variant, and dispatch support lower into separate LLVM
modules with exact target attributes. Cross-variant LTO is disabled in 0.13 so
an optional instruction cannot leak into the baseline or dispatcher. The
artifact assembler links the audited modules only after each passes LLVM
verification and feature containment disassembly.

At O2 CK converts validated counts only into the private late-layout plan
defined above; LLVM receives no profile-derived metadata or attributes. At O3,
after checking the exact KIR-to-LLVM block/function map, CK may attach LLVM
branch weights, entry counts, hot/cold attributes, and internal profile
summaries for inlining, vectorization cleanup, scheduling, instruction
selection, and other O3 transforms. Neither mode gives LLVM authority to weaken
CK alias, bounds, failure, or floating semantics.

The Native bridge gains target machines for explicit feature levels, normalized
runtime-predicate descriptions, the verified O2 late-layout boundary, O3
profile metadata attachment, and per-module feature audits. Every queried cost
and operation remains subject to the 0.12 closed target-profile validation
rules.

Final executable, dynamic, static, and supported single-version object
artifacts remain self-contained under the existing system-runtime policy.
Multiversion final artifacts are executable, dynamic, or static; the rejected
single-object combination is not silently repackaged. A profile-use or ordinary
artifact must not import a CK profile writer, LLVM profile runtime, compiler
library, or new non-system shared library. Instrumented artifacts contain only
the private CK collection runtime required for their temporary purpose.

## Compatibility, schemas, and cache identity

Public Native C ABI remains version 1 and Runtime ABI remains version 2 because
profile and dispatch helpers are private compiler support, not callable CK
runtime API. KIR advances to schema 3 for profile operations, immutable profile
annotations, multiversion bundles, and dispatch plans. The target profile,
proof, optimization audit, private profile-runtime, and private dispatch-runtime
schemas advance explicitly during implementation. The private LLVM bridge
advances from ABI 3 to ABI 4.

The Native cache advances from `CKCOBJ02`, key schema 3, and manifest schema 3
to `CKCOBJ03`, key schema 4, and manifest schema 4. In addition to every 0.12
field, the key and manifest cover:

- profile mode, profile format/contract identity, and exact `.ckprof` digest;
- physical output artifact kind and its validated profile-topology compatibility;
- all confidence, hotness, weighting, site, and PGO cost constants;
- target-set and per-variant profile/proof/codegen digests;
- dispatch table, detector, thunk, and private runtime identities; and
- every multiversion profitability and code-growth budget.

Generate-mode artifacts are not cached. Profile-use variants may be cached
individually, but a bundle hit is accepted only when the dispatcher manifest and
every referenced variant object validate. A missing, extra, reordered,
redirected, or mismatched variant rejects the complete hit.

Given identical canonical source, compiler/toolchain, flags, target set, and
`.ckprof` bytes, the final KIR, explanations, variant ordering, objects, and
artifact bytes must be reproducible under the existing platform-signing
boundary. Profile timestamps, shard order, local paths, CPU of the build host,
and map iteration cannot affect a profile-use artifact.

## Errors, security, and privacy

Profile files and directories are untrusted input. The parser validates magic,
version, lengths, counts, canonical order, duplicate identities, integer
arithmetic, resource limits, and digest before exposing a record to the
optimizer. It never follows a symlink during merge or transactional output and
never allocates from an unchecked length.

Stable diagnostic categories distinguish invalid CLI combinations, generation
runtime failure, malformed shard/profile, identity mismatch, unsupported target
set, insufficient profile observations, invalid profile-to-KIR mapping,
detector construction failure, variant verification failure, and artifact
feature leakage. Malformed identity, mapping, proof, or feature containment is a
compiler error with no output or cache entry. Low confidence and insufficient
profitability are normal baseline fallbacks with explanations.

No command uploads profiles, source, counters, or diagnostics. The format omits
raw workload values and local paths, but aggregate control flow can itself be
sensitive. Documentation must tell users to protect `.ckprof` like benchmark or
build data and not publish it unintentionally.

## Inspection and explanations

`--explain-optimization` extends its deterministic report with:

- profile identity/digest, coverage, confidence, and ignored-site reasons;
- function dynamic-work rank and selected hot roots;
- branch/layout/inline decisions and exact supporting counter IDs;
- PGO value/length candidate, guard, fallback, cost, proof, and rejection;
- multiversion target set, considered tiers, required features, predicted cost,
  code growth, accepted order, and dispatcher identity; and
- cache use, profile mapping transfers, budget exhaustion, and conservative
  fallback reasons.

Native `emit-kir --cpu multiversion` prints the target-set identity, verified
baseline, accepted variant KIR modules, dispatch plan, and hidden symbol map. It
does not resolve the current host or suppress a variant merely because the
inspection machine lacks its features.

## Verification strategy

Acceptance includes all of the following without ignored tests or weakened
thresholds:

1. Every 0.12 language, ABI, optimizer, runtime, artifact, sanitizer,
   differential, mutation, performance, and six-host contract remains green.
2. Golden and mutation tests cover canonical profile bytes, site stability,
   comment/format reuse, every identity mismatch, hash collision handling,
   malformed lengths/tags/order/digests, duplicate shards,
   final-as-merge-input rejection, counter/equation saturation, resource limits,
   and deterministic merge/JSON inspection.
3. Instrumentation tests prove exact function/edge/loop/length/constant counts
   for normal executable exit, early return, break/continue, checked failure,
   recursion, multi-threaded host calls, multiple processes, host-quiesced
   library flush, concurrent/repeat-flush idempotence, unload-without-I/O,
   write failure propagation, and abnormal termination.
4. Differential tests compare ordinary O0, ordinary O3, generate execution,
   PGO O2/O3, baseline, each forced test-only compatible variant, and production
   dispatch over training, held-out, and adversarial non-training inputs.
5. Mutation tests prove profile data alone cannot remove checks, widen memory
   footprints, change first-error precedence, reorder print/effects, enable fast
   math, forge a KIR mapping, exceed a code budget, or select an unsupported
   feature variant.
6. Artifact-matrix, object, and disassembly audits prove rejected generation
   and multiversion-object combinations fail before output, baseline/thunks
   contain no optional ISA, each variant contains only its declared features,
   variant/runtime symbols are hidden, final use artifacts contain no profile
   runtime, and ordinary/profile-use public ABI/header bytes remain stable. The
   generation-only flush declaration must be present only in its temporary
   instrumentation header. A Native-library profile generated through dynamic
   and static packaging must validate for baseline/native object use, while an
   executable-topology profile must fail that use.
7. Runtime dispatch tests cover concurrent first calls, stable public addresses,
   exactly-once capability caching, ordered per-root selection, query failure,
   baseline-only targets, and real hardware selection where available.
8. Reproducibility tests build and merge in different directory, shard, map, and
   process orders and require byte-identical final profiles and unsigned
   artifacts.
9. O2 phase-boundary mutation tests inject profile-favored inline/vector/CFG/
   tail-duplication opportunities and prove profile-on/off snapshots are
   identical immediately before `CkLateProfileLayout`. MIR/object/disassembly
   audits then prove the accepted delta contains only ordering, required
   terminator/fallthrough repair, branch relaxation, and alignment padding;
   profile-derived LLVM metadata is absent.
10. The exact final candidate SHA passes quality, Native integration, all six
   Native hosts, and fixed x86-64/AArch64 performance acceptance.

## Performance, size, and compile-time contract

The benchmark report advances to a new versioned schema and pins the exact
0.13 candidate, 0.12 replay compiler, LLVM/Clang 22.1.8, Rust 1.90.0, sources,
training shards, final profile, target sets, variant objects, sampling order,
hardware identity, and every digest. Training inputs and held-out measurement
inputs are fixed separately. Correctness includes both plus adversarial inputs;
timed PGO results use only held-out inputs. A changed workload, exclusion,
threshold, or rerun policy is a reviewed contract change.

On stable x86-64 and AArch64 workers:

- ordinary no-PGO baseline/native throughput regresses no more than 2 percent
  in geometric mean and 5 percent for any case against exact 0.12 replay;
- on the declared PGO-sensitive suite, PGO-use improves geometric-mean
  throughput by at least 5 percent over the same 0.13 no-PGO CPU policy, with no
  held-out case more than 3 percent slower;
- on the feature-eligible multiversion suite, the dispatched artifact improves
  geometric-mean throughput by at least 8 percent over portable baseline on a
  worker with the required enhanced tier, with no case more than 3 percent
  slower;
- after resolution, dispatched throughput is at least 98 percent of a separately
  linked direct artifact using the same selected tier in geometric mean and no
  case is more than 5 percent slower;
- combined PGO plus multiversion is no more than 2 percent slower in geometric
  mean and 5 percent for any case than the faster applicable PGO-only or
  multiversion-only artifact;
- against equivalent pinned Clang-PGO and Rust-PGO oracles using the same
  training/evaluation split and safety/float preconditions, CK reaches at least
  95 percent of oracle geometric-mean throughput and 90 percent for every
  accepted kernel, retaining the cumulative 0.12 hand-SIMD/domain gates;
- generation execution is no more than 5 times the ordinary artifact on the
  fixed instrumentation corpus; this is a tooling bound, not a final-artifact
  runtime allowance;
- PGO-use single-version source-to-object compilation is at most 1.5 times,
  multiversion at most 2.5 times, and combined at most 3.5 times the matching
  ordinary 0.13 baseline geometric mean, with individual limits of 2, 3, and 4;
  samples use terminated-child user-plus-system CPU time so hosted-worker
  descheduling is excluded without removing compiler work;
- PGO-only artifacts are at most 1.25 times the ordinary aggregate size and 1.5
  times any individual; multiversion or combined artifacts are at most 2 times
  aggregate and 2.5 times any individual; and
- the distributed `ckc` archive is at most 15 percent larger than exact 0.12 on
  each host after equivalent stripping/signing boundaries.

On the authoritative Linux performance workers, inherited schema-7 runtime
samples use the current thread's `CLOCK_THREAD_CPUTIME_ID` delta inside the
same-core affinity scope. This retains all CPU time consumed by the unchanged
kernel-call loop while excluding intervals when the shared host does not
schedule it. Non-Linux development runs keep monotonic wall-clock timing.

All timed channels retain the fixed warm-up, rotating-order, upper-median,
stability, equivalence, and fail-fast rules. CPU detection, dynamic loading, and
symbol resolution occur before steady-state timed calls, but a separate untimed
record proves the resolver ran only once. Failed stability is invalid evidence,
not permission to rerun until a favorable sample appears.

## CI and release acceptance

CK 0.13 does not add another full pinned-LLVM bootstrap matrix. It extends the
existing ten-job contract:

- quality owns format/schema/unit/mutation/document/cache checks that need no
  Native toolchain;
- Native integration owns real instrumentation, merge/use, final artifact, and
  profile-runtime audits;
- Darwin ARM64/x64, Linux ARM64/x64, and Windows ARM64/x64 own host correctness,
  ABI, baseline fallback, object format, and supported real detector checks; and
- stable x86-64 and AArch64 performance workers own fixed PGO/multiversion
  training, held-out performance, feature containment, size, and compile-time
  evidence.

All jobs reuse the existing verified LLVM/Clang manifest and cache. A required
feature worker publishes an exact capability manifest before measurement; a
missing required tier fails rather than silently skipping the gate. Final
acceptance is tied to one exact candidate SHA and downloadable, hashed evidence.

## Completion and future boundary

CK 0.13 is complete only when the exact candidate SHA satisfies every semantic,
profile, security, structural, dispatch, ABI, artifact, cache, reproducibility,
performance, size, compile-time, and six-host gate above, and the current
English/Chinese specifications agree with the implementation.

The following remain outside 0.13: source function pointers and indirect-call
PGO, source SIMD/intrinsics, fast math, floating reassociation, public scalable
vector KIR/ABI, GPU targets, cross-compilation, runtime adaptive optimization,
public JIT APIs, profile-server/telemetry features, arbitrary workload/value
recording, and search-based Auto-Tuning. Bounded reproducible offline
Auto-Tuning remains 0.14.
