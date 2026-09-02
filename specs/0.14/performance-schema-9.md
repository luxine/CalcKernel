# CK 0.14 Performance Report Schema 9

Status: normative, language-neutral release-evidence contract

This document is shared by the English and Simplified Chinese CK 0.14 designs.
Every JSON object rejects missing, duplicate, or unknown keys. JSON is UTF-8 with
lexicographically sorted keys, no insignificant whitespace, and one trailing LF.
Integers are nonnegative JSON integers within `u64` unless a smaller range is
stated. Timing values are positive nanoseconds. A `Digest` is exactly 64 lowercase
hexadecimal SHA-256 characters. A `FileIdentity` has exactly `root`, `path`,
`bytes`, and `sha256`. `root` is exactly `repository` or `evidence`; `path` is
relative to that one named root, never absolute or traversing, and resolves to a
regular non-symlink file. The repository root is the clean candidate-SHA checkout;
the evidence root is the real directory named by `evidenceDirectory`. Root order is
repository then evidence, and every “path-sorted” list sorts by `(root,path)` UTF-8
bytes. There is no fallback from one root to the other.

Every digest introduced by this attachment uses the following unambiguous typed
encoding. `U8/U32/U64` are fixed-width big-endian; `Text` is `U32 length` followed
by UTF-8 bytes; `DigestBytes` is the 32 bytes decoded from a `Digest`; a list is
`U32 count` followed by its elements; and a `FileIdentityValue` is root `U8`
(repository=1, evidence=2), path `Text`, bytes `U64`, then `DigestBytes`. A named record encodes its fields in the order
stated here. Define:

    P(domain, value...) = SHA-256(ASCII domain including its final NUL ||
                                  typed encoding of each value)

No digest in this attachment is a hash of informal concatenation or an
implementation-dependent JSON serialization.

## 1. Closed top level

The exact top-level keys are:

    schemaVersion, candidateVersion, candidateSha, v013ReplayCommit,
    evidenceDirectory, toolchain, hardware, recipe, candidateBinary,
    v013ReplayBundle, cumulativeSchemaEight, workload, tuningDecisions,
    tuningArtifacts, sampling, cases, validationCases, domainCases, tuneUseCompileTime,
    ordinaryCompileRegression, artifactSize, archiveSize, resourceUse,
    determinism, correctness

Fixed scalar values are `schemaVersion=9`, `candidateVersion="0.14.0"`, and
`candidateSha` equal to `git rev-parse HEAD`. `v013ReplayCommit` equals the commit
in `benches/baselines/v0_13_replay.toml`. `evidenceDirectory` matches
`v014-measurement-[0-9]+-[0-9]+` and names the real sibling directory.

## 2. Identity objects

`toolchain` has exactly `llvmVersion`, `clangVersion`, `rustVersion`,
`componentManifest`, `clangBinary`, and `clangProfileRuntime`. Versions are exactly
`22.1.8`, `22.1.8`, and `1.90.0`; the last three values are `FileIdentity` objects.
Before
report creation the collector copies the source component manifest to the fixed
evidence path `toolchain/llvm-build.toml`, the resolved regular Clang executable to
`toolchain/clang.bin`, and the discovered Clang profile runtime archive to
`toolchain/clang-profile-runtime.bin`; those are the paths recorded in the three
identities, all with `root="evidence"`. The checker hashes all three retained files,
requires the manifest to describe the pinned LLVM component build, and requires the
resolved `CKC_CLANG_ORACLE` executable to equal retained `clangBinary` and report
22.1.8. It runs that original executable with the sole argument
`--print-resource-dir`. Below the returned real directory it sorts regular
non-symlink matches of
`lib/**/libclang_rt.profile*.a` followed by matches of
`lib/**/clang_rt.profile*.lib`; exactly one total match is required, and the
retained runtime must equal it byte-for-byte.

`hardware` has exactly `target`, `arch`, `os`, `osBuild`, `kernel`, `cpuModel`,
`logicalCpus`, `physicalCpus`, `numaNodes`, `features`, `requiredTier`,
`availableTiers`, `osState`, and `capabilityDigest`. Feature and tier lists are
sorted and unique. `capabilityDigest` is
`P("CK-V014-PERF-HARDWARE\0", target Text, arch Text, os Text, osBuild Text,
kernel Text, cpuModel Text, logicalCpus U32, physicalCpus U32, numaNodes U32,
features List<Text>, requiredTier Text, availableTiers List<Text>, osState Text)`.
The x86-64 job requires
`requiredTier="x86-64-v4"`; the AArch64 job requires
`requiredTier="aarch64-sve2"`. That tier must occur in `availableTiers` and all of
its required features in `features`. Missing required hardware fails instead of
skipping.

`recipe` has exactly `schema`, `files`, `digest`, and `thresholds`. `schema=1`.
`files` is a path-sorted list of `FileIdentity` covering every path named in Section
19.1 of the design, all with `root="repository"`. `digest` is
`P("CK-V014-PERF-RECIPE\0", schema U32, files List<FileIdentityValue>,
thresholds List<ThresholdEntry>)`; `ThresholdEntry` is key `Text` then value `U64`,
and all threshold keys are encoded in lexicographic UTF-8 order. `thresholds` has
exactly:

| Key | Value |
| --- | ---: |
| `heldOutGeomeanMaximumNum/Den` | 95 / 100 |
| `selectedCaseMaximumNum/Den` | 98 / 100 |
| `validationOrHeldOutMaximumNum/Den` | 102 / 100 |
| `oracleGeomeanThroughputMinimumNum/Den` | 98 / 100 |
| `oracleCaseThroughputMinimumNum/Den` | 92 / 100 |
| `domainThroughputMinimumNum/Den` | 108 / 100, strict |
| `artifactMaximumNum/Den` | 110 / 100 |
| `tuneUseCompileGeomeanMaximumNum/Den` | 110 / 100 |
| `tuneUseCompileCaseMaximumNum/Den` | 120 / 100 |
| `ordinaryCompileGeomeanMaximumNum/Den` | 103 / 100 |
| `ordinaryCompileCaseMaximumNum/Den` | 108 / 100 |
| `archiveMaximumNum/Den` | 110 / 100 |
| `standardWallMsMaximum` | 1,800,000 |
| `peakRssMaximumNum/Den` | 2 / 1 |
| `cacheBytesMaximum` | 4,294,967,296 |

Each slash pair is represented by two separate integer keys in JSON. Geometric
ratio gates use arbitrary-precision product comparison, not floating point:
`geomean(A/B) <= p/q` iff `product(A_i*q) <= product(B_i*p)`. Throughput minimum
`perf(A) >= p/q * perf(B)` uses `product(B_ns*q) >= product(A_ns*p)`; the domain
gate uses strict `>`.

`candidateBinary` and `cumulativeSchemaEight` are evidence-root `FileIdentity`
objects. The latter
is exactly `results-schema8.json` and must independently pass the schema-8 checker.

`v013ReplayBundle` has exactly `commit`, `manifest`, `compiler`, `archive`, and
`schemaEight`; the last four are evidence-root `FileIdentity`. Its commit and all identities must
equal `benches/baselines/v0_13_replay.toml`, and its schema-8 file independently
passes before any schema-9 threshold is evaluated.

## 3. Workload and sampling

`workload` has exactly `casesManifest`, `sources`, `search`, `validation`,
`adversarial`, `releaseHeldOut`, `tuneManifests`, `runner`, `oracleManifest`,
`cOracle`, and `rustOracle`. Scalar file members are `FileIdentity`; `sources` and
`tuneManifests` are path-sorted `FileIdentity` lists of exactly seven and seven
entries. Their file set and logical rows equal Section 19.1 exactly.
Repository source, manifest, oracle, and input members use root `repository`; the
built runner uses root `evidence`. Every timed artifact, decision, compiler binary,
replay artifact, raw-sample file, and toolchain copy uses root `evidence`.

`sampling` has exactly `mainProtocol`, `validationProtocol`, `domainProtocol`,
`mainChannels`, `validationChannels`, `domainChannels`, `warmupRows`, `sampleRows`, `callsPerSample`, `statistic`,
`stabilityPolicy`, and `rerunPolicy`. Values are exactly:

- `mainProtocol="rotating-six-channel-v1"`;
- `validationProtocol="rotating-three-channel-v1"`;
- `domainProtocol="rotating-three-channel-v1"`;
- main channels `[tuned,v014Ordinary,v013Ordinary,v013Pgo,cSimd,rustSimd]`;
- validation channels `[tuned,v013Ordinary,v013Pgo]`;
- domain channels `[tuned,genericC,genericRust]`;
- warmups 3, samples 20, calls 7, statistic `minimum-then-upper-median`;
- stability `at-least-80-percent-within-20-percent-of-upper-median`;
- rerun policy `unstable-evidence-is-invalid-no-selective-rerun`.

Every steady-state order uses this exact byte formula:

    key = SHA-256("CK-V014-PERF-ORDER\0" ||
                  U32_BE(len(candidate_sha_ascii)) || candidate_sha_ascii ||
                  U32_BE(len(protocol_utf8)) || protocol_utf8 ||
                  U32_BE(len(split_utf8)) || split_utf8 ||
                  U32_BE(len(case_utf8)) || case_utf8 ||
                  phase_u8 || row_u32_be)

Phase is 1 for warmup and 2 for measured rows; row is zero-based within its phase.
The first eight digest bytes are a big-endian u64 and select a left rotation modulo
channel count from the fixed channel list. `split` is `release-held-out`,
`validation`, or `domain-release-held-out`. The stored order must equal this formula.

## 4. Decisions and artifacts

`tuningDecisions` is a case-name-sorted list of exactly seven objects with keys
`case`, `file`, `decisionDigest`, `choiceIdentityDigest`, `selectionReason`,
`planDigest`,
`objectGraphDigest`, `linkRecipeDigest`, `certificateDigest`, and `outputRecords`.
`file` is a `FileIdentity`; six digest fields are `Digest`, except
`certificateDigest` is either a `Digest` for `tuned` or JSON null for a baseline
reason. `outputRecords` is a role-sorted list with exact keys `role`, `logicalName`,
`bytes`, and `sha256`. Every extracted scalar and output record must equal the
decoded retained decision; none is trusted as an independent report assertion.

`tuningArtifacts` is a case-name-sorted list of exactly seven objects with keys
`case`, `decision`, and `outputs`; `decision` is a `FileIdentity`, and `outputs` is
the complete role-sorted list of one, two, or three `OutputArtifact` objects. An
`OutputArtifact` has exactly `role` and `file`; role is `primary`, `header`, or
`import-library`, and file is a `FileIdentity`. Every role/digest/size identity
equals `tuningDecisions.outputRecords`; logical name equals the output file's
basename. `tuningArtifacts.decision` equals `tuningDecisions.file` for the same
case, including root and path.

## 5. Steady-state cases

`cases` is a case-name-sorted list of exactly five `MainCase` objects.
`validationCases` is a case-name-sorted list of exactly seven `ValidationCase`
objects, one for every tuning case. `domainCases` is a case-name-sorted list of
exactly two `DomainCase` objects for `contract-fixed-length` and
`contract-noalias`.

A `MainCase` has exactly `case`, `eligible`, `source`, `input`, `decisionDigest`,
`correctnessDigest`, `correctnessDigests`, `artifacts`, `buildCommands`,
`warmupOrder`, `sampleOrder`, `callsNs`, `samplesNs`, and `mediansNs`. `eligible`
is true. `source` and `input` are `FileIdentity`;
`artifacts` has exactly the six main-channel `FileIdentity` keys. `warmupOrder` is
three channel permutations and `sampleOrder` is twenty channel permutations; each
row contains every channel exactly once and equals the specified digest rotation.
`callsNs` has exactly six channel keys, each containing 20 rows of exactly seven
positive integers in invocation order. `samplesNs` has exactly six channel keys,
each a list of 20 positive integers equal to the row minima in `callsNs`;
`mediansNs` has those keys and equals each list's ascending element 10. Every stream
passes the 16-of-20 inclusive 80%..120% stability rule. `correctnessDigests` has
exactly the six channel keys and every value equals `correctnessDigest`.
`buildCommands` has the six main-channel keys and `BuildCommand` values from
Section 6.

A `ValidationCase` has exactly `case`, `source`, `input`, `decisionDigest`,
`correctnessDigest`, `correctnessDigests`, `artifacts`, `buildCommands`,
`warmupOrder`, `sampleOrder`, `callsNs`, `samplesNs`, and `mediansNs`.
Channel-shaped values use the three validation
channels and otherwise obey the same 3/20/7 rotation, upper-median, and stability
rules. Its input is the case's manifest validation input, not release held-out.

A `DomainCase` has exactly the same keys as `ValidationCase`, but all channel-shaped
objects and orders use the three domain channels and the input is the domain
release-held-out input. `buildCommands` has exactly the three domain-channel keys
and closed `BuildCommand` values from Section 6. In both record types,
`correctnessDigests` has exactly the applicable channel keys, all values equal
`correctnessDigest`, `callsNs` contains every seven-call row, and `samplesNs`
contains exactly their minima.

For every MainCase, ValidationCase, and DomainCase, `decisionDigest` equals its
same-name `tuningDecisions` row; `artifacts.tuned` is the primary file in the
same-name `tuningArtifacts` row; every `buildCommands[channel].outputs` contains
exactly the published output set for that channel; and its primary file equals
`artifacts[channel]`. The tuned BuildCommand's complete output list equals
`tuningArtifacts.outputs`, and its decision equals `tuningArtifacts.decision`;
every other channel has a null decision. Every source/input identity occurs in both the build
command inputs and the corresponding workload identity. These are mandatory
foreign-key equalities, not collector conventions.

For each of the seven case names, `tuningArtifacts.decision` and `.outputs` equal
the `decision` and `outputs` of that case's `determinism.coldOne` record. The five
main rows, all seven validation rows, and both domain rows therefore time that
canonical first-cold output set through the foreign keys above.

The main gates use release-held-out rows only. For every selected tuned case,
`tuned/v013-faster` is at most 98/100; every case, including baseline selections,
enters the five-case held-out geometric gate of at most 95/100; no validation or
release-held-out ratio exceeds 102/100. Every validation case compares tuned time
with the lower median of v0.13 ordinary and v0.13 PGO and is at most 102/100 of
that faster comparator. Oracle throughput meets 98/100 geometric
and 92/100 per case. The two domain cases jointly satisfy the strict 108/100
throughput gate against the faster generic oracle.

## 6. Compilation, size, and resource records

`tuneUseCompileTime` is a case-name-sorted seven-row list comparing `tuneUse` with
`v014Ordinary`. `ordinaryCompileRegression` is the same shape comparing
`v014Ordinary` with `v013Ordinary`. Each row has exactly `case`, `warmupOrder`,
`sampleOrder`, `samplesNs`, `mediansNs`, and `commands`. Orders contain three and
fifteen two-channel permutations with alternating first channel. Each samples list
has 15 positive values, each median is ascending element 7. `commands` is an object
with exactly the two channel keys; each value is a `Command` with exactly `argv`,
`workingDirectory`, `executable`, `inputs`, `environment`, and
`environmentDigest`, where `argv` is the exact string vector,
`workingDirectory` is exactly `repository`, argv contains no absolute or traversing
path, and the process runs at the clean candidate-SHA repository root. Relative
output arguments name locations below the evidence directory;
`executable` is a `FileIdentity`, `inputs` is a path-sorted `FileIdentity` list, and
`environment` is a name-sorted list of closed `EnvironmentEntry` objects. An entry
has exactly `name`, `value`, and `references`. `value` is the exact `Text` passed to
the child process; `references` is a path-sorted list of already-retained
`FileIdentity` objects that give any path-valued variable its semantic identity.
Names are
unique and are drawn only from `CKC_LLVM_PREFIX`,
`CKC_CLANG_ORACLE`, `CKC_CANDIDATE_COMPILER`, `CKC_V013_REPLAY_BUNDLE`,
`SystemRoot`, and `WINDIR`; absent names are omitted and every other inherited
variable is cleared. Windows requires the last two entries with empty references.
The four `CKC_*` names require nonempty references. Their exact mappings are: LLVM
prefix to the three retained toolchain files; Clang
oracle to `toolchain.clangBinary`; candidate compiler to `candidateBinary`; and a
replay bundle to that bundle's manifest, compiler, archive, and accepted-report
identities. A variable with no corresponding retained reference is invalid.
The checker resolves each path-valued `value`, verifies that it denotes the location
described by its references, and rejects an unused, missing, or extra reference.
`environmentDigest` is
`P("CK-V014-PERF-COMMAND-ENV\0", environment List<EnvironmentEntryValue>)`,
where an entry value is name `Text`, value `Text`, then references
`List<FileIdentityValue>`.

A `BuildCommand` has exactly `command`, `decision`, and `outputs`. `command` is the
closed `Command` above. `decision` is the generated tuning-decision `FileIdentity`
or JSON null for a nontuned channel. `outputs` is the complete role-sorted list of
`OutputArtifact` objects produced by that invocation. An argv decision/output path
and every generated sidecar must resolve to exactly those file identities; an
unlisted generated file or an output without command provenance is invalid
evidence. All MainCase,
ValidationCase, and DomainCase `buildCommands` contain `BuildCommand`, while the
compile-time `commands` contain `Command` because their per-iteration outputs are
discarded and are not timed artifacts.

The first channel of compile warmup row 0 is the left-listed channel and alternates
across all 18 rows without restarting at the measured boundary; the report retains
the separated 3/15 orders. `artifactSize` is a case-name-sorted seven-row list with
exactly `case`, `tunedPrimary`, `baselinePrimary`, and `baselineBuild`.
`tunedPrimary` and `baselinePrimary` are `FileIdentity`; `baselineBuild` is a
nontuned `BuildCommand` whose primary output equals `baselinePrimary` and whose
decision is null. `tunedPrimary` equals the same-name `tuningArtifacts` primary.
For the five main cases, `baselinePrimary` also equals
`cases.artifacts.v014Ordinary`. The size ratio is at most 110/100.
`archiveSize` has exactly `candidate` and `v013Replay`,
both `FileIdentity`, with ratio at most 110/100.

`resourceUse` has exactly `sessions` and `cacheHardLimitBytes`. The hard limit is
4,294,967,296. `sessions` is a seven-row case-name-sorted list with exactly `case`,
`decision`, `decisionDigest`, `budget`, `wallMs`, `peakRssBytes`,
`ordinaryPeakRssBytes`, `expansions`,
`compileAttempts`, `measuredFinalists`, `validationEntrants`, and `cacheBytes`.
`decision` and `decisionDigest` equal the same-name `tuningDecisions` row. Budget is
`standard`; all counts fit its preset; wall, RSS ratio, and cache meet the frozen
thresholds.

## 7. Determinism and correctness

`determinism` is a case-name-sorted list of seven objects with exactly `case`,
`coldOne`, `coldTwo`, and `warm`. Each run has exactly `decision`, `outputs`,
`decisionDigest`, `choiceIdentityDigest`, `planDigest`, `objectGraphDigest`,
`linkRecipeDigest`, `outputContentDigest`, `compiledCandidates`, and
`measuredCandidates`. `decision` is a retained `FileIdentity`; `outputs` is the
complete role-sorted `OutputArtifact` list defined for `tuningArtifacts`. The decision
digest and
decoded fields must match the retained decision. `outputContentDigest` is
`P("CK-V014-PERF-OUTPUT-CONTENT\0", outputs List<OutputContentValue>)`, where each
value is role `Text` (exactly `primary`, `header`, or `import-library`), followed
by the artifact file's bytes `U64` and decoded SHA-256 `DigestBytes`; roots, paths,
and the
measurement-bearing decision file are deliberately excluded.

The two independent cold-cache runs must match `choiceIdentityDigest`, plan,
object-graph, link-recipe, and output-content digests. Each cold decision must be
internally valid, but their `decisionDigest` values need not match because genuine
calibration and raw measurement records are part of the decision. The warm run is
an exact reuse of `coldOne`: it matches all six digests including `decisionDigest`,
its retained decision and output files are byte-for-byte equal by role, it has zero
compiled/measured candidates, and immutable origin facts do not change.

`correctness` has exactly `search`, `validation`, `adversarial`,
`validationDifferential`, `releaseHeldOutDifferential`, `domainDifferential`,
`oracleUbAudit`, `aliasAudit`, and `featureAudit`, all boolean true.

The checker does not trust these booleans. It derives search correctness from each
decoded decision, and validation/release/domain differential correctness from the
per-channel digests above. It then executes the recipe-pinned
`scripts/audit-performance-oracles.py` directly, without a shell, from the clean
candidate root against this report and its retained files; that audit must
independently exercise the adversarial inputs and establish `adversarial`,
`oracleUbAudit`, `aliasAudit`, and `featureAudit`. The command uses the same cleared
environment and retained toolchain references defined in Section 6. A nonzero exit,
unretained input, or disagreement with any true field invalidates the report.

The collector only writes evidence. The independent checker reopens and hashes
every retained file, validates every closed object and cardinality above, replays
all integer statistics and rotations, rechecks schema 8, and evaluates every gate.
It rejects nonfinite values, JSON floats for integer fields, duplicate keys,
unknown files, symlinks, identity mismatches, and any evidence not reproducible from
the retained raw records.
