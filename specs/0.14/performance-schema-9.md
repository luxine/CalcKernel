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
`componentManifest`, `clangBinary`, `clangProfileRuntime`, and `rustCompiler`.
Versions are exactly `22.1.8`, `22.1.8`, and `1.90.0`; the last four values are
`FileIdentity` objects.
Before
report creation the collector copies the source component manifest to the fixed
evidence path `toolchain/llvm-build.toml`, the resolved regular Clang executable to
`toolchain/clang.bin`, and the discovered Clang profile runtime archive to
`toolchain/clang-profile-runtime.bin`; those are the paths recorded in the three
identities, all with `root="evidence"`. It also copies the resolved Rust compiler
to `toolchain/rustc.bin`. The checker hashes all four retained files,
requires the manifest to describe the pinned LLVM component build, and requires the
resolved `CKC_CLANG_ORACLE` executable to equal retained `clangBinary` and report
22.1.8. It runs that original executable with the sole argument
`--print-resource-dir`. Below the returned real directory it sorts regular
non-symlink matches of
`lib/**/libclang_rt.profile*.a` followed by matches of
`lib/**/clang_rt.profile*.lib`; exactly one total match is required, and the
retained runtime must equal it byte-for-byte.
The resolved Rust executable must equal retained `rustCompiler` and report 1.90.0.

`hardware` has exactly `target`, `arch`, `os`, `osBuild`, `kernel`, `cpuModel`,
`logicalCpus`, `physicalCpus`, `numaNodes`, `features`, `requiredTier`,
`availableTiers`, `osState`, and `capabilityDigest`. Feature and tier lists are
sorted and unique. `capabilityDigest` is
`P("CK-V014-PERF-HARDWARE\0", target Text, arch Text, os Text, osBuild Text,
kernel Text, cpuModel Text, logicalCpus U32, physicalCpus U32, numaNodes U32,
features List<Text>, requiredTier Text, availableTiers List<Text>, osState Text)`.
Both stable performance jobs require `os="linux"`. The x86-64 job requires
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

`candidateBinary` is an evidence-root `FileIdentity`. `cumulativeSchemaEight` has
exactly `report` and `files`; `report` is the evidence-root identity named
`results-schema8-v014-compat.json`, and `files` is a path-sorted list containing
that report and every regular file transitively referenced by it. The list is the
complete contents of its dedicated evidence prefix, with no symlink or extra file.
This is a fresh cumulative v0.13-suite run produced by the v0.14 candidate. The
current checker invokes `scripts/check-native-performance.py --schema 8
--compat-candidate 0.14.0 --candidate-sha <candidateSha>` against `report`; this
mode preserves every schema-8/schema-7 threshold and shape while replacing each
hard-coded current-candidate version/SHA expectation, including the embedded
cumulative report, with the supplied v0.14 identity. Historical baseline/replay
identities remain frozen. It is not historical replay evidence.

`v013ReplayBundle` has exactly `commit`, `manifest`, `compiler`, `archive`,
`schemaEight`, `checker`, and `evidenceFiles`; the five singular file fields are
evidence-root `FileIdentity`, and `evidenceFiles` is a path-sorted list containing
them plus every regular file transitively referenced by `schemaEight`. The list is
the complete contents of one dedicated replay prefix, so an omitted, extra,
duplicate, or symlink entry is invalid. Its commit and all identities must equal
`benches/baselines/v0_13_replay.toml`. In a detached clean checkout of that exact
commit, the retained checker is byte-equal to `scripts/check-native-performance.py`
there and the checkout copy accepts the retained historical report with its
recorded `candidateVersion=0.13.0`, SHA equal to that checkout, and evidence root
reconstructed at the report's recorded relative location. Historical acceptance
completes before the separate v0.14 compatibility run or any schema-9 threshold is
evaluated.

## 3. Workload and sampling

`workload` has exactly `casesManifest`, `sources`, `search`, `validation`,
`adversarial`, `releaseHeldOut`, `tuneManifests`, `runner`, `oracleManifest`,
`cOracle`, `rustOracle`, `profiles`, and `expectedResults`. Scalar file members are
`FileIdentity`; `sources` and `tuneManifests` are path-sorted `FileIdentity` lists
of exactly seven and seven entries. Their file set and logical rows equal Section
19.1 exactly.
Repository source, manifest, oracle, and input members use root `repository`; the
built runner uses root `evidence`. Every timed artifact, decision, compiler binary,
replay artifact, raw-sample file, and toolchain copy uses root `evidence`.

`profiles` is a case-name-sorted seven-row list. Each row has exactly `case`,
`file`, `compilerSource`, `source`, and `trainingInput`; `file` is the retained
v0.13 `.ckprof` `FileIdentity`, `compilerSource` is the digest reported by the
v0.13 replay compiler, and the other fields equal the corresponding workload
source and search input. The checker decodes each profile and verifies its schema,
compiler/source/topology/content identity before any v013Pgo build.

`expectedResults` is a case-name-sorted seven-row list with exactly `case`, `split`,
`input`, `canonicalBytes`, and `digest`. `split` is `release-held-out`; `input`
equals `workload.releaseHeldOut`; `canonicalBytes` is an evidence-root
`FileIdentity` containing the exact result bytes for that case and input. `digest`
equals `SHA-256("CK-TUNE-RESULT\0" || U32_BE(native_abi_schema) ||
U32_BE(len(case_id_utf8)) || case_id_utf8 || U64_BE(canonicalBytes.bytes) ||
canonicalBytes contents)`. The checker independently regenerates those bytes with
the audited CK, C, and Rust implementations and requires all results to match.

`sampling` has exactly `mainProtocol`, `validationProtocol`, `domainProtocol`,
`mainChannels`, `validationChannels`, `domainChannels`, `warmupRows`, `sampleRows`, `callsPerSample`, `statistic`,
`stabilityPolicy`, and `rerunPolicy`. Values are exactly:

- `mainProtocol="rotating-six-channel-v1"`;
- `validationProtocol="rotating-three-channel-v1"`;
- `domainProtocol="rotating-three-channel-v1"`;
- main channels `[tuned,v014Ordinary,v013Ordinary,v013Pgo,cSimd,rustSimd]`;
- validation channels `[tuned,v013Ordinary,v013Pgo]`;
- domain channels `[tuned,genericC,genericRust]`;
- warmups 3, samples 20, calls 7, statistic `minimum-then-upper-median`; warmup
  receipts are retained but excluded from the statistic;
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

An `ExternalCalibration` has exactly `channel`, `attempts`,
`selectedIterationsPerCall`, and `confirmation`. The calibration channel is
`v014Ordinary` for main, `v013Ordinary` for validation, and `genericC` for domain.
Starting at one iteration, it records one through 32 `CalibrationAttempt` objects,
each with exactly `iterations`, `elapsedNs`, `completed`, and `correctnessDigest`,
doubling with checked `u64` arithmetic until the first elapsed value at least
50,000,000 ns; every `completed` equals `iterations`.
`selectedIterationsPerCall` equals that first qualifying attempt's iterations and
`confirmation` is one `CallReceipt` at the same iterations. A `CallReceipt` has
exactly `elapsedNs`, `iterations`, `completed`, and `correctnessDigest`; elapsed is
positive and completed equals iterations. Calibration/confirmation happens before
ordered warmup and is not included in a sample.

A `MainCase` has exactly `case`, `eligible`, `source`, `input`, `decisionDigest`,
`correctnessDigest`, `correctnessDigests`, `artifacts`, `buildCommands`,
`calibration`, `warmupOrder`, `sampleOrder`, `warmupReceipts`, `callReceipts`,
`callsNs`, `samplesNs`, and `mediansNs`. `eligible`
is true. `source` and `input` are `FileIdentity`;
`artifacts` has exactly the six main-channel `FileIdentity` keys. `warmupOrder` is
three channel permutations and `sampleOrder` is twenty channel permutations; each
row contains every channel exactly once and equals the specified digest rotation.
`callsNs` has exactly six channel keys, each containing 20 rows of exactly seven
positive integers in invocation order. `warmupReceipts` has those keys and three
rows of seven receipts; `callReceipts` has those keys and 20 rows of seven receipts.
Every receipt uses `calibration.selectedIterationsPerCall`; each measured receipt's
elapsed value equals the same-position `callsNs` value. `samplesNs` has exactly six channel keys,
each a list of 20 positive integers equal to the row minima in `callsNs`;
`mediansNs` has those keys and equals each list's ascending element 10. Every stream
passes the 16-of-20 inclusive 80%..120% stability rule. `correctnessDigests` has
exactly the six channel keys and every value equals `correctnessDigest`.
`buildCommands` has the six main-channel keys and `BuildCommand` values from
Section 6.

A `ValidationCase` has exactly `case`, `source`, `input`, `decisionDigest`,
`correctnessDigest`, `correctnessDigests`, `artifacts`, `buildCommands`,
`calibration`, `warmupOrder`, `sampleOrder`, `warmupReceipts`, `callReceipts`,
`callsNs`, `samplesNs`, and `mediansNs`.
Channel-shaped values use the three validation
channels and otherwise obey the same 3/20/7 rotation, upper-median, and stability
rules. Its input is the case's manifest validation input, not release held-out.

A `DomainCase` has exactly the same keys as `ValidationCase`, but all channel-shaped
objects and orders use the three domain channels and the input is the domain
release-held-out input. `buildCommands` has exactly the three domain-channel keys
and closed `BuildCommand` values from Section 6. In both record types,
`correctnessDigests` has exactly the applicable channel keys, all values equal
`correctnessDigest`, `callsNs` contains every seven-call row, and `samplesNs`
contains exactly their minima. Their calibration and receipt shapes use the
applicable three-channel sets under the same rules.

For a ValidationCase, every calibration/receipt/summary correctness digest equals
the expected digest of the same case's decoded manifest validation record. For a
MainCase or DomainCase it equals the same-name `workload.expectedResults.digest`;
the case id used in that derivation is `<case>.release`. Equality among channels is
therefore insufficient unless it also reaches the independently checked expected
result.

For every MainCase, ValidationCase, and DomainCase, `decisionDigest` equals its
same-name `tuningDecisions` row; `artifacts.tuned` is the primary file in the
same-name `tuningArtifacts` row; every `buildCommands[channel].outputs` contains
exactly the published output set for that channel; and its primary file equals
`artifacts[channel]`. The tuned BuildCommand's complete output list equals
`tuningArtifacts.outputs`, and its decision equals `tuningArtifacts.decision`;
every other channel has a null decision. Every source identity occurs in the
corresponding build-command inputs and workload identity. Each runtime input occurs
in the case record, the recipe's frozen input set, and the retained harness-call
stream; it is not falsely treated as a compiler input. These are mandatory
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
with exactly the two channel keys; each value is a list of exactly 18 `Command`
objects, the first three corresponding to warmup occurrences and the remaining 15
to measured occurrences in the retained order. A `Command` has exactly `argv`,
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
`XDG_CACHE_HOME`, `HOME`, `LOCALAPPDATA`, `SystemRoot`, and `WINDIR`; absent names
are omitted and every other inherited variable is cleared. Windows requires the
last two entries with empty references.
The four `CKC_*` names require nonempty references. Their exact mappings are: LLVM
prefix to the three retained toolchain files; Clang
oracle to `toolchain.clangBinary`; candidate compiler to `candidateBinary`; and a
replay bundle to that bundle's manifest, compiler, archive, and accepted-report
identities. A `CKC_*` variable with no corresponding retained reference is invalid.
Exactly one platform cache-base variable is present for every CK compiler command:
Linux uses `XDG_CACHE_HOME`, Darwin uses `HOME`, and Windows uses `LOCALAPPDATA`.
Its value is a recipe-owned directory below the evidence root and it has empty
references. Values are unique between cold and compile-time commands; only the
declared warm run reuses cold one's value. The actual `CacheSnapshot.namespace` is the compiler's platform
mapping of that base: append `ckc` on Linux, `Library/Caches/ckc` on Darwin, or
`CalcKernel/cache` on Windows. Oracle commands contain none of those three names.
The checker resolves each path-valued `value`, verifies that it denotes the location
described by its references, and rejects an unused, missing, or extra reference.
`environmentDigest` is
`P("CK-V014-PERF-COMMAND-ENV\0", environment List<EnvironmentEntryValue>)`,
where an entry value is name `Text`, value `Text`, then references
`List<FileIdentityValue>`. Its derived `commandDigest` is
`P("CK-V014-PERF-COMMAND\0", argv List<Text>, workingDirectory Text,
executable FileIdentityValue, inputs List<FileIdentityValue>, environmentDigest
DigestBytes)`; it is not an additional JSON key.

Every compile-time command uses a distinct create-new output and an initially empty
distinct cache namespace; setup and cleanup occur outside the timed interval. The
`tuneUse` template is the v0.14 ordinary template plus `--tune-use` and the
same-name canonical decision. Cache isolation, rather than a version-specific CLI
flag, proves that the measurement covers compilation instead of a native-object
hit. The v0.13 channel uses its replay compiler, its ordinary template, and the
same cold-output/cache rule. The cache-root environment entry and the output/cache
paths are different in every invocation; the checker verifies an empty locked
namespace immediately before it starts the child.
The measured `samplesNs[channel][i]` is the elapsed time of `commands[channel][i+3]`;
warmup elapsed times are not report fields. Duplicate command output/cache paths,
missing decision inputs, or a command count/order inconsistent with the two stored
permutation matrices is invalid.

Build channels are closed. In the table, `ckc` denotes argv element zero and must
resolve to the `executable` identity in the same row. After substituting
repository/evidence-relative paths, the checker requires these semantic argv
templates and no extra optimization flag:

| Channel | Executable and required mode |
| --- | --- |
| `tuned` | `candidateBinary`; `ckc tune build <source> --config <manifest> --out <primary> --kind dynamic --cpu native -O3 --overflow unchecked --bounds unchecked --budget standard --tune-out <decision>` |
| `v014Ordinary` | `candidateBinary`; `ckc build <source> --out <primary> --kind dynamic --cpu native -O3 --overflow unchecked --bounds unchecked` |
| `v013Ordinary` | `v013ReplayBundle.compiler`; the same ordinary build template |
| `v013Pgo` | `v013ReplayBundle.compiler`; ordinary template plus `--pgo-use <case-profile>` |
| `cSimd` | `toolchain.clangBinary`; the exact C11 native explicit-SIMD template in `oracleManifest` |
| `rustSimd` | `toolchain.rustCompiler`; the exact Rust-2024 native explicit-SIMD template in `oracleManifest` |
| `genericC` | `toolchain.clangBinary`; the manifest's generic C11 O3 template with no CK domain assumption |
| `genericRust` | `toolchain.rustCompiler`; the manifest's generic Rust-2024 O3 template with no CK domain assumption |

The two explicit CK modes are part of every CK template, including tune-use and
v0.13 PGO. Oracle manifests encode the same defined-input behavior and preconditions;
checked and unchecked results are never mixed in one comparison.

Validation always uses the same tuned artifact as its same-name MainCase or
DomainCase and never rebuilds it with validation-dependent flags. For the five main
cases, validation also reuses that MainCase's `v013Ordinary` and `v013Pgo`
artifacts. For the two domain-only cases, those two validation baselines are built
once by the ValidationCase's retained commands; the DomainCase separately uses its
generic C/Rust artifacts. Every CK command's source, manifest, decision, profile,
output, and cache argument occurs
in `Command.inputs`, `BuildCommand.decision`, or `BuildCommand.outputs` with the
same root/path identity. Oracle argv must be the unique template selected by
`(split, channel, target)` from the retained closed oracle manifest. Every output
is absent before its build. Every cold or compile-time cache namespace is absent
and empty before its build; the sole exception is the explicitly linked warm
TuneRun in Section 7, whose pre-state must equal cold one's post-state. Wrappers,
shell commands, ambient PATH resolution, reused outputs, implicit shared caches,
and unlisted flags are invalid.

A `BuildCommand` has exactly `command`, `decision`, and `outputs`. `command` is the
closed `Command` above. `decision` is the consumed or generated tuning-decision
`FileIdentity` for a tuned channel and JSON null for a nontuned channel. `outputs`
is the complete role-sorted list of
`OutputArtifact` objects produced by that invocation. An argv decision/output path
and every generated sidecar must resolve to exactly those file identities; an
unlisted generated file or an output without command provenance is invalid
evidence. All MainCase,
ValidationCase, and DomainCase `buildCommands` contain `BuildCommand`, while the
compile-time `commands` contain `Command` because their per-iteration outputs are
discarded and are not timed artifacts.

For every retained `.cktune`, the checker runs the retained candidate compiler's
verbose identity query and requires Identity tags 1..15 to equal that compiler and
the report's fixed schema/toolchain identities; tag 2 equals its reported source
identity. Tag 16 equals the same-name workload source digest; tags 17..19 rederive
from the retained source, modes, and schemas; tag 20 is dynamic; target tag 21
equals the report hardware triple/native CPU/features; and optional profile tag 22
equals the same-name `workload.profiles` row when present. Workload tags 1..8 are
rederived from the retained tune manifest, runner, input files, private-free
environment identity, and cases. Replay outputs equal `tuningArtifacts`; no
decision leaf can be satisfied by a different compiler, source, workload, target,
profile, or output graph.

The first channel of compile warmup row 0 is the left-listed channel and alternates
across all 18 rows without restarting at the measured boundary; the report retains
the separated 3/15 orders. `artifactSize` is a case-name-sorted seven-row list with
exactly `case`, `tunedPrimary`, `baselinePrimary`, and `baselineBuild`.
`tunedPrimary` and `baselinePrimary` are `FileIdentity`; `baselineBuild` is a
nontuned `BuildCommand` whose primary output equals `baselinePrimary` and whose
decision is null. `tunedPrimary` equals the same-name `tuningArtifacts` primary.
For the five main cases, `baselinePrimary` also equals
`cases.artifacts.v014Ordinary`. The size ratio is at most 110/100.
`archiveSize` has exactly `candidate`, `v013Replay`, `producer`, `command`, and
`members`.
`v013Replay` equals `v013ReplayBundle.archive`. `candidate` is the deterministic
gzip-compressed POSIX-pax tar produced by the recipe-pinned
`scripts/package-v014-performance-archive.py`, whose repository-root
`FileIdentity` is `producer`. `command` is the closed `Command` from Section 6:
its executable equals `producer`, argv is exactly
`[producer,"--compiler",candidateBinary,"--license",LICENSE,
"--notices",THIRD_PARTY_NOTICES.md,"--out",candidate]` after relative-path
substitution, its inputs are exactly the three member source identities, its
environment is empty, and its output path resolves to `candidate`. The script has
an absolute interpreter directive, is invoked directly without PATH or shell, and
the two stable performance hosts are Linux as required above.
`members` is an archive-path-sorted list of exactly three `PackageMember` objects;
each has exactly `path`, `mode`, and `file`. They are
`ckc-v0.14/LICENSE` mode 0644, `ckc-v0.14/THIRD_PARTY_NOTICES.md` mode 0644, and
`ckc-v0.14/ckc` mode 0755 equal to `candidateBinary`; the first two equal the
repository files. No other member, PAX key, extended attribute, or trailing member
is permitted. Member mtime/uid/gid are zero, uname/gname empty, record order is the
listed UTF-8 path order, gzip filename is empty and mtime zero, and the script's golden
test freezes compression parameters and exact archive bytes on all acceptance
hosts. The checker extracts without trusting filenames, hashes every member,
rehashes `candidate`, verifies the producer and golden algorithm test, and enforces
the 110/100 ratio. The release audit separately proves `candidateBinary` contains
the pinned LLVM/runtime statically and has no undeclared runtime dependency.

`resourceUse` has exactly `sessions` and `cacheHardLimitBytes`. The hard limit is
4,294,967,296. `sessions` is a seven-row case-name-sorted list with exactly `case`,
`decision`, `decisionDigest`, `ordinaryBuild`, `ordinarySupervisorLog`,
`ordinarySupervisorDigest`, `budget`, `wallMs`, `peakRssBytes`,
`ordinaryPeakRssBytes`, `expansions`,
`compileAttempts`, `measuredFinalists`, `validationEntrants`, and `cacheBytes`.
`decision` and `decisionDigest` equal the same-name `tuningDecisions` row. Budget is
`standard`; `ordinaryBuild` equals the same-name
`artifactSize.baselineBuild`. Wall time, tuner RSS, counters, and cache bytes are
derived from the same-name `determinism.coldOne` event/snapshot evidence.
`ordinarySupervisorLog` is a `FileIdentity` and `ordinarySupervisorDigest` follows
the exact supervisor protocol below; its embedded command digest must equal
`ordinaryBuild.command.commandDigest`, and `ordinaryPeakRssBytes` is its checked
high-water conversion. All counts fit the preset;
wall, RSS ratio, and cache meet the frozen thresholds.

## 7. Determinism and correctness

`determinism` is a case-name-sorted list of seven objects with exactly `case`,
`coldOne`, `coldTwo`, and `warm`. Each run has exactly `decision`, `outputs`,
`decisionDigest`, `choiceIdentityDigest`, `planDigest`, `objectGraphDigest`,
`linkRecipeDigest`, `outputContentDigest`, `build`, `cacheBefore`, `cacheAfter`,
`eventLog`, `eventDigest`, `supervisorLog`, `supervisorDigest`,
`compiledCandidates`, `measuredCandidates`, `wallMs`, and `peakRssBytes`. This
closed object is a `TuneRun`. `decision` is a retained
`FileIdentity`; `outputs` is the complete role-sorted `OutputArtifact` list defined
for `tuningArtifacts`. The decision digest and decoded fields must match the
retained decision. `outputContentDigest` is
`P("CK-V014-PERF-OUTPUT-CONTENT\0", outputs List<OutputContentValue>)`, where each
value is role `Text` (exactly `primary`, `header`, or `import-library`), followed
by the artifact file's bytes `U64` and decoded SHA-256 `DigestBytes`; roots, paths,
and the measurement-bearing decision file are deliberately excluded.

`build` is the exact tuned `BuildCommand`; its decision/outputs equal the enclosing
run and its command uses that run's cache namespace. `eventLog` is an evidence-root
`FileIdentity` with canonical UTF-8 lines. Its first line is
`CK-TUNE-EVENTS<TAB>1`; each later line has exactly ordinal, event kind, plan digest
or `-`, ordering phase or `-`, case id or `-`, and calls as unsigned decimal,
separated by one TAB and ending LF. Ordinals start at zero and are contiguous;
closed event kinds are `compile-attempt`, `measurement-evaluation`, `cache-hit`,
`cache-miss`, and `publication`. `eventDigest` is
`P("CK-V014-TUNE-EVENTS\0", eventLog FileIdentityValue)`. Candidate counts,
measurement counts, cache-origin claims, and publication state are rederived from
this log and the decoded decision.

`supervisorLog` is an evidence-root `FileIdentity` containing exactly three
canonical UTF-8 lines:

    CK-TUNE-SUPERVISOR<TAB>1
    start<TAB><command-digest><TAB><monotonic-raw-ns>
    wait4<TAB><monotonic-raw-ns><TAB><wait-status><TAB><ru-maxrss-kib>

Every placeholder is lowercase digest or unsigned decimal and every line ends LF.
The stable performance hosts are Linux: the supervisor reads
`CLOCK_MONOTONIC_RAW` immediately before creating the exact direct compiler child,
then obtains that same PID through successful `wait4`; raw wait status is zero and
`ru_maxrss` is the kernel high-water value in KiB, not a sample. The second clock
read occurs immediately after `wait4` and is no earlier than start.
`supervisorDigest` is
`P("CK-V014-TUNE-SUPERVISOR\0", supervisorLog FileIdentityValue)`. The embedded
command digest equals `build.command.commandDigest`; `wallMs` is the ceiling of
`(end-start)/1,000,000`, and `peakRssBytes` is checked
`ru_maxrss_kib * 1024`. The supervisor owns the namespace lock, captures the
compiler event stream, and takes the two cache snapshots; the run-level resource
fields cannot be supplied independently. Ordinary compilation uses this identical
protocol through the resource record above.

A `CacheSnapshot` has exactly `namespace`, `files`, and `digest`. `namespace` is a
unique evidence-relative directory path; `files` is a path-sorted list of every
regular non-symlink file below it, and `digest` is
`P("CK-V014-CACHE-SNAPSHOT\0", namespace Text, files
List<FileIdentityValue>)`. Unknown entries, unsafe directories, and unlisted files
are invalid. `cacheBefore`/`cacheAfter` are complete snapshots taken under an
exclusive namespace lock immediately before/after the command and are bound to
the same supervisor record.

`coldOne` and `coldTwo` have distinct namespaces and empty `cacheBefore.files`; no
other process may use either namespace. Their exact commands differ only in
destination parent and cache namespace; all generated output basenames are equal.
The warm command differs from cold one's command only in its create-new destination
parent and uses cold one's cache base/namespace, again with the same basenames.
All three destination parents are distinct evidence directories. The two
independent cold-cache runs must match
`choiceIdentityDigest`, plan,
object-graph, link-recipe, and output-content digests. Each cold decision must be
internally valid, but their `decisionDigest` values need not match because genuine
calibration and raw measurement records are part of the decision. The warm run uses
cold one's namespace, has `cacheBefore` exactly equal to `coldOne.cacheAfter`, and
permits no intervening access. It is an exact reuse of `coldOne`: it matches all six
digests including `decisionDigest`, its retained decision and output files are
byte-for-byte equal by role, it has zero compiled/measured candidates, contains a
required cache-hit and no compile/measurement event, and immutable origin facts do
not change. Its `cacheAfter` equals `cacheBefore` except for deterministic access
metadata excluded from cache identity. Hand-copying one result into multiple run
fields cannot satisfy the distinct command, namespace, snapshot, and event-log
equalities.

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
