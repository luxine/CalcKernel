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
| `Blob32M` | digest-material only: `U64 length` then opaque bytes, at most 32 MiB; never a decision-file field |
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
| 8 | `Replay` | plan replay, chosen-code identity, output set, and origin facts |

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

`EnvironmentEntry` is tag 1 `name: Text`, tag 2 `valueBytes: U64`, and tag 3
`valueDigest: D32`. The digest is
`H("CK-TUNE-ENV-VALUE\0", name Text, value Bytes)` over the private accepted value;
the value itself is never a decision field.
`ManifestInputMaterial` and the wire-identical `InputIdentity` are tag 1
`logicalPath: Text`, tag 2 `digest: D32`, tag 3 `bytes: U64`.
`ManifestCaseMaterial` and the wire-identical `CaseIdentity` are tag 1 `id: Text`,
tag 2 `role: U8 CaseRole`, tag 3 `seed: U64`, tag 4 `weight: U32`, and tag 5
`expectedDigest: D32`.

Every argv and logical-path `Text` value must already be NFC and at most 4,096
UTF-8 bytes; non-NFC input is rejected rather than rewritten. Unix executes those
exact accepted bytes as argv elements. Windows applies the closed conversion and
quoting ABI in the main design. Unix environment identity hashes the exact non-NUL
value bytes; Windows hashes the exact UTF-8 encoding of the accepted UTF-16 value
without normalization and rejects an unrepresentable value. Entries are sorted
by Unix name bytes or Windows ASCII-case-folded name bytes and are unique under the
same comparison. The count is the complete effective set: Windows inserts required
`SystemRoot`/`WINDIR` records first, unions canonically spelled allowlist records,
and rejects a total above 16. Every stored length equals the private value length
and every digest rederives from that value during tune build; inspection exposes
only the length and digest. Inputs preserve manifest order, one input is at most
1 GiB and their sum at most 4 GiB. Cases are sorted by id bytes and include at least
one role of each kind.

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
| 19 | `measurementCacheSaltDigest` | `D32` |

Unavailable textual host facts are the literal `unavailable`; unavailable numeric
facts use `Opt(0)`. `Calibration` is tag 1 `caseId: Text`, tag 2 `iterations: U64`,
tag 3 `attempts: U32`, tag 4 `acceptedElapsedNs: U64`, tag 5
`confirmationElapsedNs: U64`, tag 6 `overshoot: Bool`.
`measurementCacheSaltDigest` is
`H("CK-TUNE-MEASUREMENT-SALT\0", Blob32M(local installation salt))`; the salt is
32 CSPRNG bytes stored with owner-only permissions outside the decision, while only
this digest is recorded.

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
| 6 | `rootAnchor` | `RootAnchor` |

`Unit` is tag 1 `unitId: D32`, tag 2 `siteIds: List<D32,4096>`, tag 3
`baselineStateDigest: D32`, and tag 4 `variants: List<UnitVariant,4>`.

`UnitVariant` is tag 1 `variantId: D32`, tag 2 `class: U8 AlternativeClass`, tag 3
`siteAlternatives: List<SiteAlternative,4096>`, tag 4 `isolatedDynamicEstimate: U64`, tag
5 `isolatedStaticEstimate: U64`, tag 6 `isolatedKirBytes: U64`, and tag 7
`postStateDigest: D32`. These isolated fields are diagnostics and variant-generation
inputs, never plan-rank keys. `SiteAlternative` is tag 1 `siteId: D32`, tag 2
`alternativeId: D32`, tag 3 `preStateDigest: D32`, tag 4 `postStateDigest: D32`,
and tag 5 `payload: AlternativePayload`.
Across all units there are at most 256 variants.

A unit has one through four variants, its `siteIds` are sorted by site id, and all
its variants and referenced sites have one `AlternativeClass`. The application
phase derived from that class is specialization=1, inlining=2,
short-slice/versioning=3, loop-SIMD=4, unrolling=5, SLP=6, and layout=7. Units are
sorted by `(application phase, unitId)`; the phase is derived and is not an
additional wire field.

`RootAnchor` is tag 1 `functionSymbol: Text`, tag 2 `kind: U8 RootKind`, and tag 3
`preorderOrdinal: U32`. The module anchor uses an empty symbol, kind module, and
ordinal zero. Every other anchor names its containing stable ABI or internal
function symbol and the zero-based preorder ordinal among nodes of the same kind in
that function's canonical pre-tune KIR. Symbols are unique in one module, and an
anchor must resolve exactly once.

`AlternativePayload` is a closed discriminated record: tag 1 repeats the
`AlternativeClass`, and tag 2 is exactly the corresponding class record below. All
integer bounds are additionally constrained by target legality; values outside the
listed structural bounds are noncanonical.

| Class | Tag-2 record |
| --- | --- |
| inlining | tag 1 `calleeSymbol: Text`; tag 2 `action: U8 InliningAction` |
| specialization | tag 1 `bindings: List<SpecializationBinding,16>` sorted by argument ordinal; tag 2 `guarded: Bool` |
| unrolling | tag 1 `factor: U32`, a power of two from 2 through 64 |
| loop-SIMD | tag 1 `vectorBits: U32`, a power of two from 64 through 2,048; tag 2 `interleave: U32` from 1 through 8; tag 3 `breakEvenIterations: U32` |
| SLP | tag 1 `packWidth: U32` from 2 through 64; tag 2 `operandAnchors: List<RootAnchor,64>` in lane order |
| short-slice/versioning | tag 1 `maximumLength: U32`; tag 2 `vectorBits: U32` under the loop-SIMD bound; tag 3 `interleave: U32` from 1 through 8 |
| layout | tag 1 `scope: U8 LayoutScope`; tag 2 `rootOrder: List<D32,4096>` containing each affected root id exactly once |

`SpecializationBinding` is tag 1 `argumentOrdinal: U32`, tag 2 `kind: U8
SpecializationValueKind`, and tag 3 `bits: U128`. For u32/i32/f32-bits/length-u32
the upper 96 bits are zero; for u64/i64/f64-bits the upper 64 bits are zero. Integer
signedness is represented by `kind`, not sign extension. Duplicate ordinals are
invalid.

`Expansion` is tag 1 `ordinal: U32`, tag 2 `parentPlanDigest: D32`, tag 3 `unitId:
D32`, tag 4 `variantId: D32`, tag 5 `disposition: U8 ExpansionDisposition`, tag 6
`resultPlanDigest: Opt<D32>`, tag 7 `diagnosticCode: U16`, and tags 8..10
`wholePlanDynamic/wholePlanStatic/wholePlanKirBytes: Opt<U64>`. Result digest and
all three metrics are present exactly for `legal`; a duplicate has only its result
digest; illegal and growth-rejected have neither. Diagnostic code is zero exactly
for legal and duplicate.

Expansion ordinals are exactly zero-based and contiguous. The first record has
ordinal 0 when nonempty, record `i` has ordinal `i`, and the list contains exactly
the indices `0 <= i < expansions.len()`. Starting with the empty baseline, the checker walks every
unit, precompile-ranked beam plan, and canonical nonbaseline variant in the main
design's nested-loop order. Each expected derivation consumes the next record,
whose parent/unit/variant, disposition, optional result, diagnostics, and recomputed
whole-plan metrics must match. The list stops only when all units are exhausted or
the preset expansion limit is reached; omission, insertion, reorder, or
reclassification is invalid.

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
| 12 | `primaryArtifactDigest` | `D32` |

`PlanChoice` is tag 1 `unitId: D32`, tag 2 `variantId: D32`, tag 3 `class: U8
AlternativeClass`, tag 4 `preStateDigest: D32`, tag 5 `postStateDigest: D32`.

`MeasurementStream` is tag 1 `phase: U8 OrderingPhase` (only 3, 5, or 7), tag 2
`round: U8` (0, 1, or 2 consistently with phase), tag 3 `caseId: Text`, tag 4
`planDigest: D32`, tag 5 `iterations: U64`, tag 6 `rows:
List<MeasurementRow,20>`, and tag 7 `correctnessDigest: D32`. A complete stream has
exactly 20 rows. `MeasurementRow` is tag 1 `ordinal: U32` (0..19), tag 2
`permutationKey: D32`, tag 3 `callsNs: List<U64,3>` with exactly three positive
values, and tag 4 `storedMinimumNs: U64`, equal to their minimum.

Every `permutationKey` equals
`H("CK-TUNE-ORDER\0", sessionDigest D32, phase U8, round U8, row U32,
caseId Text)`. The separate case-list rotation uses the same domain and first four
typed values followed by `Bytes([0xff])`; it is recomputed but not stored as a row
key. Both rotations interpret the first eight digest bytes as a big-endian `u64`.

`TimeoutRecord` is tag 1 `phase: U8 OrderingPhase` (1 through 7), tag 2 `round: U8`, tag 3
`row: U32`, tag 4 `caseId: Text`, tag 5 `call: U8`, and tag 6 `elapsedNs: U64`.
A smoke timeout is `(phase=1,round=0,row=0,call=1)`; warmup rows use call 1;
measured rows use calls 1..3. Row and round ranges must match the phase.
A timed-out candidate has this record; all others forbid it. Across the baseline and
32 trials there are at most 1,584 streams.

The terminal state matrix is exact. `S` means one complete phase-3 stream for every
search case; `V1` and `V2` mean complete phase-5 and phase-7 streams for every
validation case. Baseline validation runs even when there are zero entrants.

| Outcome | Correctness aggregate | Streams | Timeout | Diagnostic |
| --- | --- | --- | --- | --- |
| baseline | required over all cases | exactly S+V1+V2 | absent | 0 |
| compiled-unmeasured | absent | empty | absent | 0 |
| size-rejected | absent | empty | absent | artifact-size-rejected |
| timed-out | present iff at least one smoke/stream case completed | exact set of complete measured streams before the recorded timeout | required | candidate-timeout |
| search-nonwinner | required over search cases | exactly S | absent | 0 |
| validation-threshold | required over all cases | exactly S+V1+V2 | absent | 0 |
| validation-nonwinner | required over all cases | exactly S+V1+V2 | absent | 0 |
| selected | required over all cases | exactly S+V1+V2 | absent | 0 |

A timed-out stream contributes no record even when earlier rows in that stream
completed; its timeout record preserves the exact phase, round, row, case, and call.
The checker recomputes the actual case/channel rotation from the session digest and
stores, in Section 12's canonical stream order, the set of exactly those streams
whose twentieth measured row completed before that coordinate. This set need not
be a canonical prefix because the final row rotates case order. A timeout in smoke
or warmup therefore stores only measured streams completed before that phase. Any
state/field combination outside this table or any missing/extra completed stream is
invalid. The selection-wide rules below, rather than a candidate-local guess,
assign every completed validation entrant's terminal outcome.

The trace replay above deterministically reconstructs the final beam and applies
the preset compile-attempt diversity truncation. `Candidates.trials` is exactly
that compile-selection set, sorted by plan digest, with one record per plan and no
other record; each plan's choices and rank material equal the reconstructed plan.
A decision is invalid if expansion or compile selection stopped early for wall
budget. For source-aware acceptance, every trial is independently rebuilt in an
isolated cache and its object graph, link recipe, primary digest, and actual byte
count must equal the record even when the original origin was a cache hit.

Using checked `u128`, a trial is size-valid exactly when
`trial.primaryArtifactBytes * 100 <= baseline.primaryArtifactBytes * 110`.
Every other trial has outcome `size-rejected`. The checker replaces the third
precompile rank key with actual primary bytes, applies the same diversity
truncation to the complete size-valid set, and obtains exactly the preset-bounded
measured-finalist set. Every size-valid trial outside it is
`compiled-unmeasured`. Every member is either `timed-out` at its recorded point or
has the exact smoke/search/validation streams and final outcome required by this
matrix and the selection-wide table. Thus neither a compile-selected plan nor the
highest-ranked valid finalist can be silently omitted.

Candidate outcomes and `Selection.reason` obey this exact cross-record table. Let
`Q1` and `Q2` be each round's `rankedPlanDigests` after filtering to stable records
whose `thresholdPassed` is true and whose `pairedWins` is at least 16. The stored
ranked lists must equal the deterministic rank of exactly those records.

| Condition | Selection | Validation-entrant outcomes |
| --- | --- | --- |
| no validation entrants | empty-plan digest, no-candidate, no certificate | none; preserve earlier trial outcomes |
| `Q1` or `Q2` empty | empty-plan digest, validation-threshold, no certificate | all surviving completed entrants validation-threshold |
| both nonempty and first digests equal | common digest, tuned, required certificate | winner selected; all other surviving completed entrants validation-nonwinner |
| both nonempty and first digests differ | empty-plan digest, validation-disagreement, no certificate | all surviving completed entrants validation-nonwinner |

The no-entrant row takes precedence; the remaining rows require at least one
entrant. No other combination is valid. The baseline candidate retains outcome `baseline`
in every row of this table. A timed-out entrant is excluded from both Q lists and
retains `timed-out` under every selection result.

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

Every stored `RoundPlan.stable` is true because instability aborts before a
decision exists. `thresholdPassed` is true exactly when its aggregate ratio is at
most 97/100 of `2^32`, every case ratio is at most 102/100 of `2^32`, and
`pairedWins >= 16`; all comparisons use the checked integer rules in the main
design. `rankedPlanDigests` contains exactly the passing plans in the stated
validation rank, with no duplicate or omitted entrant.

These fields are derived, never asserted. For round 1 or 2, the checker selects
phase 5 or 7 streams with the same round. `RoundSummary.plans` contains exactly
every non-timeout validation entrant whose baseline and candidate stream is
complete for that round, sorted by plan digest. Each `caseMedians` list contains
exactly every validation `CaseIdentity` in case-id order. `baselineNs` and
`candidateNs` are ascending stored-minimum element 10 of the corresponding 20-row
streams; `ratioQ32` is
`ceil(candidateNs * 2^32 / baselineNs)`. `aggregateRatioQ32` is
`ceil(sum(case.weight * ratioQ32) / sum(case.weight))`, all with checked `u128`
intermediates and a `u64` result.

For each row ordinal 0..19, the checker instead uses that ordinal's stored minimum
from every validation case, computes each per-case ratio with the same ceiling,
then the same weighted aggregate. `pairedWins` is exactly the count whose aggregate
is strictly below `2^32`. `stable` is the conjunction of the attachment's 16-of-20
rule for every referenced baseline and candidate stream. `thresholdPassed` and
`rankedPlanDigests` are then rederived as above; ranking uses aggregate ratio,
candidate primary bytes, choice count, then plan digest.

The validation-entrant set is itself rederived from complete stable phase-3 search
streams: compute their weighted Q32 score with the same formulas, rank by score,
primary bytes, choice count, and plan digest, and take the preset bound. A
candidate absent from that set cannot have validation streams. A timeout at phase
4..7 proves prior entry but is excluded from both ranked qualifier lists. These
equalities connect calibration iterations, raw rows, candidates, rounds, and final
selection; changing a summary without changing its source streams is invalid.

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
| 10 | `choiceIdentityDigest` | `D32` |

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
| manifest | `CK-TUNE-MANIFEST\0`; `ManifestMaterial`: tag 1 schema `U32(1)`, tag 2 argv `List<Text,64>`, tag 3 effective environment `List<EnvironmentEntry,16>`, tag 4 timeout `U32`, tag 5 manifest-order `List<ManifestInputMaterial,64>`, tag 6 case-id-order `List<ManifestCaseMaterial,16>`, tag 7 runner bytes `U64`, tag 8 runner digest `D32` |
| target identity | `CK-TUNE-TARGET\0`; complete `TargetIdentity` record |
| pre-tune KIR | `CK-TUNE-PRE-KIR\0`; schema `U32(1)` then canonical pre-tune whole-module `Blob32M`; stored as Identity tag 18 |
| root id | `CK-TUNE-ROOT\0`; tag 1 pre-tune KIR digest, tag 2 complete `RootAnchor` |
| KIR state | `CK-TUNE-KIR-STATE\0`; `KirStateMaterial` below |
| site id | `CK-TUNE-SITE\0`; root id, class, canonical site ordinal, pre-state digest at tags 1..4 |
| alternative id | `CK-TUNE-ALTERNATIVE\0`; site id, complete `AlternativePayload`, post-state digest at tags 1..3 |
| unit id | `CK-TUNE-UNIT\0`; site-id list and baseline-state digest |
| unit variant id | `CK-TUNE-UNIT-VARIANT\0`; unit id, class, site alternatives, three isolated estimates, post-state digest |
| plan digest | `CK-TUNE-PLAN\0`; unit-id-ordered `List<PlanChoice,64>`; the empty-plan digest is this same domain over the zero-count list |
| candidate-space digest | `CK-TUNE-CANDIDATE-SPACE\0`; site-id-ordered sites and unit-id-ordered units, excluding the expansion trace |
| frontier digest | `CK-TUNE-FRONTIER\0`; candidate-space digest plus ordinal-ordered expansion trace |
| correctness aggregate | `CK-TUNE-CORRECTNESS\0`; case-id-ordered `List<CaseCorrectness,16>` where each record is tag 1 case id and tag 2 observed digest |
| object-graph digest | `CK-TUNE-OBJECT-GRAPH\0`; `ObjectGraphMaterial` below |
| link-recipe digest | `CK-TUNE-LINK-RECIPE\0`; `LinkRecipeMaterial` below |
| session digest | `CK-TUNE-SESSION\0`; `SessionMaterial` below |
| validation-round digest | `CK-TUNE-VALIDATION-ROUND\0`; complete `RoundSummary` record |
| certificate digest | `CK-TUNE-CERTIFICATE\0`; complete `Certificate` record |
| destination id | `CK-TUNE-DESTINATION\0`; complete `DestinationKeyMaterial` below |
| output-set id | `CK-TUNE-OUTPUT-SET\0`; `OutputSetMaterial` below |
| replay-result digest | `CK-TUNE-REPLAY-RESULT\0`; `ReplayResultMaterial` below |
| choice-identity digest | `CK-TUNE-CHOICE\0`; `ChoiceIdentityMaterial` below |
| compile-cache key | `CK-TUNE-COMPILE-KEY\0`; `CompileCacheKeyMaterial` below |
| compile-cache entry | `CK-TUNE-COMPILE-ENTRY\0`; `CompileCacheEntryMaterial` below |
| measurement-cache key | `CK-TUNE-MEASUREMENT-KEY\0`; `MeasurementCacheKeyMaterial` below |
| measurement-cache entry | `CK-TUNE-MEASUREMENT-ENTRY\0`; `MeasurementCacheEntryMaterial` below |

Within `UnitVariant`, `siteAlternatives` are sorted by `(siteId, alternativeId)`.
The unit-variant digest uses that sorted list. All correctness observations for the
same case must agree; a candidate aggregate contains every distinct case it
successfully executed. A profitability certificate requires all manifest cases.

Every `Site.rootId` equals the root-id derivation from its retained anchor, and its
pre-state equals the pre-tune state derivation. A `SiteAlternative.siteId` resolves
to exactly one site, repeats that site's pre-state, has a payload class equal to the
site and containing variant class, and has an alternative id equal to the stated
derivation. Each unit's site ids are unique and its baseline state equals pre-tune
state. Each variant lists exactly the alternatives it applies, and its id and
post-state equal the isolated replay derivations. Each `PlanChoice` resolves to the
named unit and one of its variants, repeats that variant's class, and obeys the
sequential state equalities below.

`KirStateMaterial` is tag 1 schema `U32(1)`, tag 2 canonical whole-module KIR as
`Blob32M`. The bytes are exactly `print_kir_module` with the existing KIR schema's
stable symbol order and without addresses, hash-map order, diagnostics, paths, or
timestamps. Every site pre-state and every unit baseline state equals the digest of
the unchanged pre-tune module. A site-alternative post-state is the digest after
applying only that alternative to a fresh pre-tune module; a unit-variant post-state
is after applying exactly its ordered site alternatives to a fresh module.

The pre-tune module is the verified v0.13 O3 state after CFG canonicalization,
initial SCCP/range analysis, loop canonicalization, and the first mandatory check
elimination, but before specialization or any other profitability-controlled O3
rewrite. A complete plan is applied by the fixed phase order specialization,
inlining, short-slice/versioning, loop-SIMD, unrolling, SLP, then layout. Units sort
by `(phase, unitId)` and alternatives within a unit sort by `(siteId,
alternativeId)`. Between phases, the ordinary mandatory analysis, legality,
cleanup, and proof-refresh passes run at their v0.13 positions; they are not plan
choices. Layout choices attach canonical layout metadata to KIR before native
lowering, and LLVM consumes it only after the fixed LLVM O3 pipeline and before
object emission. Thus the canonical KIR contains the layout intent even though its
machine-level effect is late. The empty plan executes the unmodified v0.13 O3
decisions and never changes existing ordinary or O2 late-layout behavior.

For a plan, choices are stored in the same `(phase, unitId)` application order.
Each `PlanChoice.preStateDigest` is the full-module state immediately before the
fixed pipeline reaches that chosen unit. Its `postStateDigest` is the state after
applying the unit and then executing every deterministic ordinary decision and
mandatory bridge pass up to, but not including, the next chosen unit; the final
choice also executes the remaining KIR pipeline through canonical cleanup and
layout-metadata attachment. The first pre-state equals Replay tag 2, adjacent
post/pre-state digests are equal, and the last post-state equals Replay tag 3.
For an empty plan, Replay tags 2 and 3 both equal the pre-tune state. The plan is
always reapplied from a fresh pre-tune module; these equalities are independently
recomputed rather than trusted.

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
destinations, and the measurement-cache salt digest. Its result is stored as
Environment tag 18.

`ParentIdentity` is tag 1 `platform: U8` (posix=1, windows=2), tag 2 `volume: U128`,
tag 3 `file: U128`, and tag 4 `lookup: U8` (case-sensitive=1,
ascii-case-insensitive=2). POSIX stores unsigned device and inode values in the low
64 bits with zero high bits. Windows stores volume serial in the low 64 bits and
the full 128-bit directory file id. The parent is opened no-follow and these values
come from that handle; an unavailable/unstable identity or unknown lookup behavior
is a hard error.

`DestinationKeyMaterial` is tag 1 the complete `ParentIdentity` and tag 2
`lookupLeaf: Text`. Tune destinations use only the ASCII-safe leaf grammar in the
main design. A case-sensitive parent stores exact leaf bytes; an ASCII-case-
insensitive parent stores ASCII lowercase. On Windows an existing entry's
`lookupLeaf` is always derived from the authoritative long leaf obtained from its
opened handle, even when the requested spelling was a manually assigned short
name; unavailable or inconsistent long/short-name discovery fails closed. An
absent entry uses the requested legal leaf and is re-resolved after lock acquisition
before any mutation. `OutputSetMaterial` is tag 1 output kind,
tag 2 decision destination id, and tag 3 a role-sorted list whose records contain
output role and destination id. Duplicate detection, lock identity, journal
membership, and output-set identity all use these same ids. This operational
material is journal-only, so it does not put absolute paths in a decision `Text`.

`ReplayResultMaterial` is tag 1 frontier digest, tag 2 selected plan digest, tags
3..4 selected pre/post-state digests, tags 5..6 object-graph/link-recipe digests,
and tag 7 the role-sorted `OutputIdentity` list. It excludes cache origin and local
absolute paths.

`ChoiceIdentityMaterial` is tag 1 the complete `Identity`, tag 2 `Contract`, tag 3
the `Workload.manifestDigest`, tag 4 the `EnvironmentSeed` from `SessionMaterial`,
tag 5 the frontier digest, tag 6 the selection reason, tag 7 the selected plan
digest, and tags 8..9 the selected object-graph and link-recipe digests. It binds
every input that may change the chosen code, while excluding calibration,
correctness observations, raw samples, timeouts, cache origins, output paths,
timestamps, logical basenames, and container/output bytes. Replay tag 10 must equal
this derivation. It is
the cross-session identity used to compare independent cold searches; the outer
decision digest remains the identity for exact byte-for-byte warm reuse.

`CompileCacheKeyMaterial` is tag 1 schema `U32(1)`, tag 2 the complete `Identity`,
and tag 3 plan digest. `CompileCacheEntryMaterial` is tag 1 the compile-key digest,
tag 2 primary-artifact digest, tag 3 primary-artifact bytes, tag 4 object-graph
digest, and tag 5 link-recipe digest. Every candidate's `compileOrigin.keyDigest`
and `entryDigest` equal these derivations from that candidate; `freshly-built` and
`verified-local-hit` describe how the identical entry was obtained. Replay tag 7
equals the compile origin of the selected candidate, including the baseline when
the empty plan is selected. Replay output primary digest/bytes equal that candidate's
primary-artifact digest/bytes; Replay object-graph/link-recipe digests equal the
same candidate fields. A tuned certificate repeats the selected plan, object graph,
link recipe, frontier, policy, validation-round, and all-case correctness digests.

`MeasurementCacheKeyMaterial` is tag 1 schema `U32(1)`, tag 2 session digest, and
tag 3 measurement-cache-salt digest. It is fully known before candidate execution.
`MeasurementCacheEntryMaterial` is tag 1 that key digest, tag 2 the
complete `Candidates` record, and tag 3 the complete `Selection` record. Replay
tag 8 stores exactly those derived key/entry digests. Neither cache entry digest
includes Replay, so the derivations are acyclic. All cache hits rehash and validate
the entry before use; a mismatch is a miss followed by quarantine, never a partial
hit.

The policy digest remains `H("CK-TUNE-POLICY\0", Contract tags 1..31)`. The outer
decision digest remains the framing rule in Section 1.

## 12. Enums and canonical ordering

| Enum | Values |
| --- | --- |
| `OutputKind` | executable=1, dynamic=2 |
| `Budget` | quick=1, standard=2, thorough=3 |
| `CaseRole` | search=1, validation=2 |
| `AlternativeClass` | inlining=1, specialization=2, unrolling=3, loop-SIMD=4, SLP=5, short-slice/versioning=6, layout=7 |
| `RootKind` | module=1, function=2, loop=3, block=4, instruction=5, call=6 |
| `InliningAction` | force-inline=1, keep-out-of-line=2 |
| `SpecializationValueKind` | u32=1, u64=2, i32=3, i64=4, f32-bits=5, f64-bits=6, length-u32=7 |
| `LayoutScope` | block=1, function=2, section=3 |
| `ExpansionDisposition` | legal=1, illegal=2, duplicate=3, growth-rejected=4 |
| `CandidateOutcome` | baseline=1, compiled-unmeasured=2, size-rejected=3, timed-out=4, search-nonwinner=5, validation-threshold=6, validation-nonwinner=7, selected=8 |
| `OrderingPhase` | candidate-smoke=1, search-warmup=2, search-measured=3, validation-one-warmup=4, validation-one-measured=5, validation-two-warmup=6, validation-two-measured=7 |
| `SelectionReason` | tuned=1, no-candidate=2, validation-threshold=3, validation-disagreement=4 |
| `OutputRole` | primary=1, header=2, import-library=3 |
| `CacheOriginKind` | freshly-built=1, verified-local-hit=2 |
| `DiagnosticCode` | none=0, legality-rejected=1, growth-rejected=2, artifact-size-rejected=3, candidate-timeout=4 |

Cases sort by id; sites and variants by stable id; units and choices by
`(application phase, unit id)`; site alternatives by site id then alternative id;
expansions by ordinal;
trials by plan digest; streams by phase, round, case id, and plan
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
- `tests/fixtures/tune/decision-schema1-inspection.json` for the tuned vector;
- `tests/fixtures/tune/decision-schema1-inspection.txt` for the same tuned vector.

The test source pins each fixture SHA-256. The same files drive encode, decode,
inspect, re-encode byte equality, mutation, truncation, limit, and cross-endian
tests. A golden vector change requires a future format schema.

The exact public JSON and text inspection renderings are defined by
[`inspection-schema-1.md`](inspection-schema-1.md). The inspection JSON fixture is
not a substitute for that schema; it and the required text fixture are generated
from the tuned decision vector and then frozen by digest.
