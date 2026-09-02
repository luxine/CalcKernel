# CK 0.14 Offline Auto-Tuning Specification

[简体中文](zh-CN/offline-autotuning.md)

Status: Proposed design for CK 0.14.0

Base revision: v0.13 candidate 94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05

This document is normative for the CK 0.14 implementation. It defines a bounded,
reproducible, cached, ahead-of-time auto-tuning system. It does not claim that the
implementation or release acceptance has completed.

The base revision is provisional until v0.13 passes its remote acceptance gates.
Before implementation begins, this design branch must be rebased onto, or reviewed
against, the final accepted v0.13 revision. Any semantic difference must be resolved
explicitly; it must not be hidden by adapting tests.

## 1. Objective

CK 0.14 shall let a user explicitly compile a native executable or dynamic library
against a representative workload and select a faster legal optimization plan using
real measurements.

The tuner shall:

- use CK static analyses to construct a small set of legal, high-value alternatives;
- measure those alternatives with a user-supplied, repeatable workload harness;
- validate the selected plan on distinct validation cases;
- record all inputs, decisions, measurements, and rejection reasons in a portable
  decision file;
- replay the exact selected plan during a later build without rerunning the harness;
- preserve CK safety semantics, deterministic compilation, and the existing
  self-contained system-runtime policy of the final artifact.

The performance target is not “LLVM happened to optimize it.” CK owns the facts,
decision space, legality checks, measurement policy, and reproducible replay. LLVM
remains the native code generator.

## 2. Product decision

CK 0.14 adopts bounded offline two-stage auto-tuning:

1. Static analysis and the CK cost model rank a deterministic legal frontier.
2. User-provided search workloads measure a bounded set of finalists.
3. Distinct validation workloads validate the leading plans twice.
4. A measured-profitability certificate authorizes one exact optimization plan.
5. A normal ahead-of-time build may replay that plan from a decision file.

This is an explicit opt-in workflow. Normal check, run, build, and release commands
must not start tuning, execute a harness, or consume a tuning decision implicitly.

### 2.1 Alternatives not chosen

- Static-only selection is retained for ordinary builds but cannot be called
  auto-tuning because it never observes the user's workload.
- Exhaustive or random empirical search is rejected because it is not predictably
  bounded or reproducible.
- Bayesian and learned online search are deferred because schema 1 requires a closed
  auditable algorithm with no training state.
- LLVM pass-pipeline or backend-flag search is rejected because CK could not
  independently describe and validate the semantic decision space.
- Adaptive JIT tuning is rejected because it adds warmup, runtime machinery,
  nondeterminism, and a deployment contract different from ahead-of-time artifacts.
- Implicit profile generation and workload execution are rejected because building
  source must not unexpectedly run user programs.

## 3. Scope

### 3.1 Included

- Host-native ahead-of-time compilation.
- Optimization level O3.
- An exact native CPU and feature set selected by cpu=native.
- Executable and dynamic-library outputs.
- Existing CK optimization decisions that can be expressed as finite alternatives.
- Optional use of an already collected CK profile as a ranking and code-generation
  input.
- Deterministic candidate generation, measurement scheduling, selection, caching,
  inspection, and replay.
- Stable text and JSON inspection output.

### 3.2 Excluded

- Adaptive JIT compilation and ORC runtime tuning.
- Implicit tuning during ckc run or an ordinary ckc build.
- Workload synthesis, fuzz-generated workloads, or using a profile to invent cases.
- Static-library and object-file tuning.
- Portable-baseline or cross-compilation tuning.
- One tuning decision covering multiple CPU variants or fleet machines.
- Distributed or remote tuning services.
- Arbitrary LLVM flags, pass pipelines, or plugin search.
- Indirect-call promotion, scalable KIR, source-level SIMD, relaxed floating point,
  GPU offload, and new source-language syntax.
- Changes to public ABI, native ABI, runtime ABI, or runtime dependencies.

These exclusions are deliberate. They remain candidates for later versions and
must not be smuggled into CK 0.14 as incidental implementation details.

## 4. Preconditions and compatibility

Tuning is valid only when all of the following hold:

- the output kind is executable or dynamic;
- the native backend is selected;
- optimization level is O3;
- the target is the host target;
- cpu=native is selected;
- cpu=multiversion is not selected;
- profile generation is disabled; only an explicit profile-use input is accepted;
- contract sanitizer mode is disabled;
- every optional profile and compilation mode is explicit and valid;
- the workload manifest and every declared input pass validation.

The existing safe, strict, checked, unchecked, contract, overflow, floating-point,
PGO, and multiversion semantics remain authoritative. Tuning may choose only among
plans that preserve the exact selected modes. It may not weaken guards or alter
observable failure ordering.

The C and WebAssembly backends are unaffected. The source language and public ABI
are unchanged.

Tune build never performs profile generation or “training” on the user's behalf.
When pgo-use is present, the named profile is an immutable input. Collecting a new
profile after source changes remains a separate, explicit workflow.

## 5. Terminology

- Baseline: the exact ordinary v0.13-style O3 native artifact for the same source,
  modes, target, output kind, and optional profile, with no tuning override.
- Decision site: a stable compiler location at which CK owns a finite set of legal
  optimization alternatives.
- Tuning unit: a deterministic cluster of decision sites that share a loop root,
  helper, specialization boundary, or code-size interaction.
- Plan: the canonical set of non-baseline choices for all tuning units.
- Trial artifact: an ephemeral, non-publishable artifact used only for measurement.
- Search case: a manifest workload used to compare the baseline and finalists.
- Validation case: a distinct manifest workload used only after search ranking.
- Decision file: the binary .cktune record containing identity, measurements, and
  either a selected plan or a reason for selecting the baseline.
- Measured-profitability certificate: evidence that an exact legal plan passed the
  fixed validation thresholds.

## 6. Command-line contract

### 6.1 Tune and build

The primary command is:

    ckc tune build <file> --config <workload.cktune.toml>
      --out <artifact> --kind <executable|dynamic>
      --cpu native -O3
      [--target <host-triple>]
      [--pgo-use <profile.ckprof>]
      [existing semantic and code-generation modes]
      [--budget <quick|standard|thorough>]
      [--tune-out <decision.cktune>]
      [--no-tune-cache]

The default budget is standard. If tune-out is omitted, the decision path is
<artifact>.cktune. If target is present it must normalize to the exact host triple;
omission selects that same host triple.

The artifact and decision paths must be distinct after canonical destination
resolution and must not alias any source, manifest, runner, profile, or declared
input. CK stages and verifies both files, records their digests in a durable
publication journal, publishes the decision first, and publishes the artifact last.
Each replacement is individually atomic.

On any reported failure, CK rolls both destinations back to their prior state. An
unexpected process or machine failure can leave an orphan new decision next to the
old or absent artifact, but never a new artifact for which the complete decision
was not already durable. The journal is recovered under an exclusive destination
lock before a later CK command touches either path. Pair consumers verify the final
artifact digest stored by the decision. This journaled artifact-last protocol is
the meaning of transactional pair publication in this specification; simultaneous
atomic visibility of two arbitrary filesystem paths is not claimed.

If all required baseline and surviving-candidate measurements are valid and stable
but no candidate satisfies the fixed benefit thresholds, the command succeeds with
the baseline artifact and a baseline-selection decision file. Candidates already
removed by the canonical timeout rule do not make the remaining streams incomplete.

Configuration, identity, compilation, correctness, runner, protocol, baseline
timeout, process-control, instability, or validation-completion failures are
errors. They produce no new artifact or decision output. The narrowly defined
post-calibration candidate-timeout rule in Section 8 is the sole timeout exception.

### 6.2 Inspect

    ckc tune inspect <decision.cktune> [--json]

Inspection is read-only and does not require the original source, harness, profile,
or cache. It fully validates the bounded file before displaying content.

### 6.3 Replay

    ckc build <file> --out <artifact> --kind <executable|dynamic>
      --cpu native -O3 --tune-use <decision.cktune>
      [the same profile and modes used for tuning]

Explicit tune-use is fail-closed. Any source, compiler, schema, target, native CPU,
feature, profile, mode, output-kind, frontier, or plan mismatch is a hard error.
There is no silent fallback to ordinary optimization.

CK 0.14 does not add tune-use to run or emit-kir. The exact selected plan is
inspected through ckc tune inspect.

### 6.4 Ordinary commands

In the absence of tune build or tune-use:

- no workload process is launched;
- no tuning cache is read or written;
- no tuning decision changes optimizer behavior;
- ordinary optimizer thresholds and decisions remain unchanged, except for
  versioned internal schema and diagnostic maintenance required by CK 0.14.

## 7. Workload manifest

### 7.1 Format

The input is a UTF-8 TOML file named by convention workload.cktune.toml. Schema 1 is
a closed schema: unknown, duplicate, missing, incorrectly typed, or out-of-range
fields are errors.

The manifest declares:

- schema = 1;
- an absolute runner executable after canonical resolution;
- a fixed argv vector, with no shell;
- optional explicitly allowlisted inherited environment variables;
- a list of runner input files to digest;
- one to sixteen weighted cases;
- for each case: a stable identifier, role, seed, weight, and expected digest;
- a per-invocation timeout within the schema bound; I/O limits remain fixed by
  this specification and the total wall clock by the selected budget preset.

There must be at least one search case and at least one validation case. Case
identifiers and seeds must be distinct across the two partitions. Identifiers use
ASCII letters, digits, underscore, dash, and dot and are at most 64 bytes. Weights
are positive u32 values.

Schema 1 has exactly these fields:

| TOML location | Field | Type and rule |
| --- | --- | --- |
| root | schema | Required integer, exactly 1 |
| runner | path | Required UTF-8 path to a regular executable |
| runner | args | Optional array of at most 64 UTF-8 strings; default empty; total encoded bytes at most 64 KiB |
| runner | inputs | Optional array of at most 64 manifest-relative regular-file paths; default empty; each at most 1 GiB and total at most 4 GiB |
| runner | inherit_env | Optional array of at most 16 unique ASCII environment names; default empty |
| runner | timeout_ms | Optional integer from 100 through 120,000; default 30,000 |
| each case entry | id | Required unique identifier with the syntax and length above |
| each case entry | role | Required string, exactly search or validation |
| each case entry | seed | Required u64 integer |
| each case entry | weight | Required positive u32 integer |
| each case entry | expected_digest | Required 64-character lowercase hexadecimal digest |

A canonical example is:

    schema = 1

    [runner]
    path = "./build/tune-harness"
    args = ["--ck-tune"]
    inputs = ["data/search.bin", "data/validation.bin"]
    inherit_env = []
    timeout_ms = 30000

    [[case]]
    id = "search-medium"
    role = "search"
    seed = 101
    weight = 2
    expected_digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

    [[case]]
    id = "validation-medium"
    role = "validation"
    seed = 202
    weight = 2
    expected_digest = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"

The runner working directory is always CK_TUNE_TEMP. The manifest cannot override
it. Resource-output limits are fixed by this specification rather than configurable
manifest fields.

### 7.2 Paths and inputs

Manifest-relative paths resolve under the canonical manifest directory. Parent
traversal, symlink ambiguity, non-regular declared input files, and paths escaping
that directory are rejected. The runner itself may be outside that directory but
must be named by an explicit path; PATH lookup is forbidden.

The canonical bytes of the manifest, runner executable, and every declared input
file are SHA-256 digested. A change invalidates measurement reuse. Absolute paths
and timestamps are diagnostic metadata, not canonical tuning identity.

At session start, CK opens every validated input without following a symlink,
streams it into a private immutable-content snapshot, and verifies its digest. Before
each timed invocation, CK copies that snapshot below CK_TUNE_TEMP/inputs while
preserving the manifest-relative path. Input preparation is outside the measured
interval, and each invocation receives a fresh copy, so one runner invocation cannot
change a later invocation's input. The harness uses CK_TUNE_TEMP plus the documented
inputs subdirectory to locate these files.

### 7.3 Environment

The runner starts from an empty environment. On Windows, CK may provide only the
minimal SystemRoot and WINDIR values needed to create a process. Any other inherited
variable must be explicitly allowlisted. Both its name and exact value enter tuning
identity.

CK sets these protocol variables:

    CK_TUNE_PROTOCOL=1
    CK_TUNE_ARTIFACT=<absolute candidate artifact path>
    CK_TUNE_ARTIFACT_KIND=<executable|dynamic>
    CK_TUNE_CASE=<case identifier>
    CK_TUNE_SEED=<unsigned decimal u64>
    CK_TUNE_ITERATIONS=<unsigned decimal u64>
    CK_TUNE_TEMP=<absolute private per-run directory>

The argv and environment are passed directly to process creation. CK never builds
a shell command string.

### 7.4 Harness responsibility

The harness is tuning-only and is not linked into or required by the final artifact.
It must load or execute CK_TUNE_ARTIFACT, run exactly CK_TUNE_ITERATIONS logical
iterations of the named case, and produce a deterministic correctness digest.

For a dynamic library, the harness owns loading and calling its exported ABI. For
an executable, the harness owns invoking or driving it. The manifest declares the
kind, but CK does not infer an application protocol.

The harness is arbitrary user-authorized code. CK does not claim to provide a
portable filesystem or network sandbox. Users must apply their own operating-system
sandbox if the harness is untrusted.

## 8. Runner protocol and timing

### 8.1 Output

A successful runner exits with status zero and writes exactly one line to stdout:

    CKTUNE/1 <case-id> <seed-u64> <iterations-u64> <completed-u64> <digest>\n

The digest is exactly 64 lowercase hexadecimal characters. completed must equal
iterations, and the echoed case, seed, and iteration count must match the request.
Any extra stdout is a protocol error.

Stdout is limited to 4 KiB. Stderr is captured for diagnostics and limited to
1 MiB. Truncation, invalid UTF-8 protocol data, nonzero exit, signal termination,
or malformed output is an error.

### 8.2 Correctness

For each case and seed, the manifest declares the expected digest. The baseline
must match it before CK accepts calibration. Every candidate invocation, including
warmups, calibration confirmation, search samples, and validation samples, must
also return that digest.

A mismatch is a compiler-correctness failure or an invalid harness, not a slow
candidate. The complete tuning session aborts. It may not be converted into an
ordinary candidate rejection.

### 8.3 Timing and process control

CK measures elapsed time outside the runner with a monotonic high-resolution clock.
It creates a private per-invocation directory, enforces output and timeout limits,
terminates the complete process tree on timeout, and cleans temporary files.

The harness must batch enough work to amortize process startup. CK calibrates the
baseline by doubling a power-of-two iteration count until one invocation lasts at
least 50 ms, targeting no more than 250 ms. Overflow or inability to reach this
window within the preset limit is an error. The resulting iteration count is fixed
for all candidates for that case.

A candidate timeout after successful baseline calibration is a canonical
performance rejection. CK kills and reaps its process tree, records the timeout,
removes that candidate from later rows and rounds, and continues with the remaining
candidates. This counts as completed validation for that rejected candidate. A
baseline timeout, inability to kill and reap the complete tree, crash, protocol
error, or correctness mismatch aborts the session.

## 9. Legal candidate model

### 9.1 CK-owned alternatives

CK 0.14 may tune only finite alternatives already derived from CK facts and owned
by CK:

- direct-call inlining choices;
- function specialization and guarded value or length specialization choices;
- loop unroll factors;
- Loop SIMD vector width, interleave factor, and break-even threshold choices;
- SLP pack choices;
- short-slice and loop-versioning choices;
- CK-owned block, function, and section-layout alternatives.

Each decision site and alternative has a canonical stable identifier, precondition
digest, and ordering. The trace records both accepted and rejected alternatives.

### 9.2 Never tunable

The tuner may not change:

- language or safety semantics;
- bounds, overflow, contract, or other required guards;
- proven facts, alias classes, pointer provenance, ranges, alignment, or effects;
- strict floating-point behavior;
- failure and side-effect ordering;
- source or public ABI;
- target triple, CPU, feature set, or runtime ABI;
- sanitizer mode;
- LLVM pass pipeline or arbitrary backend flags.

Measurement establishes profitability only. It never establishes safety, semantic
equivalence, or target legality.

### 9.3 Tuning units

Overlapping roots, cloned helpers, specialization boundaries, and shared code-size
effects are clustered deterministically into one tuning unit. A session considers
at most 64 units. Units beyond that bound use ordinary optimizer decisions, selected
by canonical rank rather than discovery order.

Each site exposes at most four non-baseline alternatives. A plan and trace contain
at most 4096 explicit choices.

### 9.4 Trial typestate

Candidate materialization separates legality from static profitability:

1. CK recomputes all structural, proof, effect, guard, failure-order, target-feature,
   and growth checks.
2. A legal tuning trial may bypass only the ordinary static-profitability threshold.
3. The resulting trial artifact has a non-publishable typestate.
4. Trial artifacts cannot enter production output or the production object cache.
5. Only a valid measured-profitability certificate may authorize replay of that
   exact plan in a publishable build.
6. The final checker independently recomputes plan legality and the measurement
   threshold before publication.

Ordinary optimizer thresholds do not change. A candidate that fails after CK has
declared its plan legal is a compiler error, not a search rejection.

## 10. Deterministic search

CK uses a deterministic beam search over stable tuning units and canonical
alternative sets.

Candidate ordering is:

1. predicted dynamic cost;
2. predicted static cost;
3. artifact size;
4. number of non-baseline choices;
5. alternative class and canonical order;
6. plan digest.

A deterministic diversity round-robin admits candidates from each available
alternative class before filling remaining beam slots by rank. This prevents the
static model from eliminating every structurally different legal candidate.

The closed presets are:

| Preset | Beam | Plan expansions | Candidate compile attempts | Measured finalists | Validation entrants | Wall-clock limit |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| quick | 4 | 1,024 | 8 | 4 | 2 | 600 s |
| standard | 8 | 4,096 | 16 | 8 | 3 | 1,800 s |
| thorough | 16 | 16,384 | 32 | 16 | 4 | 7,200 s |

The baseline is always present and does not count against candidate compile
attempts. User-supplied numeric inflation of these bounds is not supported in
schema 1.

On a cache miss, the preset wall clock starts after manifest validation and input
snapshotting, immediately before baseline construction. It includes baseline and
candidate compilation, runner setup, search, both validation rounds, final replay,
and transactional staging. Each runner timeout is also capped by the remaining
wall-clock budget. An exact completed-decision cache hit performs no search session.

A candidate artifact may be at most 110% of matching baseline bytes. Existing KIR,
rewrite, specialization, and per-pass growth bounds remain in force. Rejected or
invalid attempts consume their audit budget and are never refunded.

When the wall-clock budget expires, CK stops creating candidates. If the fixed
validation protocol cannot complete for all required entrants, the command fails
without output. It must not select from incomplete evidence.

## 11. Measurement and selection

### 11.1 Search phase

The baseline and measured finalists run on search cases only. For every case:

- calibration fixes the iteration count;
- three warmup rows are executed and not scored;
- twenty measured rows are executed;
- each row evaluates every active channel exactly once in deterministic rotation;
- one channel evaluation consists of exactly three identical runner invocations,
  and its stored sample is their minimum.

A channel is the baseline or one candidate. Case order, channel order, and rotation
derive from the session identity digest. There is no mutable random state.

Per-case time is the upper median of twenty stored samples: element 10 using
zero-based indexing after ascending sort. A stream is stable only if at least 16 of
its 20 samples are within the inclusive interval from 80% through 120% of that
upper median. All percentage comparisons use checked integer cross multiplication.
Instability in any required baseline, candidate, or case stream is an error.

Search ranks candidates with exact integer Q32 normalized time:

    ratio_q32 = ceil(candidate_ns * 2^32 / baseline_ns)
    score_q32 = ceil(sum(weight * ratio_q32) / sum(weight))

No floating-point arithmetic participates in selection. The best bounded entrants
advance to validation. All products and sums use checked u128 arithmetic, and the
persisted Q32 result must fit u64.

### 11.2 Validation phase

Validation cases have identifiers and seeds distinct from all search cases. Each
entrant and the baseline are validated in two independent complete rounds using the
same fixed sample protocol.

In each round, a plan qualifies only if:

- weighted score is at most 0.97 of baseline;
- no validation case is slower than 1.02 of baseline;
- the candidate has a lower weighted paired-row time in at least 16 of 20 rows.

For paired row r, CK computes the weighted Q32 score from row r of every validation
case using the same per-case normalization formula as the aggregate score. “Lower”
means strictly below the baseline Q32 value 2^32. Row indices are synchronized
across validation cases.

Within each round, qualifying plans are ranked and ties are resolved by:

1. lower validation score;
2. smaller artifact;
3. fewer non-baseline choices;
4. lower plan digest.

The selected plan must qualify and rank first in both rounds. If the two rounds
have different first-ranked plans, or no plan qualifies in both, CK records a
successful baseline decision with the canonical reason validation-disagreement or
validation-threshold, respectively.

CK performs no discretionary rerun after observing an unfavorable outcome. Stable
evidence that selects no plan therefore produces a successful baseline decision.

### 11.3 Final correctness rule

All candidates receive the same cases, seeds, iteration counts, scheduling policy,
and correctness checks. Search and validation never change selected language
semantics. The tuner may exploit domain constraints represented by the workload,
but only through an optimization plan whose guards and preconditions remain valid
for every legal program input.

## 12. Decision file

### 12.1 Encoding

The public decision file has:

- magic CK TUNE 01 encoded as the eight bytes CKTUNE01;
- format schema 1;
- contract schema 1;
- measurement schema 1;
- inspection schema 1;
- plan schema 1;
- canonical big-endian lengths and counts;
- canonical field and collection ordering;
- a trailing domain-separated SHA-256 digest.

The outer encoding follows the repository's existing canonical profile framing:

1. bytes 0 through 7 are CKTUNE01;
2. bytes 8 through 11 are the big-endian u32 format schema;
3. each field is a big-endian u16 tag, a big-endian u32 payload length, and exactly
   that many payload bytes;
4. the last 32 bytes are
   SHA-256("CK-TUNING-DECISION\0" followed by every preceding file byte).

Schema 1 requires these top-level tags in increasing order:

| Tag | Payload |
| ---: | --- |
| 1 | Compiler, source, semantic, schema, target, mode, profile, and output identity |
| 2 | Frozen tuning contract and all numeric policy constants |
| 3 | Manifest, runner, allowlisted environment, and declared-input identity |
| 4 | Normalized measurement environment and timer evidence |
| 5 | Decision sites, tuning units, alternatives, and candidate frontier |
| 6 | Baseline and candidate plans, artifacts, rejections, correctness, and raw measurements |
| 7 | Both validation rounds, selection result, and measured-profitability certificate |
| 8 | Replay frontier, pre/post states, object graph, link recipe, and cache-reuse facts |

Nested records use the same increasing u16-tag/u32-length framing. Unsigned scalars
are fixed-width big-endian; booleans are one canonical byte 0 or 1; strings are
length-prefixed valid UTF-8; lists start with a checked big-endian u32 count and
contain canonical ordered records. Optional values use an explicit one-byte
presence discriminator. There is exactly one encoding for every valid decision.
The trailing hash is the canonical decision digest used by replay and cache keys.

The maximum file size is 32 MiB. It contains at most 33 candidates including the
baseline, 16 cases, and 4096 plan choices. Every string, collection, sample matrix,
and diagnostic field has an implementation constant below the total limit.

Unknown, duplicate, truncated, trailing, out-of-order, over-limit, or noncanonical
content is rejected. Parsing allocates only after bounds and overflow checks.

### 12.2 Recorded identity

The file records:

- CK compiler version and source identity;
- Rust toolchain, LLVM, and LLVM bridge identity;
- language, native ABI, runtime ABI, KIR, proof, cost-model, target, and cache schemas;
- source, semantic, pre-tune KIR, contract, and mode digests;
- output kind, exact host target triple, normalized CPU, feature set, and target
  profile;
- optional .ckprof identity and digest, or explicit absence;
- canonical manifest, runner, allowlisted environment, and declared-input digests;
- budget preset, candidate-space digest, and measurement-policy digest;
- OS, kernel, hardware, timer, and topology evidence for the measurement
  environment;
- every candidate plan, object-graph digest, artifact size, rejection reason,
  correctness digest, raw stored sample, and stability result;
- both validation-round decisions;
- the selected plan or the canonical baseline-selection reason;
- the staged final artifact byte digest and physical size;
- cache reuse facts.

Raw workload files, arbitrary runner stdout, secrets, and absolute paths are not
stored in canonical identity. Human diagnostics may show explicitly marked
noncanonical local paths while the tuning command is running.

The measurement-environment tuple is closed: operating-system family and build,
kernel version, architecture, CPU vendor/family/model/stepping, microcode when the
host exposes it, normalized CPU features, physical/logical-core and NUMA topology,
and monotonic-timer kind and reported resolution. An unavailable field uses one
explicit unavailable value rather than being omitted. Hostname, username, hardware
serial numbers, and operating-system machine identifiers are forbidden.

### 12.3 Replay identity

Replay does not require the original manifest, runner, or workload inputs. It does
require an exact match for:

- compiler and all relevant schemas;
- source, semantics, contracts, and pre-tune KIR;
- target triple, native CPU, features, and target profile;
- optional profile identity or explicit absence;
- compilation modes and output kind;
- decision frontier, preconditions, and canonical selected plan.

The canonical .cktune decision digest enters the production native cache key. A
compiler, schema, source, CPU, feature, profile, mode, or plan change requires
retuning. A baseline-selection decision contains an empty override plan; tune-use
validates the decision normally and then reproduces the exact ordinary baseline.

## 13. Compilation and replay pipeline

The tuning pipeline is:

1. Resolve and digest compiler, source, target, modes, manifest, runner, inputs, and
   optional profile.
2. Build and fully verify the exact ordinary baseline.
3. Enumerate stable decision sites, tuning units, alternatives, and the frontier.
4. Run deterministic beam search under the selected preset.
5. Compile legal alternatives as non-publishable trial artifacts.
6. Run correctness smoke checks before full measurement.
7. Measure search finalists.
8. Validate top entrants twice on distinct validation cases.
9. Issue a measured-profitability certificate for the winning exact plan, or
   record the baseline reason.
10. Independently replay the selected plan from the pre-tune compiler state.
11. Rebuild and verify the selected object graph.
12. Compare canonical object-graph and link-recipe digests with the measured
    candidate.
13. Publish the final artifact and decision with the journaled artifact-last
    protocol in Section 6.

A later tune-use build repeats steps 1, 9 through 12 using the decision file. Every
decision site, frontier, precondition, pre-state, post-state, object-graph, and link
recipe digest must match.

The measured candidate and final code object graph must be identical. Packaging
paths, timestamps, and platform signing containers are excluded from canonical
comparison only when they cannot affect loaded code; the exclusion is explicit and
tested per platform.

## 14. Cache and interrupted sessions

Tuning data lives below the existing CK cache root in tune-v1. The default tuning
cache hard limit is 4 GiB.

The cache separates:

- compile identity: code identity plus exact plan;
- measurement identity: artifact, harness, workload, environment, and policy.

Measurement keys also contain a randomly generated local cache-installation salt
stored with private permissions. The salt is not written to .cktune. Consequently,
raw measurements are never reused merely because another machine reports the same
CPU model and operating-system tuple; moving a decision file remains allowed for
explicit tune-use on an otherwise exact compatible target.

Entries use private permissions, checksums, canonical validated paths, atomic
publication, and deterministic LRU eviction. Symlink and traversal attacks are
rejected.

An exact completed decision may be reused by a warm tune build. no-tune-cache forces
a fresh search and measurement. Interrupted sessions may reuse only fully verified
compiled candidates. An incomplete measurement phase is discarded and restarted
at row zero; samples from separate sessions are never spliced.

A completed baseline decision may record a candidate timeout, but that complete
decision is not eligible for completed-decision cache reuse. Crashes, protocol
errors, digest mismatches, semantic mismatches, and partial decisions are likewise
not cached as successful results. ckc cache clean removes both ordinary and tuning
cache entries with the existing safe-root protections.

The published artifact and decision file do not depend on the continued existence
of the cache.

## 15. Security, privacy, and failure behavior

- Tuning performs no telemetry, network upload, profile service, or remote
  execution.
- The runner is an explicit user-authorized executable; no shell interpolation is
  used.
- Inputs and outputs are bounded before allocation.
- Temporary directories and process trees are owned and cleaned by the session.
- Final pair publication uses the journaled, digest-checked, artifact-last protocol
  in Section 6.
- The public parser is fuzzed and mutation-tested.
- Trial typestate makes an unvalidated artifact unpublishable by construction.

The following abort the entire command with no new outputs:

- invalid configuration or identity;
- baseline compilation or verification failure;
- runner crash, signal, malformed protocol, output overflow, or wrong digest;
- required measurement instability;
- inability to complete fixed validation;
- a legal-plan compilation, verification, or replay mismatch;
- object-graph or link-recipe mismatch;
- any internal invariant or arithmetic overflow.

Only two outcomes are ordinary successes: a qualifying tuned artifact, or a
well-measured baseline artifact when no candidate clears the threshold.

## 16. Diagnostics and inspection

Text and JSON inspection expose:

- all compiler, schema, source, profile, target, CPU, feature, and mode identities;
- manifest and measurement policy identity;
- tuning units, alternatives, candidates, rejections, and plan choices;
- calibration, raw stored samples, medians, stability, correctness digests, and
  weighted scores;
- both validation decisions and thresholds;
- the selected plan or baseline reason;
- compile and measurement cache reuse;
- final replay and object-graph verification.

JSON uses the inspection schema and stable field ordering. Deterministic output does
not contain absolute paths, timestamps, temporary identifiers, hash-map order, or
localized prose.

When tune-use is combined with explain-optimization, each selected choice maps back
to its stable decision site, static prediction, measured evidence, guards, and
replay result.

## 17. Version and ABI contract

CK 0.14 changes optimization and cache behavior but not language or runtime ABI.

The required schema state is:

| Contract | CK 0.14 value |
| --- | ---: |
| Language contract | unchanged from v0.13 |
| Native ABI | 1 |
| Runtime ABI | 2 |
| KIR format | 3 |
| LLVM bridge ABI | 4 |
| CK profile schemas | 1 |
| Multiversion schemas | 1 |
| Native cache entry magic | CKCOBJ04 |
| Native cache key and manifest | 5 |
| Tuning input manifest | 1 |
| .cktune format, contract, measurement, inspection, and plan | 1 |

No new runtime symbol or shared-library dependency is permitted. Tuning schema
constants are centralized and covered by mismatch and mutation tests.

CKCOBJ03/schema 4 entries are clean misses under CK 0.14. They are never upgraded
in place or interpreted as schema 5.

## 18. Test and CI requirements

The existing ten required jobs remain required:

- quality;
- native integration;
- six native host jobs;
- two stable performance jobs.

All run against the exact candidate SHA. None may use continue-on-error or silently
skip a required capability.

The six native hosts verify:

- manifest and decision parsing;
- executable and dynamic harness protocols;
- deterministic search and replay;
- process-tree timeout and cleanup;
- cache permissions, invalidation, corruption, traversal, and eviction;
- journal recovery, rollback, digest-pair validation, and artifact-last output;
- ordinary non-tuning behavior;
- final artifacts preserving the existing self-contained system-runtime policy.

Performance claims are made only on the stable enhanced x86-64 and AArch64 workers.
Performance output advances to schema 9. CI additionally includes:

- sanitizer and ASan coverage for parsers, planner, and process control;
- decision and manifest fuzzing;
- mutation tests for schema, identity, digest, threshold, and typestate checks;
- fixed fixtures for endian, truncation, duplicate, trailing, and oversized input;
- killed-session recovery and sample non-splicing tests;
- deterministic cold and warm cache tests;
- negative tests proving tune-use fails closed;
- tests proving ordinary builds do not consult tuning state.

No CK 0.14 tag or release may be created before every required local and remote gate
passes.

## 19. Performance acceptance

CK 0.14 preserves every accepted v0.12 and v0.13 correctness, code-quality,
performance, compilation-time, and artifact gate. A regression against those gates
blocks release even if tuning benchmarks improve.

The frozen tuning corpus is partitioned before measurement into search, validation,
and sealed held-out cases. The held-out corpus is unavailable to the tuner. Cases
eligible for tuning and exclusions are declared before results; post-measurement
exclusion is forbidden.

For tune-eligible cases, compare the selected tuned result with the faster
identical-semantics v0.13 ordinary or PGO native baseline:

- held-out geometric mean is at least 5% faster;
- every selected case is at least 2% faster;
- no validation or held-out case is more than 2% slower.

Every corpus member declared tune-eligible before measurement participates,
including one for which the tuner selects the baseline. A baseline selection
therefore cannot be excluded after the fact to improve the release result.

Against the faster audited hand-written C or Rust plus explicit SIMD reference with
identical semantics and hardware:

- geometric mean performance is at least 98%;
- every case is at least 92%.

On the frozen domain-constraint suite, tuned CK beats the generic faster C or Rust
O3 result by more than 8% geometric mean.

Resource and determinism gates are:

- tuned artifact bytes are at most 110% of the matching baseline;
- tune-use compilation overhead is at most 10% geometric mean and 20% for any case
  versus the same build without tune-use, excluding tuning search;
- ordinary non-tuning compilation regresses by at most 3% geometric mean and 8% for
  any case;
- compiler archive size is at most 110% of the v0.13 accepted archive;
- a standard session completes within 30 minutes and its declared candidate bounds;
- peak tuner compiler RSS is at most twice the matching ordinary compilation;
- the tuning cache never exceeds its 4 GiB hard limit;
- two cold runs select the same plan and object graph;
- an exact warm-cache run compiles and measures zero candidates and reproduces the
  same decision and artifact plan digest;
- final artifacts contain no tuning runner, tuning symbol, runtime dispatch, or new
  runtime dependency.

The release evidence records hardware, operating system, compiler identity, raw
samples, exclusions, and exact artifact digests.

## 20. Release gate

CK 0.14 is releasable only when:

1. the final accepted v0.13 base is integrated and all carried gates remain green;
2. every normative behavior in this specification has positive and negative tests;
3. local total acceptance passes from a clean checkout;
4. all ten exact-SHA remote jobs pass;
5. the frozen performance corpus meets every threshold in Section 19;
6. documentation, CLI help, examples, schemas, and inspection output agree;
7. produced executables and dynamic libraries retain the promised zero-dependency
   deployment model;
8. the repository is clean and all release evidence is committed.

Passing only local functional tests, only search workloads, or only one performance
host is not sufficient.

## 21. Risks and controls

| Risk | Required control |
| --- | --- |
| Workload overfitting | Distinct search and validation cases, two validation rounds, and sealed release held-out cases |
| Measurement noise | External monotonic timing, calibrated batches, rotation, fixed samples, stability gate, and no cherry-picked reruns |
| Unsafe measured choice | Legality independent of profitability, trial typestate, exact replay, and final independent verification |
| Combinatorial explosion | Stable tuning units, finite alternatives, beam search, closed presets, and hard candidate limits |
| Stale decision reuse | Complete compiler/source/target/profile/mode/frontier identity and fail-closed replay |
| Cache poisoning | Private roots, canonical paths, checksums, bounded parsing, atomic entries, and identity separation |
| Harness compromise | Explicit user authorization, no shell, cleared environment, bounded I/O, and honest no-sandbox statement |
| Hidden ordinary-build cost | Explicit-only integration and ordinary compile-regression gates |
| Reproducing a measured artifact | Canonical plan, object graph and link recipe digests, and deterministic replay |

## 22. Deferred evolution

Later designs may consider multi-CPU or fleet tuning, portable baseline tuning,
static libraries, object outputs, cross compilation, scalable KIR, indirect-call
promotion, adaptive ORC JIT, source SIMD, relaxed floating point, GPU targets, remote
services, or telemetry.

They require separate language, security, identity, reproducibility, and acceptance
decisions. No compatibility promise for those features is made by the CK 0.14
tuning schemas.

## 23. Design completion criteria

This design is complete when its English and Chinese documents:

- describe the same normative behavior and constants;
- contain no unresolved choice, placeholder, or “implementation decides” escape;
- preserve all v0.13 language, safety, ABI, and release contracts;
- close the loop from explicit workload through legal search, measurement,
  validation, certificate, deterministic replay, cache, inspection, and release;
- can be decomposed into an implementation plan without inventing product policy.

Implementation must not begin from this branch until the design is reviewed and
approved and its provisional v0.13 base condition is resolved.
