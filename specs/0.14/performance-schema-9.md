# CK 0.14 Performance Report Schema 9

Status: normative, language-neutral release-evidence contract

This document is shared by the English and Simplified Chinese CK 0.14 designs.
Every JSON object rejects missing, duplicate, or unknown keys. JSON is UTF-8 with
lexicographically sorted keys, no insignificant whitespace, and one trailing LF.
Integers are nonnegative JSON integers within `u64` unless a smaller range is
stated. Timing values are positive nanoseconds. A `Digest` is exactly 64 lowercase
hexadecimal SHA-256 characters. A `FileIdentity` has exactly `path`, `bytes`, and
`sha256`; `path` is repository-relative or evidence-directory-relative, never
absolute or traversing, and resolves to a regular non-symlink file.

## 1. Closed top level

The exact top-level keys are:

    schemaVersion, candidateVersion, candidateSha, v013ReplayCommit,
    evidenceDirectory, toolchain, hardware, recipe, candidateBinary,
    v013ReplayBundle, cumulativeSchemaEight, workload, tuningDecisions,
    tuningArtifacts, sampling, cases, domainCases, tuneUseCompileTime,
    ordinaryCompileRegression, artifactSize, archiveSize, resourceUse,
    determinism, correctness

Fixed scalar values are `schemaVersion=9`, `candidateVersion="0.14.0"`, and
`candidateSha` equal to `git rev-parse HEAD`. `v013ReplayCommit` equals the commit
in `benches/baselines/v0_13_replay.toml`. `evidenceDirectory` matches
`v014-measurement-[0-9]+-[0-9]+` and names the real sibling directory.

## 2. Identity objects

`toolchain` has exactly `llvmVersion`, `clangVersion`, `rustVersion`,
`componentManifestSha256`, and `clangProfileRuntimeSha256`. Versions are exactly
`22.1.8`, `22.1.8`, and `1.90.0`; both digests match retained files.

`hardware` has exactly `target`, `arch`, `os`, `osBuild`, `kernel`, `cpuModel`,
`logicalCpus`, `physicalCpus`, `numaNodes`, `features`, `requiredTier`,
`availableTiers`, `osState`, and `capabilityDigest`. Feature and tier lists are
sorted and unique. The digest is over the other fields in
canonical JSON with domain `CK-V014-PERF-HARDWARE\0`. The x86-64 job requires
`requiredTier="x86-64-v4"`; the AArch64 job requires
`requiredTier="aarch64-sve2"`. That tier must occur in `availableTiers` and all of
its required features in `features`. Missing required hardware fails instead of
skipping.

`recipe` has exactly `schema`, `files`, `digest`, and `thresholds`. `schema=1`.
`files` is a path-sorted list of `FileIdentity` covering every path named in Section
19.1 of the design; `digest` is the domain-separated digest of their path and SHA
pairs. `thresholds` has exactly:

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

`candidateBinary` and `cumulativeSchemaEight` are `FileIdentity` objects. The latter
is exactly `results-schema8.json` and must independently pass the schema-8 checker.

`v013ReplayBundle` has exactly `commit`, `manifest`, `compiler`, `archive`, and
`schemaEight`; the last four are `FileIdentity`. Its commit and all identities must
equal `benches/baselines/v0_13_replay.toml`, and its schema-8 file independently
passes before any schema-9 threshold is evaluated.

## 3. Workload and sampling

`workload` has exactly `casesManifest`, `sources`, `search`, `validation`,
`adversarial`, `releaseHeldOut`, `tuneManifests`, `runner`, `oracleManifest`,
`cOracle`, and `rustOracle`. Scalar file members are `FileIdentity`; `sources` and
`tuneManifests` are path-sorted `FileIdentity` lists of exactly seven and seven
entries. Their file set and logical rows equal Section 19.1 exactly.

`sampling` has exactly `mainProtocol`, `domainProtocol`, `mainChannels`,
`domainChannels`, `warmupRows`, `sampleRows`, `callsPerSample`, `statistic`,
`stabilityPolicy`, and `rerunPolicy`. Values are exactly:

- `mainProtocol="rotating-six-channel-v1"`;
- `domainProtocol="rotating-three-channel-v1"`;
- main channels `[tuned,v014Ordinary,v013Ordinary,v013Pgo,cSimd,rustSimd]`;
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
channel count from the fixed channel list. `split` is `release-held-out` or
`domain-release-held-out`. The stored order must equal this formula.

## 4. Decisions and artifacts

`tuningDecisions` is a case-name-sorted list of exactly seven objects with keys
`case`, `file`, `decisionDigest`, `selectionReason`, `planDigest`,
`objectGraphDigest`, `linkRecipeDigest`, `certificateDigest`, and `outputRecords`.
`file` is a `FileIdentity`; five digest fields are `Digest`, except
`certificateDigest` is either a `Digest` for `tuned` or JSON null for a baseline
reason. `outputRecords` is a role-sorted list with exact keys `role`, `logicalName`,
`bytes`, and `sha256`, and must equal the decoded decision and retained outputs.

`tuningArtifacts` is a case-name-sorted list of exactly seven objects with keys
`case`, `decision`, and `outputs`; `decision` is a `FileIdentity`, and `outputs` is
the complete role-sorted list of one, two, or three objects with exactly `role`,
`path`, `bytes`, and `sha256`. Every identity equals `tuningDecisions`.

## 5. Steady-state cases

`cases` is a case-name-sorted list of exactly five `MainCase` objects. `domainCases`
is a case-name-sorted list of exactly two `DomainCase` objects for
`contract-fixed-length` and `contract-noalias`.

A `MainCase` has exactly `case`, `eligible`, `source`, `input`, `decisionDigest`,
`correctnessDigest`, `artifacts`, `warmupOrder`, `sampleOrder`, `samplesNs`, and
`mediansNs`. `eligible` is true. `source` and `input` are `FileIdentity`;
`artifacts` has exactly the six main-channel `FileIdentity` keys. `warmupOrder` is
three channel permutations and `sampleOrder` is twenty channel permutations; each
row contains every channel exactly once and equals the specified digest rotation.
`samplesNs` has exactly six channel keys, each a list of 20 positive integers;
`mediansNs` has those keys and equals each list's ascending element 10. Every stream
passes the 16-of-20 inclusive 80%..120% stability rule.

A `DomainCase` has the same keys and rules except `eligible` is absent,
`buildCommands` is added, and all channel-shaped objects/orders use exactly the
three domain channels. `buildCommands` has exactly the three channel keys and each
value is the closed command object from Section 6. The tuned artifact and
`decisionDigest` must equal the corresponding entries in `tuningArtifacts` and
`tuningDecisions`. Each case's correctness digest agrees across all channels.

The main gates use release-held-out rows only. For every selected tuned case,
`tuned/v013-faster` is at most 98/100; every case, including baseline selections,
enters the five-case held-out geometric gate of at most 95/100; no validation or
release-held-out ratio exceeds 102/100. Oracle throughput meets 98/100 geometric
and 92/100 per case. The two domain cases jointly satisfy the strict 108/100
throughput gate against the faster generic oracle.

## 6. Compilation, size, and resource records

`tuneUseCompileTime` is a case-name-sorted seven-row list comparing `tuneUse` with
`v014Ordinary`. `ordinaryCompileRegression` is the same shape comparing
`v014Ordinary` with `v013Ordinary`. Each row has exactly `case`, `warmupOrder`,
`sampleOrder`, `samplesNs`, `mediansNs`, and `commands`. Orders contain three and
fifteen two-channel permutations with alternating first channel. Each samples list
has 15 positive values, each median is ascending element 7. `commands` is an object
with exactly the two channel keys; each value has exactly `argv`, `executable`,
`inputs`, and `environmentDigest`, where `argv` is the exact string vector,
`executable` is a `FileIdentity`, `inputs` is a path-sorted `FileIdentity` list, and
`environmentDigest` is a `Digest`.

The first channel of compile warmup row 0 is the left-listed channel and alternates
across all 18 rows without restarting at the measured boundary; the report retains
the separated 3/15 orders. `artifactSize` is a case-name-sorted seven-row list with exactly `case`,
`tunedPrimary`, and `baselinePrimary`; the latter two are `FileIdentity` and the
ratio is at most 110/100. `archiveSize` has exactly `candidate` and `v013Replay`,
both `FileIdentity`, with ratio at most 110/100.

`resourceUse` has exactly `sessions` and `cacheHardLimitBytes`. The hard limit is
4,294,967,296. `sessions` is a seven-row case-name-sorted list with exactly `case`,
`budget`, `wallMs`, `peakRssBytes`, `ordinaryPeakRssBytes`, `expansions`,
`compileAttempts`, `measuredFinalists`, `validationEntrants`, and `cacheBytes`.
Budget is `standard`; all counts fit its preset; wall, RSS ratio, and cache meet the
frozen thresholds.

## 7. Determinism and correctness

`determinism` is a case-name-sorted list of seven objects with exactly `case`,
`coldOne`, `coldTwo`, and `warm`. Each run has exactly `decisionDigest`,
`planDigest`, `objectGraphDigest`, `linkRecipeDigest`, `outputSetDigest`,
`compiledCandidates`, and `measuredCandidates`. The two cold runs match all five
digests. Warm matches them, has zero compiled/measured candidates, and reproduces
the original decision bytes; immutable origin facts therefore do not change.

`correctness` has exactly `search`, `validation`, `adversarial`,
`releaseHeldOutDifferential`, `domainDifferential`, `oracleUbAudit`, `aliasAudit`,
and `featureAudit`, all boolean true.

The collector only writes evidence. The independent checker reopens and hashes
every retained file, validates every closed object and cardinality above, replays
all integer statistics and rotations, rechecks schema 8, and evaluates every gate.
It rejects nonfinite values, JSON floats for integer fields, duplicate keys,
unknown files, symlinks, identity mismatches, and any evidence not reproducible from
the retained raw records.
