# CK 0.14 Offline Auto-Tuning Specification

[简体中文](zh-CN/offline-autotuning.md)

Status: Proposed design for CK 0.14.0

Accepted base revision: v0.13 repaired candidate a42fbb08d067d77cc896d937b04876b858878d5d

This document is normative for the CK 0.14 implementation. It defines a bounded,
reproducible, cached, ahead-of-time auto-tuning system. It does not claim that the
implementation or release acceptance has completed.

Implementation began from v0.13 candidate
`94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`. Before final acceptance, the complete
delta through accepted v0.13 revision
`a42fbb08d067d77cc896d937b04876b858878d5d` was reviewed file by file and integrated
with v0.14-equivalent fixes. Deliberate supersessions are recorded in implementation
design correction 10; no semantic difference may be hidden by adapting tests.

For inherited schema-7 runtime samples on Linux, the unchanged native kernel-call
loop is measured with current-thread CPU time. The existing one-allowed-CPU affinity
scope and `bounded-upper-band-v1` calibration before each retained seven-call sample remain required. This excludes time while a
hosted runner does not schedule the benchmark thread without excluding kernel work;
non-Linux hosts retain the monotonic timer defined by schema 7. Historical schema-8
reports and their evidence are copied into the uploadable replay bundle before the
retained checker runs, so a checker rejection remains diagnosable.

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
- Optional use of an already collected CK profile from the exact same v0.14
  compiler/source/schema identity as a ranking and code-generation input. A v0.13
  profile is not accepted after the schema-5 cache identity change.
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
      [--explain-optimization]

The default budget is standard. Existing `NativeArtifactPaths` resolution defines
the complete output set: an executable has one primary output; a dynamic library
has the primary library and generated C header; and a Windows dynamic library also
has its import library. If `--tune-out` is omitted, the decision path is the
resolved primary-output path with `.cktune` appended. An explicit decision path
must have the same canonical parent directory as every output. If `--target` is
present it must normalize to the exact host triple; omission selects that triple.

For `tune build`, `--explain-optimization` retains ordinary diagnostics and, for
each selected predicated-update alternative, emits the independently verified
canonical attestation defined by
[`predicated-update-performance-1.md`](predicated-update-performance-1.md). It
does not alter candidate discovery, selection, decision bytes, or artifact bytes.

Every destination must be distinct after canonical destination resolution and
must not alias any source, manifest, runner, profile, declared input, or another
destination. Schema 1 does not support a multi-directory output set. The decision
records the role, canonical logical name, staged byte digest, and physical size of
the decision-independent primary, header, and import-library outputs that exist.

Every tune destination leaf is 1..255 ASCII bytes, matches
`[A-Za-z0-9][A-Za-z0-9._-]*`, is neither `.` nor `..`, does not begin
`.ckc-tune-`, and is not a Windows device name or a name with a trailing dot or
space. The grammar excludes `~`; therefore no requested destination can spell the
ordinary automatically generated Windows 8.3 form of a newly created long name.
That observation is not used for existing entries: on Windows CK opens every
existing destination by handle, obtains its authoritative long leaf and any short
leaf, replaces an alias spelling with the long leaf for canonical path and key
construction, and rejects the operation if either query is unsupported,
inconsistent, or collides with another destination. Thus a manually assigned short
name such as `ALT.DLL` cannot acquire a separate lock. CK opens the already-existing common parent directory no-follow and obtains
its stable volume/directory identity and lookup case behavior from the platform
adapter. The adapter must distinguish case-sensitive from ASCII-case-insensitive
lookup for that exact directory; unknown, mutable, or unsupported equivalence fails
before staging. Restricting leaves to ASCII removes Unicode normalization aliases.
All alias checks, sort keys, and locks use the resulting parent identity plus the
canonical long leaf's exact or ASCII-lowercase lookup key. Existing destinations
are additionally checked by handle identity. A destination absent during this
canonicalization is rechecked, with the same no-follow long/short-name procedure,
after the complete lock set is acquired and immediately before staging; a changed
namespace causes release and restart. CK never creates or assigns a short name.

Publication uses one persistent sibling lock per canonical decision, artifact, or
sidecar destination, plus a set journal, stage files, and backup files. A
destination lock is named from all 64 hexadecimal characters of
`H("CK-TUNE-DESTINATION\0", DestinationKeyMaterial)`; the set journal is named from
all 64 characters of `H("CK-TUNE-OUTPUT-SET\0", OutputSetMaterial)`. CK opens every
reserved file without following symbolic links or reparse points, acquires the
complete overlap-closure of destination locks in canonical destination-id order, and
holds it throughout recovery and publication. Lock files and journals store and
verify their full 32-byte ids; an identity mismatch is a hard error, never an alias.
Consequently two commands that publish the same primary artifact but choose
different explicit decision paths still serialize on the primary destination.

The sole byte-, filename-, barrier-, phase-, and recovery-level authority is the
shared normative attachment
[`publication-journal-1.md`](publication-journal-1.md). The summary below cannot
weaken or reorder that protocol.

The bounded journal schema 1 contains the transaction id, output-set id, phase,
durable recovery direction,
and, in publication order decision, header, import-library, primary, for each
present destination its destination, stage, and backup basename,
old-presence bit, old digest, new digest, and new size. Its phases are `Prepared=1`,
`BackedUp=2`, `DecisionPublished=3`, `SidecarsPublished=4`,
`PrimaryPublished=5`, and `Committed=6`. Publication is exactly:

1. create and verify all sibling stage files, flush every file, then flush the
   parent directory;
2. write and flush `Prepared`, then flush the parent directory;
3. rename each prior destination to its backup, flush the parent, then write and
   flush `BackedUp`;
4. atomically rename and flush the decision and parent, then record
   `DecisionPublished`;
5. publish and flush the header and then import library when present, flush the
   parent, and record `SidecarsPublished`;
6. publish and flush the primary artifact and parent last, then record
   `PrimaryPublished`;
7. verify the entire new set, record and flush `Committed`, remove backups and
   stages, remove the journal, and flush the directory.

Directory flush uses the strongest documented platform equivalent and is covered
by each native-host recovery test. Lock and journal final names are atomically
exposed only after their complete bytes are flushed. An error before primary
publication durably switches direction and rolls back; at or after primary
publication it completes roll-forward. On restart, recovery first resolves the
attachment's exhaustive active/update/private-write table and hashes every
destination, stage, and backup. Durable rollback always continues rollback;
otherwise phases and primary identity select the specified roll-forward or durable
rollback path. Both operations are idempotent. An impossible or unrecorded digest combination is a hard recovery
error that preserves the journal and all evidence for diagnosis rather than
guessing. A later command may not touch the set until recovery succeeds.

This journaled primary-last protocol is the meaning of transactional output-set
publication here; simultaneous atomic visibility of several files is not claimed.
Pair consumers accept a published result only after the decision and every recorded
output role agree with the on-disk digest and size.

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
- one host-native runner executable path used only to acquire an immutable snapshot;
- one operational input-root directory, defaulting to the manifest directory;
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
| runner | path | Required UTF-8 path to a host-format native executable; relative values resolve from the canonical manifest parent, absolute values are accepted, and the path is operational rather than canonical identity |
| runner | input_root | Optional UTF-8 directory path resolved from the manifest directory; default `.`; operational only |
| runner | args | Optional array of at most 64 `Text` strings; default empty; each is already NFC, NUL-free, at most 4,096 UTF-8 bytes, and the total is at most 64 KiB |
| runner | inputs | Optional array of at most 64 input-root-relative regular-file `Text` paths; default empty; each path is already NFC, NUL-free, nonabsolute, nontraversing, and at most 4,096 UTF-8 bytes; each file is at most 1 GiB and their total at most 4 GiB |
| runner | inherit_env | Optional array of at most 16 unique names matching [A-Za-z_][A-Za-z0-9_]*; default empty |
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
    input_root = "."
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

On Unix the runner receives the exact accepted UTF-8 argv bytes. On Windows CK
passes the runner snapshot path separately as `lpApplicationName`, uses that same
path as argv element zero, and converts every accepted Unicode scalar sequence to
UTF-16 without normalization or lossy replacement. It applies the Microsoft
UCRT argv inverse and always quotes each argument. For every run of `n` backslashes,
ordinary following text emits `n`; a following quote emits `2n+1` backslashes then
the quote; and the closing quote emits `2n` backslashes. The quoted arguments join
with one U+0020. A conforming runner uses UCRT-compatible argv decoding; a runner
that reparses the raw command line differently is outside schema 1. Golden process
tests execute a retained probe and require exact argv recovery for empty,
whitespace, quote, trailing-backslash, and non-ASCII arguments. CK never normalizes
an argument after validation. The runner working directory is always CK_TUNE_TEMP.
The manifest cannot override it. Resource-output limits are fixed by this
specification rather than configurable manifest fields.

### 7.2 Paths and inputs

An absolute `runner.path` is used as written; a relative value, including parent
components, resolves from the canonical manifest parent, never the caller or source
working directory. CK walks every component without following symlinks/reparse
points and opens the final file by handle before snapshotting; any ambiguity is an
error. `input_root` resolves from the canonical manifest directory and is opened as one
stable no-follow directory handle; its spelling may contain parent components, but
the resolved root is fixed before any input is opened. Each logical input path then
resolves beneath that handle. Absolute paths, parent traversal within a logical
path, symlink/reparse ambiguity, non-regular files, and escape from the opened root
are rejected. Two inputs resolving to the same file handle identity are rejected.
The runner itself may be outside that directory but
must be named by an explicit path; PATH lookup is forbidden. Schema 1 accepts only
the host's native ELF, Mach-O, or PE/COFF executable format, not a script or
interpreter directive.

The canonical manifest identity is not the raw TOML byte stream. It is
`H("CK-TUNE-MANIFEST\0", ManifestMaterial)` using the primitive framing in the
normative decision-schema attachment, in this order:

1. schema;
2. runner argv strings;
3. sorted effective inherited-environment name/value-length/value-digest records;
4. timeout;
5. `ManifestInputMaterial` records in manifest order, each containing logical path,
   content digest, and byte length at tags 1..3;
6. `ManifestCaseMaterial` records in canonical identifier order, each containing
   case id, role, seed, weight, and expected digest at tags 1..5;
7. the immutable runner-snapshot byte length and content digest.

The exact material records and outer tags are defined once in
[`decision-schema-1.md`](decision-schema-1.md); this list is only an aligned
overview. `input_root` and source path spellings are operational and excluded; the
logical path plus immutable bytes remain canonical identity.

The operational manifest and runner absolute paths, TOML whitespace/comments,
timestamps, file permissions other than executable validity, and temporary paths
are excluded. Changing any logical field or content byte invalidates reuse.

At session start, CK opens the runner and every validated input without following
symlinks or reparse points. It copies the runner into a private session snapshot,
verifies the copied bytes and host executable format, and executes only that
snapshot for the rest of the session. It likewise streams each input into a private
immutable-content snapshot and verifies its digest. Before each timed invocation,
CK copies snapshots to flat, content-addressed files below
`CK_TUNE_TEMP/inputs`, named exactly as eight lowercase hexadecimal digits for the
zero-based manifest ordinal, one `-`, the 64-character lowercase content digest,
and `.bin`, and creates a
read-only `CK_TUNE_INPUT_MAP`. That bounded canonical map is exactly eight ASCII
bytes `CKTIMAP1`, `U32_BE(input_count)`, then one concatenated record for each
manifest-order input. A record is logical-path `Text`, staged ASCII basename
`Text`, byte length `U64`, and digest `D32`; `Text` is `U32_BE(UTF-8 byte length)`
plus those bytes, `U64` is big-endian, and `D32` is exactly 32 bytes. The count is
0..64 and parsing must end exactly after its records; truncation, overflow, invalid
UTF-8, a count mismatch, or a trailing byte is an error. Generated long basenames
are distinct under exact and ASCII-folded comparison. CK opens the actual temporary
parent, create-news every entry no-follow, and on Windows enumerates every resulting
long/short-name pair and requires a one-to-one relation: no long or short spelling
may equal or resolve to any other staged entry. Unsupported or inconsistent
enumeration fails closed. All files are rehashed. Input preparation is outside the measured
interval, and each invocation receives a fresh map and files, so one runner
invocation cannot change a later invocation's input.

### 7.3 Environment

The runner starts from an empty environment. On Windows, CK may provide only the
minimal SystemRoot and WINDIR values needed to create a process. Any other inherited
variable must be explicitly allowlisted. A requested variable that is absent is an
error. Names are unique byte-for-byte on Unix and unique under ASCII
case-insensitive comparison on Windows; duplicate or noncanonical spelling is an
error. The complete effective environment, not merely the user allowlist, is capped
at 16 entries. Windows inserts its required base names first; an allowlisted base
name with canonical spelling refers to that one existing entry, while conflicting
case is rejected. Validation fails if the union exceeds 16. Each value is at most
4,096 bytes and the complete effective environment is
at most 65,536 bytes; NUL is rejected. Every effective name and exact value
identity, including the Windows platform base values, enters tuning identity.

Canonical inherited values are the exact non-NUL bytes on Unix. Windows values are
encoded as UTF-8 without normalization and an unrepresentable value is rejected.
Only name, exact byte length, and
`H("CK-TUNE-ENV-VALUE\0", name Text, value Bytes)` enter the public decision; the
actual value remains only in private session memory and process state and is never
rendered by inspect.

CK sets these protocol variables:

    CK_TUNE_PROTOCOL=1
    CK_TUNE_ARTIFACT=<absolute candidate artifact path>
    CK_TUNE_ARTIFACT_KIND=<executable|dynamic>
    CK_TUNE_CASE=<case identifier>
    CK_TUNE_SEED=<unsigned decimal u64>
    CK_TUNE_ITERATIONS=<unsigned decimal u64>
    CK_TUNE_TEMP=<absolute private per-run directory>
    CK_TUNE_INPUT_MAP=<absolute private input-map path>

The argv and environment are passed directly to process creation. CK never builds
a shell command string.

### 7.4 Harness responsibility

The harness is tuning-only and is not linked into or required by the final artifact.
It must read declared input locations from CK_TUNE_INPUT_MAP, load or execute
CK_TUNE_ARTIFACT, run exactly CK_TUNE_ITERATIONS logical
iterations of the named case, and produce a deterministic correctness digest.

For a dynamic library, the harness owns loading and calling its exported ABI. For
an executable, the harness owns invoking or driving it. Artifact kind comes only
from ckc tune build; it is passed through CK_TUNE_ARTIFACT_KIND and is not a
manifest field. CK does not infer an application protocol.

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
and cleans temporary files.

Schema 1 provides cooperative process containment, not a hostile-code sandbox:

- Windows creates the runner suspended, assigns it to a non-breakaway Job Object
  with KILL_ON_JOB_CLOSE, then resumes it. Failure to establish that job is a tuning
  error.
- Linux and Darwin create a new process group before runner code executes. The
  runner contract forbids setsid, changing process group, double-fork
  daemonization, background work that outlives the invocation, and any equivalent
  containment escape.
- On timeout or output overflow CK requests group/job termination, waits 250 ms,
  forcibly terminates the group/job, reaps the direct runner, and allows at most
  another 2,000 ms for the cooperative containment to become empty.
- Failure to establish containment, terminate, reap the runner, or observe the
  cooperative containment empty aborts the session.

An intentionally escaping POSIX descendant is arbitrary hostile same-user behavior
outside this no-sandbox contract. CK does not claim that it can discover or kill
such a process.

The harness must batch enough work to amortize startup. For every search and
validation case in canonical identifier order, baseline calibration is:

1. start at iterations = 1;
2. perform at most 32 timed baseline attempts;
3. validate the expected digest after every attempt;
4. accept the first attempt lasting at least 50 ms;
5. otherwise double iterations with checked u64 arithmetic and retry;
6. after acceptance, run one additional baseline confirmation invocation with the
   same iterations and digest;
7. record calibrationOvershoot when the accepted attempt exceeds 250 ms.

Failure to reach 50 ms in 32 attempts, arithmetic overflow, baseline timeout, or
confirmation failure aborts the session. The 250 ms value is a preferred upper
target, not a reason to reject a workload whose single logical iteration is
coarser. The accepted iteration count is fixed for that case in search and both
validation rounds; validation never recalibrates.

A candidate that consumes its complete configured timeout after successful
baseline calibration is a canonical performance rejection. CK applies the
containment shutdown above, records the timeout, and skips that immutable channel
slot in all later rows and rounds without changing the order of surviving
channels. This counts as completed validation for the rejected candidate. A
baseline timeout, shortened deadline, crash, protocol error, correctness mismatch,
or containment failure aborts the session.

### 8.4 Invocation state machine and ordering

After all cases are calibrated, the fixed state machine is:

1. after artifact-size rejection and postcompile finalist selection, each
   size-valid measured finalist, in plan-digest order, receives one
   correctness-smoke invocation for every search case in case-id order;
2. search executes three warmup rows and twenty measured rows over the immutable
   baseline-plus-finalists channel list;
3. the best bounded surviving candidates enter validation;
4. validation round 1 executes three warmup and twenty measured rows over its
   immutable baseline-plus-entrants list;
5. validation round 2 repeats the same matrix with a distinct ordering domain;
6. selection runs only after every required surviving stream is complete.

Before candidate smoke, CK derives:

    session_digest = H("CK-TUNE-SESSION\0",
                       Identity,
                       Contract,
                       Workload,
                       Environment tags 1..16,
                       complete Frontier,
                       baseline plan/object-graph/link-recipe/size tuple)

`H` and each canonical record are defined by `decision-schema-1.md`. Calibration
records, measurements, correctness results, cache origins, temporary paths,
timestamps, and publication destinations are excluded. The derived digest is stored
as Environment tag 18 and is the sole measurement-order seed.

A warmup channel evaluation is exactly one invocation. A measured channel
evaluation is exactly three invocations and stores their minimum. Every invocation
validates protocol and correctness. The baseline is present in every phase.

Smoke has phase 1, round 0, row 0, and call 1; candidates and cases retain the
plan-digest and case-id order stated above, so smoke requires no channel rotation.

Cases are stored in case-id order. Channels are stored as baseline followed by
ascending plan digest. For each row, CK computes
`H("CK-TUNE-ORDER\0", sessionDigest D32, phase U8, round U8, row U32,
caseId Text)` using the attachment's canonical typed encoding. The first eight
bytes interpreted as a big-endian u64 select a left-rotation modulo the channel
count for that case and are stored as the row's `permutationKey`. The same domain
and first four typed values followed by `Bytes([0xff])` select the case-list
rotation. Phase values are 1 candidate-smoke,
2 search-warmup, 3 search-measured, 4 validation-1-warmup,
5 validation-1-measured, 6 validation-2-warmup, and 7 validation-2-measured. round is zero outside
validation and 1 or 2 inside it. Rejected channel slots remain explicit skips, so
their removal cannot reorder favorable samples.

If a candidate times out, the partial stream is discarded but the exact timeout
coordinate is retained. The decision stores the canonically sorted set of every
stream whose twentieth row completed earlier in the actual rotated invocation
schedule; it is not required to be a prefix of canonical stream order. The checker
recomputes this set from the session digest and timeout coordinate.

Before starting an invocation, CK requires at least the complete configured
timeout plus the fixed 2,250 ms containment-cleanup allowance to remain in the
session wall budget. Otherwise it aborts with incomplete evidence without launching
the process. CK never shortens a runner timeout to fit the session deadline. Only
expiration of the complete configured candidate timeout is a performance rejection.

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

The Loop SIMD class also owns a predicated same-place update alternative for the
canonical loop form `if candidate < old { dst[index] = candidate }`. This is not
masked-memory support. A legal rewrite loads `old` from the exact destination,
computes a vector predicate, selects `candidate` or `old`, and performs one
ordinary unmasked vector store. It is available only when all of the following
close over the immutable pre-rewrite KIR:

- one branch path contains exactly one store, the empty path goes directly to the
  merge/latch without a memory operation, and the store path joins that same
  merge/latch;
- the store destination is the same affine place as the dominating `old` load,
  consumes its incoming Memory SSA version, and has no intervening memory
  definition;
- the condition, candidate, and index computation are free of calls, prints,
  volatile access, possible failure, and any other ordered effect;
- the dependence proof excludes loop-carried and cross-lane conflicts for every
  load and the selected store;
- strict floating-point compare/select semantics preserve unordered comparisons,
  NaNs, infinities, signed zero, and operand order without fast math or
  contraction;
- every vector lane is in range and every checked arithmetic operation is proven
  non-failing when checked modes are selected.

The independent vector checker reconstructs the diamond, same-place relation,
affine accesses, Memory SSA versions, lane bounds, dependence result, and exact
compare/select/store rewrite. A missing proof rejects the candidate and leaves the
scalar loop byte-for-byte unchanged. Accepted loops retain the ordinary runtime
alias/versioning guard when required and always retain an ordered scalar epilogue.
The alternative uses the existing Loop SIMD unit class and its vector-width,
interleave-factor (UF), and break-even parameters. It adds no decision-file tag,
language construct, public/native/runtime ABI surface, or target feature.
For each eligible loop, the proposer emits the canonical bounded set of distinct
target-supported VF/UF combinations already representable by that payload instead
of collapsing them to the ordinary cost-model winner. The ordinary winner remains
the baseline; legal combinations enter the existing unit-variant bound and are
measured by the unchanged deterministic frontier search. An unsupported vector
width, operation, or target feature is never emitted as a trial.

Each decision site and alternative has a canonical stable identifier, precondition
digest, and ordering. The trace records both accepted and rejected alternatives.

The canonical pre-tune snapshot is the verified v0.13 O3 KIR after CFG
canonicalization, initial SCCP/range analysis, loop canonicalization, and the first
mandatory check elimination, immediately before specialization. Candidate replay
uses one fixed profitability-controlled phase order: specialization, inlining,
short-slice/versioning, loop-SIMD, unrolling, SLP, then layout. Tuning units sort by
`(phase, unit id)` and a unit's alternatives sort by `(site id, alternative id)`.
The existing mandatory analyses, legality checks, proof refreshes, and cleanup
passes remain at their v0.13 positions between those phases and cannot be selected
or suppressed by a plan. Layout choices are canonical KIR metadata before native
lowering; the backend consumes that metadata after the unchanged fixed LLVM O3
pipeline and before object emission. The empty plan uses the unmodified v0.13 O3
profitability decisions and leaves ordinary compilation and the existing O2-only
late-layout behavior unchanged.
Once a non-layout tuning choice is present, replay never re-enters an ordinary
profitability-controlled phase: only the exact selected alternatives run, followed
by the mandatory analyses and cleanup suffix. This prevents an early-only plan
from acquiring an unrecorded ordinary specialization or inline choice.
Because layout is metadata rather than an O3 rewrite, a layout-only plan first
completes the fixed ordinary KIR O3 suffix. CK then projects the selected pre-tune
block permutation onto surviving block ids and appends any blocks created by the
fixed suffix in their canonical post-suffix order. An empty projection or a
projection equal to the canonical order is a measured no-op. This deterministic
projection is part of source-aware replay; layout may never suppress the KIR O3
suffix or name blocks that no longer exist.
LLVM O3 may legally delete a selected function or block before late layout. The
backend therefore preserves selected functions when possible, then reconciles the
layout list against the post-O3 module. It applies the complete surviving mapping;
if no complete selected mapping survives, layout becomes a measured no-op rather
than naming a nonexistent object or changing the LLVM pipeline.

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

Within one alternative class, overlapping roots, cloned helpers, specialization
boundaries, and shared code-size effects are clustered deterministically into one
tuning unit. The schema-1 `Unit.class` field keeps every unit class-homogeneous;
cross-class interactions are represented by canonical whole-plan expansions over
multiple units. If a later class invalidates an earlier class's anchor or
precondition, that expansion is retained as an `illegal` search result and search
continues rather than aborting the session. A session considers at most 64 units.
Units beyond that bound use ordinary optimizer decisions, selected by canonical
rank rather than discovery order.

Each unit exposes at most four coherent non-baseline unit variants. A unit variant
is one closed set of choices at one or more sites; the command line never forms a
Cartesian product of independent site alternatives. A session records at most
4,096 sites, 64 non-baseline choices in one plan, 256 unit variants, and 16,384
attempted plan expansions.

### 9.4 Trial typestate

Candidate materialization separates legality from static profitability:

1. CK recomputes all structural, proof, effect, guard, failure-order, target-feature,
   and growth checks.
2. CK must expose a legal measurement-owned alternative even when the ordinary
   static-profitability threshold rejects it; such a trial may bypass only that
   threshold, never any legality, proof, target, transaction, or growth check.
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
unit-variant sets. Before compilation, candidate ordering is exactly:

1. predicted dynamic cost in canonical cost-model units;
2. predicted static cost in canonical cost-model units;
3. canonical `print_kir_module` byte length;
4. number of non-baseline choices;
5. the unit-order vector of alternative-class enum values;
6. the unit-order vector of `(unit id, variant id)` byte pairs;
7. plan digest.

These are whole-plan keys, never a sum of per-variant estimates. After every legal
extension CK reapplies the complete plan to a fresh copy of the same pre-tune KIR,
runs the canonical cost model over the resulting whole module, and computes the
printed canonical KIR byte length with checked `u64` conversion. Failure or overflow
is a compiler error. The expansion trace stores all three whole-plan metrics.

Artifact bytes are unavailable before compilation and therefore never participate
in this ranking. After compilation, actual artifact bytes replace canonical KIR
byte length in the same ordering for measured-finalist selection.

The closed algorithm is:

    beam = [baseline]
    expansions = 0
    for unit in canonical_unit_order:
        pool = beam                         # free baseline carry
        for plan in beam_precompile_rank_order:
            for variant in unit.nonbaseline_variants_in_canonical_order:
                if expansions == expansion_limit: stop all further expansion
                ordinal = expansions
                expansions += 1
                derive plan + variant and run all KIR legality/growth checks
                record the attempt with ordinal, including illegal, duplicate, or over-growth
                add a legal, unique derived plan to pool
        unique = deduplicate(pool without baseline)
        beam = [baseline] + diversity_truncate(unique, beam_width)
    frontier = beam without baseline
    compile_selection = diversity_truncate(frontier, compile_attempt_limit)

Baseline carry never consumes a beam slot, expansion, or compile attempt. Every
attempted non-baseline derivation consumes one expansion before validation; an
illegal, duplicate, over-growth, or cache-hit attempt is never refunded. Reaching
the expansion limit stops expansion before the next derivation; already accepted
plans remain eligible and later units retain their baseline behavior. Selecting a
plan for compilation consumes one compile-attempt slot even when a verified compile
cache hit avoids physical recompilation. A failure to compile a plan that CK already
declared legal is a compiler error.

Expansion ordinals are zero-based and contiguous: the first recorded attempt is 0,
each later attempt is exactly one greater, the list is exactly
`0..expansions-1`, and `expansions` equals its length. The trace must contain every
attempt generated by the nested loops until units are exhausted or the preset limit
is reached; an omitted, inserted, reordered, or reclassified attempt is invalid.

`deduplicate` keeps the first canonical precompile-ranked occurrence of each plan
digest. `diversity_truncate` considers alternative classes in this fixed order:
inlining, specialization, unrolling, loop SIMD, SLP, short-slice/versioning, and
layout. While slots remain, it takes the best plan whose newest non-baseline choice
belongs to each class, once per class; if the beam is narrower than the number of
available classes, the fixed class order wins. It fills remaining slots by the
global rank, skipping plans already chosen. This rule applies identically to the
beam, compile selection, and postcompile finalist selection and prevents the static
model from eliminating every structurally different legal candidate.

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
and transactional staging. A runner invocation starts only when the remaining
budget can cover its full configured timeout plus the fixed 2,250 ms containment
allowance from Section 8.4; its timeout is never shortened to fit the session. An
exact completed-decision cache hit performs no search session.

A candidate artifact may be at most 110% of matching baseline bytes. Existing KIR,
rewrite, specialization, and per-pass growth bounds remain in force. Rejected or
invalid attempts consume their recorded expansion or compile budget and are never
refunded. Of size-valid compiled plans, postcompile ranking selects at most the
preset's measured-finalist count; fewer valid plans simply proceed as a smaller
complete frontier.

A successful decision exists only after the entire deterministic expansion trace
and derived compile selection complete. The final checker replays the closed
algorithm from the candidate space and preset: the decision's trial set is exactly
the complete compile selection, stored by plan digest with no omission or extra
plan. It independently rebuilds each trial in an isolated cache and verifies its
plan, object/link identities, primary digest, and actual bytes. Size-rejected trials
are exactly those above 110%; applying the same diversity rule with actual primary
bytes in place of KIR bytes to every remaining trial yields the exact measured
finalist set. Every size-valid nonfinalist is `compiled-unmeasured`; every finalist
must have exactly the smoke/search/validation outcome and streams required by the
state machine. If the wall budget prevents completing the expansion or compile
selection, the command fails without a decision rather than serializing a partial
frontier.

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

No floating-point arithmetic participates in selection. Entrants are totally
ordered by lower search score, smaller actual primary-artifact bytes, fewer
non-baseline choices, then lower plan digest; the best bounded entrants advance to
validation. All products and sums use checked u128 arithmetic, and the persisted
Q32 result must fit u64.

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

Decision Schema 1 makes every persisted round field a checked derivation of the
matching calibration records and phase-5/7 raw streams: case medians, Q32 ratios,
weighted aggregate, stability, paired wins, entrant membership, threshold bit, and
rank cannot be supplied independently.

Within each round, qualifying plans are ranked and ties are resolved by:

1. lower validation score;
2. smaller artifact;
3. fewer non-baseline choices;
4. lower plan digest.

Let `Q1` and `Q2` be the ordered qualifying-plan lists for rounds 1 and 2. Selection
is the following disjoint and exhaustive table, evaluated in order:

| Predicate | Result |
| --- | --- |
| there are no validation entrants | baseline, `no-candidate` |
| `Q1` or `Q2` is empty | baseline, `validation-threshold` |
| both are nonempty and `Q1[0] == Q2[0]` | that plan, `tuned` |
| both are nonempty and `Q1[0] != Q2[0]` | baseline, `validation-disagreement` |

Under a threshold result every surviving completed validation entrant has candidate outcome
`validation-threshold`. Under a disagreement all entrants are
`validation-nonwinner`; under a tuned result the common winner is `selected` and
every other entrant is `validation-nonwinner`. The no-candidate row has no
validation-entrant outcomes and does not rewrite earlier trial outcomes. A timed-out
entrant always retains `timed-out` and is absent from `Q1`/`Q2`; it is never
rewritten by this table.

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
| 8 | Replay frontier, pre/post states, chosen-code identity, object graph, link recipe, and cache-reuse facts |

Nested records use the same increasing u16-tag/u32-length framing. Unsigned scalars
are fixed-width big-endian; booleans are one canonical byte 0 or 1; strings are
length-prefixed valid UTF-8; lists start with a checked big-endian u32 count and
contain canonical ordered records. Optional values use an explicit one-byte
presence discriminator. There is exactly one encoding for every valid decision.
The trailing hash is the canonical decision digest used by replay and cache keys.

The maximum file size is 32 MiB. It contains at most 33 candidates including the
baseline, 16 cases, and 64 choices per plan. The following limits are normative:

| Item | Limit |
| --- | ---: |
| UTF-8 text field / diagnostic | 4,096 bytes |
| argv entries / total argv bytes | 64 / 65,536 |
| environment entries / total environment bytes | 16 / 65,536 |
| declared inputs / one input / all inputs | 64 / 1 GiB / 4 GiB |
| cases / sites / units / variants | 16 / 4,096 / 64 / 256 |
| variants per unit / expansion records | 4 / 16,384 |
| candidates including baseline / choices per plan | 33 / 64 |
| output records / samples per complete stream | 3 / 20 |
| measurement streams | 1,584 |

Unknown, duplicate, truncated, trailing, out-of-order, over-limit, or noncanonical
content is rejected. Parsing allocates only after bounds and overflow checks.

### 12.2 Closed schema 1 records

The sole tag-by-tag wire authority is the shared normative attachment
[`decision-schema-1.md`](decision-schema-1.md). It freezes every primitive, nested
tag, type, enum, required/optional state, value, bound, and ordering. The overview
below is explanatory and cannot override that attachment. A required field is
present exactly once; only `Opt<T>` fields are optional; and absolute paths are
forbidden in every `Text` field.

The exact public JSON and stable text projections are separately frozen by the
shared normative attachment
[`inspection-schema-1.md`](inspection-schema-1.md). Neither renderer may invent,
omit, localize, or reorder validated decision data.

Top-level tag 1, `Identity`, contains:

| Tag | Type | Meaning |
| ---: | --- | --- |
| 1 | Text | CK version |
| 2 | D32 | CK source identity |
| 3 | Text | Rust toolchain identity |
| 4 | Text | LLVM identity |
| 5 | D32 | LLVM bridge identity |
| 6..15 | U32 | language, native ABI, runtime ABI, KIR, proof, cost-model, target, native-cache, profile, and PGO-analysis schemas |
| 16 | D32 | source digest |
| 17 | D32 | semantic and contract digest |
| 18 | D32 | pre-tune KIR digest |
| 19 | D32 | compilation-mode digest |
| 20 | U8 | output kind enum |
| 21 | Record | target triple, CPU, features, and target-profile identity |
| 22 | Opt<Record> | profile schema, compiler/source identity, topology, and byte digest |

Top-level tag 2, `Contract`, contains the five schema values, budget preset and its
six search bounds, exact artifact ratio, calibration, sampling, containment,
stability and validation integers, and the domain-separated policy digest. Its 32
tags and their exact values are fixed by the attachment and must equal Sections 8,
10, and 11.

Top-level tag 3, `Workload`, contains:

| Tag | Type | Meaning |
| ---: | --- | --- |
| 1 | D32 | canonical manifest identity |
| 2 | D32 | private runner-snapshot digest |
| 3 | U64 | runner-snapshot length |
| 4 | List<Text> | argv, in manifest order |
| 5 | List<Record> | effective environment, sorted by platform-normalized name |
| 6 | U32 | timeout milliseconds |
| 7 | List<Record> | inputs in manifest order: logical path, digest, size |
| 8 | List<Record> | cases sorted by case id: id, role enum, seed, weight, expected digest |

Top-level tag 4, `Environment`, contains the closed measurement tuple, timer and
scheduling evidence, followed by a case-id-ordered calibration record list. Each
calibration records iterations, attempts, accepted and confirmation elapsed times,
and overshoot, followed by the derived session digest and local measurement-cache
salt digest. Unavailable text uses
`unavailable`; unavailable numeric host facts
use their explicit absent optional state.

Top-level tag 5, `Frontier`, contains the candidate-space digest at tag 1, sites at
tag 2, units at tag 3, and expansion trace at tag 4. Records are:

| Record | Required fields in tag order |
| --- | --- |
| Site | stable site id `D32`; class enum `U8`; root id `D32`; pre-state digest `D32`; canonical rank `U32`; stable root anchor |
| Unit | stable unit id `D32`; ordered site-id list; baseline state digest `D32`; ordered variant list |
| UnitVariant | variant id `D32`; class enum `U8`; ordered choices with closed class payloads; isolated dynamic/static/KIR-byte estimates `U64`; post-state digest `D32` |
| PlanChoice | unit id `D32`; variant id `D32`; class enum `U8`; pre-state `D32`; post-state `D32` |
| Expansion | ordinal `U32`; parent plan `D32`; unit id `D32`; variant id `D32`; disposition enum `U8`; resulting plan `Opt<D32>`; diagnostic code `U16`; three optional whole-plan rank metrics |

Top-level tag 6, `Candidates`, contains the baseline candidate at tag 1 and a list
of non-baseline candidates in plan-digest order at tag 2. A candidate record has:

| Tag | Type | Meaning |
| ---: | --- | --- |
| 1 | D32 | plan digest; baseline uses the canonical empty-plan digest |
| 2 | List<PlanChoice> | choices in unit order |
| 3 | D32 | object-graph digest |
| 4 | D32 | link-recipe digest |
| 5 | U64 | actual primary-artifact bytes |
| 6 | U8 | outcome enum |
| 7 | U16 | diagnostic code, zero when absent |
| 8 | Opt<D32> | correctness digest |
| 9 | List<Record> | measurement streams in canonical order |
| 10 | Record | immutable compile CacheOrigin |
| 11 | Opt<Record> | exact timeout location; required only for timed-out outcome |
| 12 | D32 | actual primary-artifact content digest |

A measurement stream contains phase, round, case, plan, iterations, twenty ordered
row records, and correctness digest. Each row contains its ordinal, permutation-key
digest, exactly three raw nanosecond calls, and their minimum stored sample. Warmup
calls are executed but not stored. A canonically timed-out candidate has no later
streams and carries the exact timeout location. The attachment's terminal-state
matrix defines every intentionally unmeasured state; a stream required by that
matrix may not be absent.

Top-level tag 7, `Selection`, contains round-one and round-two summaries at tags 1
and 2, the selected plan digest at tag 3, selection-reason enum at tag 4, and an
`Opt<Certificate>` at tag 5. Each summary contains case medians, aggregate Q32 ratio,
stability result, threshold result, and ranked entrant plan digests. A certificate
contains the exact plan, frontier, policy, both-round, correctness, object-graph,
and link-recipe digests. A tuned selection requires a certificate; a baseline
selection forbids one.

Top-level tag 8, `Replay`, contains frontier, selected pre-state, selected post-state,
object-graph, and link-recipe digests at tags 1..5; a role-sorted list of output
records at tag 6; immutable compile and measurement CacheOrigin records at tags
7..8; the replay-result digest at tag 9; and the measurement-independent
choice-identity digest at tag 10. Each output record contains output-role enum, canonical
logical basename, staged byte digest, and physical size. The executable output set
contains only primary; a dynamic output set additionally contains header and, on
Windows, import library.

Closed enum values are:

| Enum | Values |
| --- | --- |
| output kind | executable=1, dynamic=2 |
| budget | quick=1, standard=2, thorough=3 |
| case role | search=1, validation=2 |
| alternative class | inlining=1, specialization=2, unrolling=3, loop-SIMD=4, SLP=5, short-slice/versioning=6, layout=7 |
| expansion disposition | legal=1, illegal=2, duplicate=3, growth-rejected=4 |
| candidate outcome | baseline=1, compiled-unmeasured=2, size-rejected=3, timed-out=4, search-nonwinner=5, validation-threshold=6, validation-nonwinner=7, selected=8 |
| ordering phase | candidate-smoke=1, search-warmup=2, search-measured=3, validation-one-warmup=4, validation-one-measured=5, validation-two-warmup=6, validation-two-measured=7; only 3, 5, and 7 occur in stored streams |
| selection reason | tuned=1, no-candidate=2, validation-threshold=3, validation-disagreement=4 |
| output role | primary=1, header=2, import-library=3 |
| cache-origin kind | freshly-built=1, verified-local-hit=2 |
| diagnostic code | none=0, legality-rejected=1, growth-rejected=2, artifact-size-rejected=3, candidate-timeout=4 |

Collections are ordered by: cases by case id; sites, units, and variants by stable
id; expansion records by ordinal; candidates with baseline first then plan digest;
plan choices by application phase then unit id; streams by phase, round, case id, and plan digest; stream
rows by ordinal; and outputs by output role. Text comparisons and ordering operate
on encoded UTF-8 bytes.

The repository carries five normative schema fixtures:

- `tests/fixtures/tune/decision-schema1-framing.hex`;
- `tests/fixtures/tune/decision-schema1-baseline.cktune`;
- `tests/fixtures/tune/decision-schema1-tuned.cktune`;
- `tests/fixtures/tune/decision-schema1-inspection.json`;
- `tests/fixtures/tune/decision-schema1-inspection.txt`.

The framing vector covers every scalar/container type and both optional states. The
baseline vector has one search and one validation case and `no-candidate`; the tuned
vector has one unit, one legal variant, complete three-phase samples, and a valid
certificate. Before the parser implementation is accepted, these bytes and their
SHA-256 values are frozen in the schema test; encode, decode, inspect, re-encode,
truncation, mutation, and cross-endian tests consume the same fixtures.

### 12.3 Recorded identity

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
- the complete role-tagged staged output-set byte digests and physical sizes;
- the measurement-independent chosen-code identity used across cold searches;
- immutable compile and measurement cache-origin facts.

Raw workload files, arbitrary runner stdout, secrets, and absolute paths are not
stored in canonical identity. Human diagnostics may show explicitly marked
noncanonical local paths while the tuning command is running.

The measurement-environment tuple is closed: operating-system family and build,
kernel version, architecture, CPU vendor/family/model/stepping, microcode when the
host exposes it, normalized CPU features, physical/logical-core and NUMA topology,
and monotonic-timer kind and reported resolution. An unavailable field uses one
explicit unavailable value rather than being omitted. Hostname, username, hardware
serial numbers, and operating-system machine identifiers are forbidden.

### 12.4 Replay identity

Replay does not require the original manifest, runner, or workload inputs. It does
require an exact match for:

- compiler and all relevant schemas;
- source, semantics, contracts, and pre-tune KIR;
- target triple, native CPU, features, and target profile;
- optional profile identity or explicit absence;
- compilation modes and output kind;
- decision frontier, preconditions, and canonical selected plan.

The canonical .cktune decision digest enters the production native cache key. The
recorded output-set digests validate the original published pair and any completed-
decision cache hit. A later tune-use may use a different destination basename; it
must reproduce the recorded object graph and link recipe exactly, while destination-
derived packaging bytes are newly audited and recorded by that build rather than
compared to the original path-dependent container digest. Cache-origin facts in an
existing decision are immutable and re-encoding or reuse cannot rewrite them. A
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
stored with private permissions. The raw salt is not written to .cktune, but its
domain-separated digest is recorded so the cache origin can be rederived. Consequently,
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
- Temporary directories and cooperative process groups/Job Objects are owned and
  cleaned by the session under Section 8.3's explicit boundary.
- Final output-set publication uses the journaled, digest-checked, primary-last
  protocol in Section 6.
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
- the measurement-independent choice identity;
- compile and measurement cache reuse;
- final replay and object-graph verification.

JSON and default text use the exact bytes and complete-tree traversal in
[`inspection-schema-1.md`](inspection-schema-1.md). Deterministic output does not
contain absolute paths, timestamps, temporary identifiers, hash-map order, or
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

The matrix is also a portability gate for the native runtime and backend. The
following are release-blocking requirements, not host-specific test exceptions:

- the profile runtime uses an explicit internal atomic abstraction; Windows uses
  supported Interlocked operations rather than assuming that MSVC C11 atomics are
  enabled, while every host preserves the same acquire/release publication model;
- Linux x86-64 and AArch64 plus Darwin x86-64 and AArch64 must durably publish and
  reopen profile shards using their platform adapter; directory, open, identity,
  write, rename, and sync failures retain distinct internal causes before mapping
  to the stable public status;
- artifact assertions derive the host filename from `NativeArtifactPaths` and may
  not hard-code a Darwin extension on Linux or Windows;
- LLVM call lowering never assigns a value name to a `void` call, and a regression
  test covers the PGO/tuning fixture that previously exercised that assertion;
- all six native-host jobs compile the profile runtime, run the exact publication
  tests, and build both executable and dynamic artifacts before a platform is
  considered supported.

The six native hosts verify:

- manifest and decision parsing;
- executable and dynamic harness protocols;
- deterministic search and replay;
- cooperative process-group/Job-Object timeout and cleanup, including hostile
  POSIX escape rejection as outside the runner contract;
- cache permissions, invalidation, corruption, traversal, and eviction;
- journal recovery, rollback/roll-forward, full output-set digest validation, and
  primary-last publication at every phase boundary;
- ordinary non-tuning behavior;
- final artifacts preserving the existing self-contained system-runtime policy.

Performance claims are made only on the stable Linux enhanced x86-64 and AArch64 workers.
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

### 19.1 Frozen schema 9 evidence contract

The sole field-by-field JSON authority is the shared normative attachment
[`performance-schema-9.md`](performance-schema-9.md). It fixes all nested keys,
types, cardinalities, statistics, identities, and fail-closed checks; this section
fixes the associated product policy and repository assets.

Schema 9 extends, and never substitutes for, two distinct schema-8 gates. The
historical accepted report named by `benches/baselines/v0_13_replay.toml` is checked
with its retained checker in a detached exact v0.13 checkout. A fresh cumulative
compatibility report reruns the entire schema-8 suite with candidate version 0.14.0
and the current candidate SHA through the checker's explicit compatibility mode;
all old thresholds remain unchanged. Rewriting the historical report or checking
it against v0.14 HEAD is forbidden. The five tune-eligible cases are exactly the
five rows of `benches/cases/pgo-cases.tsv`: `branch-layout`,
`call-constant-length`, `trip-unroll-simd`, `memory-bound`, and `compute-bound`.
There are no optional rows or post-result exclusions.

The existing `training.tsv` is search input and `held-out.tsv` is validation input.
The tuner receives both through seven fixed `benches/tune/workloads/*.cktune.toml`
manifests. It never receives the sealed release file
`benches/fixtures/tune/release-held-out.tsv`, whose exact data rows are:

    ckc-tune-inputs\t1\trelease-held-out
    branch-layout\trelease-branch-prime\t16381\t79\t3
    call-constant-length\trelease-fixed-4000\t4000\t83\t13
    trip-unroll-simd\trelease-map-4093\t4093\t89\t0
    memory-bound\trelease-zip-4096\t4096\t97\t0
    compute-bound\trelease-f64-4091\t4091\t101\t1.0009765625

`benches/cases/tune-cases.tsv` has exactly these seven logical rows after its schema
header; each row fixes the source, manifest basename, and search/validation/release
record provenance:

| Tune case | Source | Search record | Validation record | Release record |
| --- | --- | --- | --- | --- |
| branch-layout | `benches/fixtures/pgo/branch_layout.ck` | train-branch-biased | held-branch-prime | release-branch-prime |
| call-constant-length | `benches/fixtures/pgo/call_constant_length.ck` | train-fixed-4000 | held-fixed-4000 | release-fixed-4000 |
| trip-unroll-simd | `benches/oracles/fixtures/map_u32.ck` | train-map-4000 | held-map-3967 | release-map-4093 |
| memory-bound | `benches/oracles/fixtures/zip_u32.ck` | train-zip-4000 | held-zip-4000 | release-zip-4096 |
| compute-bound | `benches/fixtures/pgo/compute_bound.ck` | train-f64-4000 | held-f64-3989 | release-f64-4091 |
| contract-noalias | `benches/oracles/fixtures/contract_noalias.ck` | train-zip-4000 | held-zip-4000 | release-zip-4096 |
| contract-fixed-length | `benches/oracles/fixtures/contract_fixed_length.ck` | train-fixed-4000 | held-fixed-4000 | release-fixed-4000 |

For each row, `<tune-case>.cktune.toml` is exactly schema 1 with runner path
`../../../target/release/ckc-tune-runner`, input root `../..`, args
`["--ck-tune"]`, inputs
`["fixtures/pgo/training.tsv","fixtures/pgo/held-out.tsv"]`, empty
`inherit_env`, and `timeout_ms=30000`. It contains exactly `<tune-case>.search`
and `<tune-case>.validation`, with roles search/validation, weight 1, and the seed
from the named input record. Expected digests are not discretionary constants:
each is

    SHA-256("CK-TUNE-RESULT\0" || U32_BE(native_abi_schema) ||
            U32_BE(len(case_id_utf8)) || case_id_utf8 ||
            U64_BE(result_byte_count) || canonical_result_bytes)

and is written in `tune-cases.tsv` and the manifest. The audited CK, C, and Rust
implementations must independently produce those exact bytes before evidence
collection. The release digest uses `<tune-case>.release`; the release record and
digest are never present in a tune manifest.

The fixed recipe contains `benches/cases/tune-cases.tsv`, the seven workload
manifests, `benches/tune/runner.rs`, `benches/oracles/tune/manifest.toml`,
`benches/oracles/tune/c/tune_oracle.c`,
`benches/oracles/tune/rust/tune_oracle.rs`, the seven CK sources comprising the
five cases plus `contract_noalias.ck` and `contract_fixed_length.ck`, the four input
partitions, `benches/tune_perf.rs`, `scripts/measure-v014-performance.py`,
`scripts/check-native-performance.py`, `scripts/audit-performance-oracles.py`,
`scripts/package-v014-performance-archive.py`, `LICENSE`, and
`THIRD_PARTY_NOTICES.md`, plus `benches/baselines/v0_13_replay.toml` and the
normative `specs/0.14/performance-schema-9.md`.
The oracle manifest pins C11, Rust 2024, strict floating-point behavior, safety
preconditions, and the UB/alias audit. Any recipe-byte change invalidates evidence.

The report path is `target/ckc-perf/v0.14-results.json`; evidence is a real,
non-symlink directory named `v014-measurement-<unix-seconds>-<pid>` beside it. Its
top-level key set is exactly:

    schemaVersion, candidateVersion, candidateSha, v013ReplayCommit,
    evidenceDirectory, toolchain, hardware, recipe, candidateBinary,
    v013ReplayBundle, cumulativeSchemaEight, workload, tuningDecisions,
    tuningArtifacts, sampling, cases, validationCases, domainCases, tuneUseCompileTime,
    ordinaryCompileRegression, artifactSize, archiveSize, resourceUse,
    determinism, correctness

`schemaVersion` is 9 and `candidateVersion` is `0.14.0`. Candidate SHA, compiler
bytes, the exact v0.13 historical replay evidence closure, the fresh v0.14
schema-8 compatibility evidence closure, pinned LLVM/Clang 22.1.8, Rust 1.90.0,
the retained `/usr/bin/ld` system-linker identity, runner bytes, manifests,
source/input bytes, hardware, operating system, CPU
features, recipe, artifacts, decisions, and every retained evidence file have size
and SHA-256 identity. Every evidence-root entry must be a regular
file below the evidence directory, with no symlink, traversal, missing, duplicate,
or unknown entry; repository-root identities resolve only in the clean candidate
checkout. The stable Linux x86-64 worker requires x86-64-v4; the stable Linux AArch64 worker
requires SVE2. A missing tier is a failed gate, never workflow discretion.

Main-case timing uses `rotating-six-channel-v1` with channels in this exact order:
`tuned`, `v014Ordinary`, `v013Ordinary`, `v013Pgo`, `cSimd`, and `rustSimd`.
Validation timing uses `rotating-three-channel-v1` with `tuned`, `v013Ordinary`,
and `v013Pgo`.
Domain timing uses `rotating-three-channel-v1` with `tuned`, `genericC`, and
`genericRust`. All three protocols execute and retain receipts for three unscored warmup rows, retain twenty
measured rows, call each channel seven equal batches per sample, store the minimum,
and use the upper median. At least 16 of 20 samples must lie within inclusive
80%..120% of that median. Rotation derives from the candidate, case, split, and row
digests. Dynamic loading, symbol resolution, setup, tuning search, and harness I/O
are outside steady timing. There is no selective rerun. The internal `.cktune`
three-invocation decision evidence remains separate from these external seven-call
release samples.
Every external receipt is timed inside the retained native runner's iteration
loop. The collector starts that runner directly with an empty environment and
parses its exact `CKPERF/1` receipt; process startup, dynamic loading, input
allocation, result hashing, output, and Python/FFI loop overhead are outside the
reported elapsed time.
Every C/Rust oracle build also starts with an exactly empty environment. C oracle
argv names the independently resolved `/usr/bin/ld` through Clang `--ld-path`;
Rust oracle argv names the resolved pinned Clang driver and that same linker
through explicit `-C linker` and `-C link-arg`. Their live bytes must equal the
retained toolchain identities, so no oracle build resolves a linker through PATH.

Each case/split first records the fixed doubling calibration and one confirmation;
its selected iterations-per-call is identical for every channel, warmup receipt,
and measured receipt. Every receipt records requested/completed iterations, time,
and correctness. Validation receipts must equal the manifest expected digest;
release/domain receipts must equal the independently regenerated frozen result.

Each main case records all six raw seven-call streams and orders, their per-row
minima, medians, per-channel correctness digests,
source/input identities, selected or baseline decision, complete `.cktune` identity,
all artifact identities, eligibility bit fixed true, and release-held-out result.
Each of the seven validation cases records tuned, v0.13 ordinary, and v0.13 PGO raw
seven-call streams and per-channel correctness digests on the manifest validation
input; the checker chooses the faster v0.13
median and applies the unchanged 102/100 ceiling.
The two domain cases record the same facts for three streams plus their exact tuned
decision, output set, and three build commands. All seven `.cktune` files and
complete role-tagged published output sets are copied into evidence; their
schema, identity, certificate/baseline reason, plan, object graph, link recipe,
measurements, and on-disk digests are independently checked.

Tune-use compilation time is measured against v0.14 ordinary compilation with
three warmup pairs and fifteen measured pairs, alternating first channel by row;
ordinary v0.14 compilation is compared with exact v0.13 ordinary compilation by
the same protocol. Both use upper medians, bind every raw time to its command in a
closed `TimedCommand` receipt, measure terminated-child user-plus-system CPU time
so hosted-runner descheduling is excluded without removing compiler work, and
exclude tuning search. Artifact size uses the exact primary outputs paired with
timing. Resource evidence retains standard-session wall time, peak compiler/tuner
RSS, expansion/compile/finalist counts, and cache bytes. Determinism consists of two
independent cold-cache sessions compared by measurement-independent choice identity,
plan, object graph, link recipe, and published-content identity, plus one exact
warm-cache reuse compared by decision and output bytes as well as zero compile and
measurement counts. Genuine cold-session raw timings are preserved and may differ.
Each session retains its exact tune build command, a locked complete before/after
cache inventory, canonical event log, raw counters, wall time, peak RSS, decision,
and outputs. Cold namespaces are distinct and empty; warm begins from cold one's
exact post-cache inventory with no intervening access.
On both stable Linux performance hosts, tuned and ordinary compiler processes use
the same direct-child `wait4` supervisor; its retained receipt binds the exact
command, `CLOCK_MONOTONIC_RAW` interval, zero wait status, and kernel `ru_maxrss`
high-water value converted from KiB to bytes. Sparse polling is not accepted as a
peak-memory source.
Every timed channel carries a closed build command and an explicit foreign-key
chain from its output bytes to the tuned decision/output set or audited baseline.
All CK performance builds explicitly use `--overflow unchecked --bounds unchecked`,
and every oracle channel fixes the same defined-input semantics; no comparison
relies on a CLI default or mixes safety modes.
The canonical tuning decision/output set is the first cold determinism run; main
and validation timing, artifact-size, resource, and warm-reuse records reference
that same retained identity rather than parallel unbound copies.
Every file identity explicitly names either the candidate-SHA repository root or
the retained evidence root.

Archive size compares the replay manifest's exact v0.13 archive with a deterministic
three-member v0.14 archive containing the same candidate compiler used above plus
the repository license and notices. Its recipe-pinned producer, closed invocation,
complete member manifest, metadata, compression bytes, and static-dependency audit
are retained.

`scripts/measure-v014-performance.py` only collects raw evidence.
`scripts/check-native-performance.py` is the sole authority that accepts it and
fails closed on any missing/unknown key, identity mismatch, nonfinite or nonpositive
measurement, wrong cardinality/order, unstable stream, ineligible hardware,
decision mismatch, threshold failure, selective rerun, or unretained evidence.
`scripts/audit-performance-oracles.py` independently rechecks source/oracle/input
coverage and semantics. The two required stable-performance jobs run this complete
contract on Linux enhanced x86-64 and AArch64 hosts at the same candidate SHA.

### 19.2 Frozen thresholds

The corpus above is partitioned before measurement into search, validation, and
sealed held-out cases. Cases eligible for tuning and exclusions are declared before
results; post-measurement exclusion is forbidden.

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

On the frozen two-case domain-constraint suite, `contract_noalias.ck` and
`contract_fixed_length.ck`, tuned CK beats the faster semantically generic C or
Rust O3 result by more than 8% geometric mean. Both cases participate.

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
- two cold runs have the same measurement-independent choice identity, plan,
  object graph, link recipe, and published output content; their raw decision
  digests may differ with genuine calibration and timing evidence;
- an exact warm-cache reuse compiles and measures zero candidates and reproduces
  the first cold run's decision and role-tagged output bytes exactly;
- final artifacts contain no tuning runner, tuning symbol, runtime dispatch, or new
  runtime dependency.

The release evidence records hardware, operating system, compiler identity, raw
samples, exclusions, and exact artifact digests.

### 19.3 Predicated-update optimization-fulfilment gate

Schema 9 and its frozen corpus remain unchanged. In addition, both stable Linux
performance hosts run one independent fail-closed gate that proves the new
predicated-update capability is selected and profitable rather than merely
present. Its source is a strict-`f64` Floyd-Warshall kernel whose inner loop is the
canonical same-place conditional update. The source language, ABI, safety modes,
target, CPU policy, LLVM pipeline, and input generator are identical between
channels.
The sole field-by-field authority for this independent report, recipe, runner,
attestation, sampling, evidence closure, checker, and CI invocation is
[`predicated-update-performance-1.md`](predicated-update-performance-1.md).

The corpus is fixed before measurement: PGO generation and tuning search use a
deterministic `N=128`, seed-113 matrix; tuning validation uses a disjoint
deterministic `N=256`, seed-127 matrix; and release timing uses a sealed
deterministic `N=1024`, seed-131 matrix. The generator emits zero on the diagonal,
finite non-negative edge weights, and positive infinity for absent edges, so the
contract contains no negative cycle or NaN input.
Seeds, source bytes, generator bytes, profile, manifest, decision, artifacts,
compiler, LLVM, hardware capability, commands, correctness digests, and raw timing
receipts are retained. Training and validation inputs are never accepted as
release timing evidence.

There are exactly two release channels:

- `pgoOnly`: `ckc build`, O3, native CPU, explicit PGO-use,
  `--overflow unchecked --bounds unchecked`, and no tune-use;
- `pgoTuned`: the same build identity and PGO profile plus the decision produced by
  `ckc tune build --pgo-use`.

The decision must select a non-baseline plan with exactly one PlanChoice. That
choice resolves to a Loop SIMD UnitVariant containing exactly one SiteAlternative,
the verified predicated-update alternative; no layout, short-slice, second Loop
SIMD, or other choice is present. The source-aware checker also proves the recorded
minimum is at most 128 and that the fixed N/slice facts make every runtime legality
guard true and execute at least one vector chunk on training, validation, and
release. A baseline decision, a compound plan, a dynamically unreachable rewrite,
a stale profile/decision, or a decision that cannot be replayed exactly fails the
gate. Correctness is checked against an independent scalar oracle before any
timing is considered.
Separate positive and negative optimizer tests cover checked mode: the same
rewrite is accepted only when lane bounds, overflow, and first-failure ordering
are all proven, and otherwise remains scalar.

Timing uses three unscored warm-up rows followed by twenty measured rows, rotates
the first channel deterministically, retains every raw monotonic-clock receipt,
uses the upper median, and applies the schema-9 16-of-20 inclusive 80%..120%
stability rule independently to both streams. On each host `pgoTuned/pgoOnly` must
be at most `95/100`; no validation case may exceed `102/100`. Failure, instability,
missing evidence, or a post-result exclusion blocks release. This gate is
additional to every schema-8/schema-9, resource, size, determinism, and ten-job CI
requirement and cannot be traded against them.

## 20. Release gate

CK 0.14 is releasable only when:

1. the final accepted v0.13 base is integrated and all carried gates remain green;
2. every normative behavior in this specification has positive and negative tests;
3. local total acceptance passes from a clean checkout;
4. all ten exact-SHA remote jobs pass;
5. the frozen schema-9 corpus and the independent predicated-update corpus meet
   every threshold in Section 19;
6. documentation, CLI help, examples, schemas, and inspection output agree;
7. produced executables and dynamic libraries retain the promised zero-dependency
   deployment model;
8. the repository is clean and all release evidence is retained by the exact-SHA
   CI run and release archive; generated evidence is not committed into the source
   commit whose SHA it records.

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
