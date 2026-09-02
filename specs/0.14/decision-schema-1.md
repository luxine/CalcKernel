# CK 0.14 `.cktune` Decision Schema 1

Status: normative, language-neutral wire contract

This document is shared by the English and Simplified Chinese CK 0.14 designs.
Field names are diagnostic labels only; tag, type, value, ordering, and bound are
the wire contract. No implementation may add, omit, reorder, or reinterpret a field
while claiming format schema 1.

## 1. Framing and primitive types

The outer file is exactly:

1. bytes `CKTUNE01`;
2. `U32(1)` format schema;
3. top-level records with tags 1 through 8, once each in increasing order;
4. `D32 = SHA-256("CK-TUNING-DECISION\0" || every preceding byte)`.

Every record field is `U16 tag || U32 payload_length || payload`. Records contain
each required tag exactly once in increasing order and no unknown tag. Primitive
types are:

| Type | Unique encoding |
| --- | --- |
| `U8/U16/U32/U64/U128` | fixed-width unsigned big-endian integer |
| `Bool` | one byte, exactly 0 or 1 |
| `D32` | exactly 32 opaque bytes |
| `Text` | `U32 length` then NFC-normalized valid UTF-8 bytes, no NUL, at most 4,096 bytes |
| `Bytes` | `U32 length` then that many opaque bytes, at most 4,096 bytes |
| `Record` | `U32 byte_length` then the record's increasing-tag TLV sequence |
| `List<T,N>` | `U32 count`, at most `N`, followed by exactly that many `T` values |
| `Opt<T>` | `U8(0)` or `U8(1)` followed by exactly one `T` |

Absolute paths are forbidden in all `Text` values. Arithmetic, lengths, counts,
NFC checks, and aggregate bounds are validated before allocation. The complete file
is at most 32 MiB.

## 2. Top-level record map

| Tag | Type | Record |
| ---: | --- | --- |
| 1 | `Identity` | compiler, source, modes, target, and optional profile |
| 2 | `Contract` | all schema and fixed policy values |
| 3 | `Workload` | canonical manifest, runner snapshot, inputs, and cases |
| 4 | `Environment` | measurement host, timer, scheduler, and calibration |
| 5 | `Frontier` | sites, units, variants, and expansion trace |
| 6 | `Candidates` | baseline, trials, correctness, and raw samples |
| 7 | `Selection` | two validation rounds and optional certificate |
| 8 | `Replay` | plan replay, output set, and origin facts |

## 3. Identity

| Tag | Field | Type |
| ---: | --- | --- |
| 1 | `ckVersion` | `Text` |
| 2 | `compilerSource` | `D32` |
| 3 | `rustToolchain` | `Text` |
| 4 | `llvmIdentity` | `Text` |
| 5 | `llvmBridge` | `D32` |
| 6 | `languageSchema` | `U32` |
| 7 | `nativeAbiSchema` | `U32` |
| 8 | `runtimeAbiSchema` | `U32` |
| 9 | `kirSchema` | `U32` |
| 10 | `proofSchema` | `U32` |
| 11 | `costModelSchema` | `U32` |
| 12 | `targetSchema` | `U32` |
| 13 | `nativeCacheSchema` | `U32` |
| 14 | `profileSchema` | `U32` |
| 15 | `pgoAnalysisSchema` | `U32` |
| 16 | `sourceDigest` | `D32` |
| 17 | `semanticContractDigest` | `D32` |
| 18 | `preTuneKirDigest` | `D32` |
| 19 | `compilationModeDigest` | `D32` |
| 20 | `outputKind` | `U8 OutputKind` |
| 21 | `target` | `TargetIdentity` |
| 22 | `profile` | `Opt<ProfileIdentity>` |

`TargetIdentity`:

| Tag | Field | Type |
| ---: | --- | --- |
| 1 | `triple` | `Text` |
| 2 | `cpu` | `Text` |
| 3 | `features` | `List<Text,256>` |
| 4 | `targetProfile` | `Text` |

Features are unique and sorted by encoded UTF-8 bytes.

`ProfileIdentity`:

| Tag | Field | Type |
| ---: | --- | --- |
| 1 | `formatSchema` | `U32` |
| 2 | `compilerSource` | `D32` |
| 3 | `sourceDigest` | `D32` |
| 4 | `topologyDigest` | `D32` |
| 5 | `contentDigest` | `D32` |
| 6 | `contentBytes` | `U64` |

## 4. Contract

| Tag | Field | Type / required value |
| ---: | --- | --- |
| 1..5 | `format/contract/measurement/inspection/planSchema` | five `U32(1)` fields |
| 6 | `budget` | `U8 Budget` |
| 7 | `beamWidth` | preset `U32` |
| 8 | `expansionLimit` | preset `U32` |
| 9 | `compileAttemptLimit` | preset `U32` |
| 10 | `measuredFinalistLimit` | preset `U32` |
| 11 | `validationEntrantLimit` | preset `U32` |
| 12 | `wallClockMs` | preset `U64` |
| 13 | `artifactRatioNumerator` | `U32(11)` |
| 14 | `artifactRatioDenominator` | `U32(10)` |
| 15 | `calibrationMinimumNs` | `U64(50,000,000)` |
| 16 | `calibrationPreferredMaximumNs` | `U64(250,000,000)` |
| 17 | `calibrationAttemptLimit` | `U32(32)` |
| 18 | `warmupRows` | `U32(3)` |
| 19 | `measuredRows` | `U32(20)` |
| 20 | `callsPerMeasuredEvaluation` | `U32(3)` |
| 21 | `containmentAllowanceMs` | `U32(2,250)` |
| 22..25 | `stableLowerNum/Den, stableUpperNum/Den` | `U32(4), U32(5), U32(6), U32(5)` |
| 26 | `stableRowsRequired` | `U32(16)` |
| 27..28 | `validationScoreNum/Den` | `U32(97), U32(100)` |
| 29..30 | `validationCaseMaximumNum/Den` | `U32(102), U32(100)` |
| 31 | `pairedWinsRequired` | `U32(16)` |
| 32 | `policyDigest` | `D32` over tags 1..31 with domain `CK-TUNE-POLICY\0` |

Preset tuples `(beam, expansion, compile, finalist, entrant, wall-ms)` are quick
`(4,1024,8,4,2,600000)`, standard `(8,4096,16,8,3,1800000)`, and thorough
`(16,16384,32,16,4,7200000)`.

## 5. Workload

| Tag | Field | Type |
| ---: | --- | --- |
| 1 | `manifestDigest` | `D32` |
| 2 | `runnerSnapshotDigest` | `D32` |
| 3 | `runnerSnapshotBytes` | `U64` |
| 4 | `argv` | `List<Text,64>`; aggregate at most 65,536 bytes |
| 5 | `environment` | `List<EnvironmentEntry,16>`; aggregate at most 65,536 bytes |
| 6 | `timeoutMs` | `U32`, 100..120,000 |
| 7 | `inputs` | `List<InputIdentity,64>` |
| 8 | `cases` | `List<CaseIdentity,16>` |

`EnvironmentEntry` is tag 1 `name: Text`, tag 2 `value: Bytes`. Unix stores the
exact non-NUL environment value bytes; Windows stores the exact value as UTF-8
without normalization and rejects an unrepresentable value. Entries are sorted
by Unix name bytes or Windows ASCII-case-folded name bytes and are unique under the
same comparison. The count is the complete effective set: Windows inserts required
`SystemRoot`/`WINDIR` records first, unions canonically spelled allowlist records,
and rejects a total above 16. `InputIdentity` is tag 1 `logicalPath: Text`, tag 2 `digest: D32`,
tag 3 `bytes: U64`; inputs preserve manifest order, one input is at most 1 GiB and
their sum at most 4 GiB. `CaseIdentity` is tag 1 `id: Text`, tag 2 `role: U8
CaseRole`, tag 3 `seed: U64`, tag 4 `weight: U32`, tag 5 `expectedDigest: D32`;
cases are sorted by id bytes and include at least one role of each kind.

## 6. Environment

| Tag | Field | Type |
| ---: | --- | --- |
| 1..9 | `osFamily/osBuild/kernel/architecture/cpuVendor/cpuFamily/cpuModel/cpuStepping/microcode` | nine `Text` fields |
| 10 | `cpuFeatures` | sorted unique `List<Text,256>` |
| 11 | `physicalCores` | `Opt<U32>` |
| 12 | `logicalCores` | `U32`, positive |
| 13 | `numaNodes` | `Opt<U32>` |
| 14 | `timerKind` | `Text` |
| 15 | `timerResolutionNs` | `U64`, positive |
| 16 | `schedulingPolicy` | `Text` |
| 17 | `calibrations` | `List<Calibration,16>` in case-id order |
| 18 | `sessionDigest` | `D32`, derived by Section 11 |

Unavailable textual host facts are the literal `unavailable`; unavailable numeric
facts use `Opt(0)`. `Calibration` is tag 1 `caseId: Text`, tag 2 `iterations: U64`,
tag 3 `attempts: U32`, tag 4 `acceptedElapsedNs: U64`, tag 5
`confirmationElapsedNs: U64`, tag 6 `overshoot: Bool`.

## 7. Frontier

`Frontier` is tag 1 `candidateSpaceDigest: D32`, tag 2 `sites:
List<Site,4096>`, tag 3 `units: List<Unit,64>`, and tag 4 `expansions:
List<Expansion,16384>`.

`Site`:

| Tag | Field | Type |
| ---: | --- | --- |
| 1 | `siteId` | `D32` |
| 2 | `class` | `U8 AlternativeClass` |
| 3 | `rootId` | `D32` |
| 4 | `preStateDigest` | `D32` |
| 5 | `canonicalRank` | `U32` |

`Unit` is tag 1 `unitId: D32`, tag 2 `siteIds: List<D32,4096>`, tag 3
`baselineStateDigest: D32`, and tag 4 `variants: List<UnitVariant,4>`.

`UnitVariant` is tag 1 `variantId: D32`, tag 2 `class: U8 AlternativeClass`, tag 3
`siteAlternatives: List<SiteAlternative,4096>`, tag 4 `predictedDynamic: U64`, tag
5 `predictedStatic: U64`, tag 6 `predictedKirUnits: U64`, and tag 7
`postStateDigest: D32`. `SiteAlternative` is tag 1 `siteId: D32`, tag 2
`alternativeId: D32`, tag 3 `preStateDigest: D32`, tag 4 `postStateDigest: D32`.
Across all units there are at most 256 variants.

`Expansion` is tag 1 `ordinal: U32`, tag 2 `parentPlanDigest: D32`, tag 3 `unitId:
D32`, tag 4 `variantId: D32`, tag 5 `disposition: U8 ExpansionDisposition`, tag 6
`resultPlanDigest: Opt<D32>`, and tag 7 `diagnosticCode: U16`.

## 8. Candidates and measurements

`Candidates` is tag 1 `baseline: Candidate` and tag 2 `trials:
List<Candidate,32>`.

`Candidate`:

| Tag | Field | Type |
| ---: | --- | --- |
| 1 | `planDigest` | `D32`; baseline is the canonical empty-plan digest |
| 2 | `choices` | `List<PlanChoice,64>` |
| 3 | `objectGraphDigest` | `D32` |
| 4 | `linkRecipeDigest` | `D32` |
| 5 | `primaryArtifactBytes` | `U64` |
| 6 | `outcome` | `U8 CandidateOutcome` |
| 7 | `diagnosticCode` | `U16`, zero means absent |
| 8 | `correctnessDigest` | `Opt<D32>` |
| 9 | `streams` | `List<MeasurementStream,48>` |
| 10 | `compileOrigin` | `CacheOrigin` |
| 11 | `timeout` | `Opt<TimeoutRecord>` |

`PlanChoice` is tag 1 `unitId: D32`, tag 2 `variantId: D32`, tag 3 `class: U8
AlternativeClass`, tag 4 `preStateDigest: D32`, tag 5 `postStateDigest: D32`.

`MeasurementStream` is tag 1 `phase: U8 OrderingPhase` (only 3, 5, or 7), tag 2
`round: U8` (0, 1, or 2 consistently with phase), tag 3 `caseId: Text`, tag 4
`planDigest: D32`, tag 5 `iterations: U64`, tag 6 `rows:
List<MeasurementRow,20>`, and tag 7 `correctnessDigest: D32`. A complete stream has
exactly 20 rows. `MeasurementRow` is tag 1 `ordinal: U32` (0..19), tag 2
`permutationKey: D32`, tag 3 `callsNs: List<U64,3>` with exactly three positive
values, and tag 4 `storedMinimumNs: U64`, equal to their minimum.

`TimeoutRecord` is tag 1 `phase: U8 OrderingPhase` (1 through 7), tag 2 `round: U8`, tag 3
`row: U32`, tag 4 `caseId: Text`, tag 5 `call: U8`, and tag 6 `elapsedNs: U64`.
A smoke timeout is `(phase=1,round=0,row=0,call=1)`; warmup rows use call 1;
measured rows use calls 1..3. Row and round ranges must match the phase.
A timed-out candidate has this record; all others forbid it. Across the baseline and
32 trials there are at most 1,584 streams.

## 9. Selection

`Selection` is tag 1 `roundOne: RoundSummary`, tag 2 `roundTwo: RoundSummary`, tag
3 `selectedPlanDigest: D32`, tag 4 `reason: U8 SelectionReason`, and tag 5
`certificate: Opt<Certificate>`. A tuned result requires the certificate; every
baseline reason forbids it.

`RoundSummary` is tag 1 `round: U8`, tag 2 `plans: List<RoundPlan,4>`, and tag 3
`rankedPlanDigests: List<D32,4>`. `RoundPlan` is tag 1 `planDigest: D32`, tag 2
`caseMedians: List<CaseMedian,16>`, tag 3 `aggregateRatioQ32: U64`, tag 4 `stable:
Bool`, tag 5 `thresholdPassed: Bool`, tag 6 `pairedWins: U32`. `CaseMedian` is tag 1
`caseId: Text`, tag 2 `baselineNs: U64`, tag 3 `candidateNs: U64`, tag 4
`ratioQ32: U64`.

`Certificate` is tag 1 `planDigest: D32`, tag 2 `frontierDigest: D32`, tag 3
`policyDigest: D32`, tag 4 `roundOneDigest: D32`, tag 5 `roundTwoDigest: D32`, tag
6 `correctnessDigest: D32`, tag 7 `objectGraphDigest: D32`, tag 8
`linkRecipeDigest: D32`.

## 10. Replay

| Tag | Field | Type |
| ---: | --- | --- |
| 1 | `frontierDigest` | `D32` |
| 2 | `selectedPreStateDigest` | `D32` |
| 3 | `selectedPostStateDigest` | `D32` |
| 4 | `objectGraphDigest` | `D32` |
| 5 | `linkRecipeDigest` | `D32` |
| 6 | `outputs` | `List<OutputIdentity,3>` |
| 7 | `compileOrigin` | `CacheOrigin` |
| 8 | `measurementOrigin` | `CacheOrigin` |
| 9 | `replayResultDigest` | `D32` |

`OutputIdentity` is tag 1 `role: U8 OutputRole`, tag 2 `logicalBasename: Text`, tag
3 `contentDigest: D32`, tag 4 `contentBytes: U64`. Executable has primary only;
dynamic has primary and header; Windows dynamic also has import library.

`CacheOrigin` is tag 1 `kind: U8 CacheOriginKind`, tag 2 `keyDigest: D32`, tag 3
`entryDigest: D32`. These describe the decision's origin session and are immutable
when a completed decision is reused.

## 11. Digest derivations

For every derived digest below:

    H(domain, value...) = SHA-256(ASCII domain including its final NUL ||
                                  canonical typed encoding of each value)

The typed encoding is Section 1's encoding, including every length and count. A
named material record uses the stated increasing tags and the ordinary `Record`
encoding. This rule removes concatenation and empty-value ambiguity.

| Digest | Domain and canonical material |
| --- | --- |
| manifest | `CK-TUNE-MANIFEST\0`; `ManifestMaterial`: schema `U32(1)`, argv, effective environment, timeout, manifest-order inputs, case-id-order cases, runner bytes, runner digest at tags 1..8 |
| target identity | `CK-TUNE-TARGET\0`; complete `TargetIdentity` record |
| site id | `CK-TUNE-SITE\0`; root id, class, canonical site ordinal, pre-state digest |
| alternative id | `CK-TUNE-ALTERNATIVE\0`; site id, class, canonical choice payload `Bytes`, post-state digest |
| unit id | `CK-TUNE-UNIT\0`; site-id list and baseline-state digest |
| unit variant id | `CK-TUNE-UNIT-VARIANT\0`; unit id, class, site alternatives, three predicted costs, post-state digest |
| plan digest | `CK-TUNE-PLAN\0`; unit-id-ordered `List<PlanChoice,64>`; the empty-plan digest is this same domain over the zero-count list |
| candidate-space digest | `CK-TUNE-CANDIDATE-SPACE\0`; site-id-ordered sites and unit-id-ordered units, excluding the expansion trace |
| frontier digest | `CK-TUNE-FRONTIER\0`; candidate-space digest plus ordinal-ordered expansion trace |
| correctness aggregate | `CK-TUNE-CORRECTNESS\0`; case-id-ordered `List<CaseCorrectness,16>` where each record is tag 1 case id and tag 2 observed digest |
| object-graph digest | `CK-TUNE-OBJECT-GRAPH\0`; `ObjectGraphMaterial` below |
| link-recipe digest | `CK-TUNE-LINK-RECIPE\0`; `LinkRecipeMaterial` below |
| session digest | `CK-TUNE-SESSION\0`; `SessionMaterial` below |
| validation-round digest | `CK-TUNE-VALIDATION-ROUND\0`; complete `RoundSummary` record |
| certificate digest | `CK-TUNE-CERTIFICATE\0`; complete `Certificate` record |
| output-set id | `CK-TUNE-OUTPUT-SET\0`; `OutputSetMaterial` below |
| replay-result digest | `CK-TUNE-REPLAY-RESULT\0`; `ReplayResultMaterial` below |

Within `UnitVariant`, `siteAlternatives` are sorted by `(siteId, alternativeId)`.
The unit-variant digest uses that sorted list. All correctness observations for the
same case must agree; a candidate aggregate contains every distinct case it
successfully executed. A profitability certificate requires all manifest cases.

`ObjectGraphMaterial` is tag 1 `schema: U32(1)`, tag 2 `outputKind: U8`, tag 3
`targetIdentityDigest: D32`, tag 4 `objects: List<ObjectIdentity,4096>`. Objects sort
by `stableObjectId`. `ObjectIdentity` is tag 1 `stableObjectId: D32`, tag 2
`contentDigest: D32`, tag 3 `contentBytes: U64`, tag 4 `dependencies:
List<D32,4096>` sorted by digest.

`LinkRecipeMaterial` is tag 1 `schema: U32(1)`, tag 2 `outputKind: U8`, tag 3
`targetIdentityDigest: D32`, tag 4 `objectsInLinkOrder: List<D32,4096>`, tag 5
`librariesInLinkOrder: List<LinkInput,256>`, tag 6 `exports: List<Text,4096>` sorted
by bytes, tag 7 `normalizedFlagsInOrder: List<Text,256>`. `LinkInput` is tag 1
`logicalName: Text`, tag 2 `identityDigest: D32`. Semantic link order is preserved,
not sorted.

`SessionMaterial` is tag 1 the complete top-level `Identity`, tag 2 `Contract`, tag
3 `Workload`, tag 4 an `EnvironmentSeed` containing Environment tags 1..16, tag 5
the complete `Frontier`, and tag 6 `BaselineSeed`. `BaselineSeed` is plan digest,
object-graph digest, link-recipe digest, and primary bytes at tags 1..4. It excludes
calibration records, correctness, measurements, cache facts, paths, timestamps, and
destinations. Its result is stored as Environment tag 18.

`OutputSetMaterial` is tag 1 output kind, tag 2 canonical decision path `Bytes`, and
tag 3 a role-sorted list whose records contain output role and canonical destination
path `Bytes`. Unix paths are exact non-NUL bytes; Windows paths are normalized
absolute UTF-8 with invariant case-folding for comparison. This operational digest
is journal-only, so the decision's prohibition on absolute `Text` remains intact.

`ReplayResultMaterial` is tag 1 frontier digest, tag 2 selected plan digest, tags
3..4 selected pre/post-state digests, tags 5..6 object-graph/link-recipe digests,
and tag 7 the role-sorted `OutputIdentity` list. It excludes cache origin and local
absolute paths.

The policy digest remains `H("CK-TUNE-POLICY\0", Contract tags 1..31)`. The outer
decision digest remains the framing rule in Section 1.

## 12. Enums and canonical ordering

| Enum | Values |
| --- | --- |
| `OutputKind` | executable=1, dynamic=2 |
| `Budget` | quick=1, standard=2, thorough=3 |
| `CaseRole` | search=1, validation=2 |
| `AlternativeClass` | inlining=1, specialization=2, unrolling=3, loop-SIMD=4, SLP=5, short-slice/versioning=6, layout=7 |
| `ExpansionDisposition` | legal=1, illegal=2, duplicate=3, growth-rejected=4 |
| `CandidateOutcome` | baseline=1, compiled-unmeasured=2, size-rejected=3, timed-out=4, search-nonwinner=5, validation-threshold=6, validation-disagreement=7, selected=8 |
| `OrderingPhase` | candidate-smoke=1, search-warmup=2, search-measured=3, validation-one-warmup=4, validation-one-measured=5, validation-two-warmup=6, validation-two-measured=7 |
| `SelectionReason` | tuned=1, no-candidate=2, validation-threshold=3, validation-disagreement=4 |
| `OutputRole` | primary=1, header=2, import-library=3 |
| `CacheOriginKind` | freshly-built=1, verified-local-hit=2 |

Cases sort by id; sites, units, and variants by stable id; site alternatives by
site id then alternative id; expansions by ordinal;
trials by plan digest; choices by unit id; streams by phase, round, case id, and plan
digest; rows by ordinal; validation plans by plan digest; and outputs by role. The
baseline precedes all trials. Ordering compares digest bytes, integer values, or
encoded UTF-8 bytes as applicable. Duplicate sort keys are invalid unless a record
definition explicitly permits them; schema 1 permits none.

## 13. Normative fixtures

The repository must freeze these before the format implementation passes:

- `tests/fixtures/tune/decision-schema1-framing.hex` covering all primitive and
  container encodings and both optional states;
- `tests/fixtures/tune/decision-schema1-baseline.cktune` with one search case, one
  validation case, an empty frontier, and `no-candidate`;
- `tests/fixtures/tune/decision-schema1-tuned.cktune` with one legal unit variant,
  all three measured phases, two agreeing rounds, and a certificate;
- `tests/fixtures/tune/decision-schema1-inspection.json` for the tuned vector.

The test source pins each fixture SHA-256. The same files drive encode, decode,
inspect, re-encode byte equality, mutation, truncation, limit, and cross-endian
tests. A golden vector change requires a future format schema.
