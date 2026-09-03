# CK 0.14 Predicated-Update Performance Contract 1

Status: normative attachment to
[`offline-autotuning.md`](offline-autotuning.md). This contract is independent of
performance report schema 9 and does not change `CKTUNE01`, tuning decision schema
1, the language, or any public/native/runtime ABI.

## 1. Authority and outcome

The two required stable Linux performance jobs each produce and check one
`target/ckc-perf/v0.14-predicated-update-results.json` report. Collection and
acceptance are separate:

```sh
cargo bench --features native-toolchain --bench tune_perf -- \
  --task collect-predicated-update \
  --out target/ckc-perf/v0.14-predicated-update-results.json
python3 scripts/check-v014-predicated-update.py \
  target/ckc-perf/v0.14-predicated-update-results.json \
  --schema-nine target/ckc-perf/v0.14-results.json
```

The collector records evidence but cannot declare success. The checker is the
sole acceptance authority and rejects missing, extra, reordered, noncanonical,
unstable, mismatched, unretained, or ineligible evidence. Passing this contract
requires all of the following on each host:

1. source-aware replay proves that the selected non-baseline plan contains the
   predicated same-place update alternative for the `floyd` inner loop;
2. validation `pgoTuned/pgoOnly` is at most `102/100`;
3. sealed release `pgoTuned/pgoOnly` is at most `95/100`;
4. both channels pass the fixed stability and correctness rules;
5. exact replay reproduces the tuned output set and attestation.

No geometric aggregation across hosts, selective rerun, case deletion, or
trade-off against another v0.14 gate is permitted.

## 2. Frozen repository assets and recipe

The recipe contains exactly these candidate-SHA repository files:

```text
benches/fixtures/tune/predicated_update.ck
benches/fixtures/tune/predicated-update-training.tsv
benches/fixtures/tune/predicated-update-validation.tsv
benches/fixtures/tune/predicated-update-release.tsv
benches/tune/workloads/predicated-update.cktune.toml
benches/tune/runner.rs
benches/tune_perf.rs
scripts/measure-v014-predicated-update.py
scripts/check-v014-predicated-update.py
specs/0.14/offline-autotuning.md
specs/0.14/predicated-update-performance-1.md
LICENSE
THIRD_PARTY_NOTICES.md
```

Each file is a schema-9 `FileIdentity` rooted at `repository`, sorted by UTF-8
path. The recipe object has exactly `schema`, `files`, `thresholds`, and `digest`.
`schema` is 1. `thresholds` has exactly the following U64 entries, sorted by key:

| Key | Value |
| --- | ---: |
| `callsPerRow` | 3 |
| `measuredRows` | 20 |
| `releaseMaximumDen` | 100 |
| `releaseMaximumNum` | 95 |
| `stabilityLowerDen` | 100 |
| `stabilityLowerNum` | 80 |
| `stabilityRequiredRows` | 16 |
| `stabilityUpperDen` | 100 |
| `stabilityUpperNum` | 120 |
| `validationMaximumDen` | 100 |
| `validationMaximumNum` | 102 |
| `warmupRows` | 3 |

Using the schema-9 primitive `P`, its digest is:

```text
P("CK-V014-PRED-RECIPE\0",
  schema U32,
  files List<FileIdentityValue>,
  thresholds List<ThresholdEntry>)
```

Any recipe byte, path, threshold, compiler identity, or runner identity change
invalidates earlier evidence.

### 2.1 Exact CK source

`benches/fixtures/tune/predicated_update.ck` is exactly the following UTF-8 bytes,
with LF line endings and one final LF:

```ck
export unsafe fn floyd(distance: slice<f64>, n: u32) -> void
contract {
  requires n <= 65535;
  effects readwrite(distance);
}
{
  let k: u32 = 0;
  while k < n {
    let k_row: u32 = k * n;
    let i: u32 = 0;
    while i < n {
      let i_row: u32 = i * n;
      let dik: f64 = distance[i_row + k];
      let j: u32 = 0;
      while j < n {
        let index: u32 = i_row + j;
        let candidate: f64 = dik + distance[k_row + j];
        let old: f64 = distance[index];
        if candidate < old {
          distance[index] = candidate;
        }
        j = j + 1;
      }
      i = i + 1;
    }
    k = k + 1;
  }
}
```

### 2.2 Exact tune manifest

`benches/tune/workloads/predicated-update.cktune.toml` is exactly the following
UTF-8 bytes, with LF line endings and one final LF:

```toml
schema = 1

[runner]
path = "../../../target/release/ckc-tune-runner"
input_root = "../.."
args = ["--ck-predicated-tune"]
inputs = ["fixtures/tune/predicated-update-training.tsv", "fixtures/tune/predicated-update-validation.tsv"]
inherit_env = []
timeout_ms = 30000

[[case]]
id = "predicated-update.search"
role = "search"
seed = 113
weight = 1
expected_digest = "42c6b833bf2207f5d0716d249099daf28dcf0250e63dbd2a9a4f438a10a215af"

[[case]]
id = "predicated-update.validation"
role = "validation"
seed = 127
weight = 1
expected_digest = "8b9f2194f5fe7afdfd1d856689ac288d04b70bf984f2310e7011d2ced391aa10"
```

## 3. Inputs and exact generator

The three TSV files have exactly one header and one data row:

```text
ckc-predicated-inputs\t1\ttraining
predicated-update\ttrain-floyd-128\t128\t113
```

```text
ckc-predicated-inputs\t1\tvalidation
predicated-update\tvalidate-floyd-256\t256\t127
```

```text
ckc-predicated-inputs\t1\trelease-held-out
predicated-update\trelease-floyd-1024\t1024\t131
```

There is one LF after every row and no BOM, comment, blank row, trailing field,
or trailing byte. Integers are canonical unsigned decimal. The tune manifest
declares only the training and validation files; it cannot name, read, or derive
the release file.

The runner and independent scalar oracle generate an `N*N` row-major binary64
matrix. All U64 arithmetic below wraps modulo `2^64`:

```text
splitmix64(x):
    z = x + 0x9e3779b97f4a7c15
    z = (z xor (z >> 30)) * 0xbf58476d1ce4e5b9
    z = (z xor (z >> 27)) * 0x94d049bb133111eb
    return z xor (z >> 31)

cell(i, j, n, seed):
    if i == j: return +0.0
    r = splitmix64(seed xor (U64(i) << 32) xor U64(j))
    if j == (i + 1) mod n: return f64(1 + (r mod 16))
    if ((r >> 8) mod 4) == 0: return +infinity
    return f64(1 + (r mod 1024))
```

`n` is nonzero, at most 1,024, and multiplication to `n*n` is checked before
allocation. Integer-to-binary64 conversions above are exact. The ring edge makes
the graph strongly connected; inputs contain no negative value, negative cycle,
NaN, or negative zero. The scalar oracle runs the source-order `k/i/j` algorithm
with strict binary64 add and ordered less-than, no contraction, fast math, or
parallelism.

For split name `S`, the expected result digest is:

```text
SHA-256("CK-V014-PRED-RESULT\0" ||
        U32_BE(n) || U64_BE(n*n) ||
        for each row-major result: U64_BE(f64.to_bits()))
```

The report stores all three expected digests. The checker regenerates them from
the frozen recipe and rejects channel agreement that does not equal the scalar
oracle.

The frozen expected result digests are:

| Split | Digest |
| --- | --- |
| training | `d6105453012eedb8a8db812555f116dd69ca6a8e0242faf81f10038947581608` |
| validation | `e21128a8623d0c072111b02c8a3f1ce3309d12f8bf0b3beddc1e0f6342dfc6c8` |
| release-held-out | `4d9a1612967ec78ffb3d0ecc035929bb8217cefc13dfeb3fb2e21989186b055d` |

For the schema-1 tuning receipts, the canonical result bytes are the same
row-major `U64_BE(f64.to_bits())` sequence. Applying the existing
`CK-TUNE-RESULT` digest with case ids `predicated-update.search` and
`predicated-update.validation` yields the two manifest constants in Section 2.2.

## 4. Runner boundaries

`benches/tune/runner.rs` retains its schema-1 tuning protocol and adds four closed
argument protocols:

```text
ckc-tune-runner --ck-predicated-tune

ckc-tune-runner --ck-predicated-profile
  <generation-library> <flush-symbol> 128 113

ckc-tune-runner --ck-predicated-oracle
  <training|validation|release-held-out> <n> <seed>

ckc-tune-runner --ck-predicated-perf
  <library> <validation|release-held-out> <n> <seed> <iterations>
```

Arguments are passed directly without a shell. The profile and performance
protocols receive an empty environment; the tuning protocol receives only the
closed schema-1 `CK_TUNE_*` environment. Unknown/missing/extra arguments fail.
The profile protocol loads the library,
generates one training matrix, calls exported `floyd(slice<f64>, u32)` once,
checks its digest, calls the exact flush symbol once after all CK calls quiesce,
requires status zero, and emits exactly:

```text
CKPREDPROFILE/1 128 113 <result-digest> <flush-status>\n
```

The performance protocol loads and resolves the artifact, checks `n/seed` against
the named frozen split, preallocates `iterations` independent fresh matrices, and
only then starts `CLOCK_MONOTONIC_RAW`. It calls `floyd` exactly once on each
matrix, stops the clock immediately after the last return, and checks every result
against the scalar oracle outside the timed region. Allocation, initialization,
dynamic loading, symbol lookup, digesting, and output are excluded. It emits
exactly:

```text
CKPREDPERF/1 <split> <n> <seed> <iterations> <completed> <elapsed-ns> <digest>\n
```

The oracle protocol runs the independent scalar implementation once and emits
exactly:

```text
CKPREDORACLE/1 <split> <n> <seed> <digest>\n
```

`iterations`, `completed`, and elapsed nanoseconds are positive U64 values;
completed equals iterations. Arithmetic/allocation overflow, a nonmatching result,
nonfinite unexpected result, loader error, or protocol mismatch is fatal. The
collector sets a 1 GiB per-invocation matrix cap and never shortens a requested
iteration count to fit it.

The schema-1 tune manifest uses the same runner with args
`["--ck-predicated-tune"]`. Search is exactly `N=128, seed=113`; validation is
exactly `N=256, seed=127`. Each tuning invocation also uses independent fresh
matrices and returns the existing exact `CKTUNE/1` receipt. The runner rejects a
release record in the CKTIMAP1 snapshot.
All direct protocol `<digest>` fields equal the applicable frozen result digest
from Section 3; the tuning protocol instead emits its case-bound `CK-TUNE-RESULT`
digest as required by schema 1.

## 5. Exact build graph

All commands use repository- or evidence-relative paths, no shell, and the clean
candidate repository as working directory. `C` is the retained candidate `ckc`;
`S` the retained CK source; `R` the retained runner; `E` the relative evidence-root
path. The collector substitutes those four tokens literally and records the
resulting argv. Output arguments below are logical bases resolved through
`NativeArtifactPaths` for the host. Argument order is normative. Runner commands
have an empty environment. Each CK build or tune command has exactly one
`XDG_CACHE_HOME` entry naming a distinct initially absent directory below `E/cache`;
merge and inspect use an empty environment. No other variable is inherited.

| Name | Exact argv |
| --- | --- |
| profile generation | `C build S --out E/build/generation/artifact --kind dynamic --cpu native -O3 --overflow unchecked --bounds unchecked --pgo-generate E/profile/shards` |
| training run | `R --ck-predicated-profile <generation-primary> <flush-symbol> 128 113` |
| profile merge | `C pgo merge <profile-shard> --out E/profile/predicated.ckprof` |
| profile inspect | `C pgo inspect E/profile/predicated.ckprof --json` |
| PGO-only | `C build S --out E/build/pgo-only/artifact --kind dynamic --cpu native -O3 --overflow unchecked --bounds unchecked --pgo-use E/profile/predicated.ckprof` |
| tuned | `C tune build S --config <manifest> --out E/build/pgo-tuned/artifact --kind dynamic --cpu native -O3 --overflow unchecked --bounds unchecked --pgo-use E/profile/predicated.ckprof --budget standard --tune-out E/build/pgo-tuned/decision.cktune --no-tune-cache --explain-optimization` |
| exact replay | `C build S --out E/build/replayed/artifact --kind dynamic --cpu native -O3 --overflow unchecked --bounds unchecked --pgo-use E/profile/predicated.ckprof --tune-use E/build/pgo-tuned/decision.cktune --explain-optimization` |

The generation header supplies exactly one
`ck_profile_flush_<64-lowercase-hex>` declaration; `<flush-symbol>` is that name
and must resolve in the generation primary. Every CommandEvidence retains executable,
inputs, environment, complete platform-resolved outputs, primary output, status,
stdout, and stderr identities. Every status is zero. The profile has exactly one
completed shard and its inspected compiler/source/KIR/site-table/target/mode
identity equals all three PGO-use builds.

Before profile generation, the collector creates `E/profile/shards` as a new empty
directory, opens it with Linux `O_RDONLY|O_DIRECTORY|O_NOFOLLOW`, and retains that
descriptor through profile generation and the training run. `<profile-shard>` is
the path of the sole regular no-follow file discovered through that descriptor
after the flush succeeds. The merge command consumes that exact file, not a
directory or a newly enumerated spelling. Section 8 binds the directory descriptor,
its empty pre-state, and its one-file post-state.

The `E/cache` namespaces are private compiler scratch, not artifact outputs. The
collector records the initially absent namespace, lets only its associated compiler
command populate it, and removes that namespace after the command has returned and
all declared outputs have been opened and retained. No cache path may escape its
namespace or alias an evidence member. Cache files therefore do not survive into
the closed evidence-directory inventory. Persistent publication lock files are not
scratch and are retained as required by Section 8.

The PGO-only and tuned channels share byte-identical compiler, source, profile,
target, CPU features, optimization, overflow, and bounds identities. Their only
optimizer-policy difference is the measured tuning plan. Replayed primary/header
bytes equal the tuned output set byte-for-byte.

## 6. Source-aware attestation

`--explain-optimization` on tuned and replay builds emits one canonical line only
after the independent vector checker accepts the selected rewrite:

```text
CKTUNE-ATTEST/1 shape=predicated-same-place-update function=floyd \
header=<u32> compare=<u32> load=<u32> store=<u32> \
unit=<64hex> variant=<64hex> alternative=<64hex> \
vectorBits=<u32> uf=<u32> minimum=<u32> pre=<64hex> post=<64hex>\n
```

The actual line has one ASCII space between fields and no continuation. Decimal
numbers are canonical and positive except block/instruction ids, which may be
zero. Digests are lowercase. There is exactly one such line in each retained
stderr stream; unrelated diagnostics may surround it but cannot start with
`CKTUNE-ATTEST/`.

The source-aware checker independently reconstructs the pre-tune KIR and verifies
that the named function/header contains the exact scalar compare, dominating
same-place load, conditional store, Memory SSA relation, affine/dependence facts,
and no ordered-effect violation. It independently verifies the post-state contains
the corresponding vector compare, select(candidate, old), one unmasked vector
store, runtime legality guard when required, and scalar epilogue. `unit`,
`variant`, `alternative`, vectorBits/UF/minimum, and pre/post digests must equal the
selected decision record and replay reconstruction. Tuned and replay attestation
lines are byte-identical. A different Loop SIMD site cannot satisfy this rule.

This diagnostic adds no `CKTUNE01` field. The existing stable site and alternative
identities commit to the scalar root and site ordinal, Loop SIMD payload, and
complete post-state digest. The shape label is diagnostic only; source-aware
reconstruction is what proves it. Replay rejects a decision if the scalar site,
payload, or rewritten state changes.

## 7. Sampling

Validation and release-held-out are measured separately. Each split first
calibrates `pgoOnly`, beginning at one iteration and doubling with checked U64
arithmetic until elapsed time is at least 50,000,000 ns, with at most 32 attempts.
One confirmation receipt at that iteration count must also reach 50,000,000 ns.
The same selected count is used by both channels for all rows in the split.

For phase `warmup` rows `0..2` and phase `measured` rows `0..19`, channel order is
the left rotation of `[pgoOnly,pgoTuned]` by:

```text
FIRST_U64_BE(SHA-256("CK-V014-PRED-ORDER\0" ||
                    candidateSha Text ||
                    split Text || phase Text || row U32_BE)) mod 2
```

`FIRST_U64_BE` interprets the first eight digest bytes as one unsigned big-endian
integer. `candidateSha` uses the schema-9 `Text` encoding of its 40 lowercase
ASCII Git SHA-1 characters; it is not a 32-byte `DigestBytes` value. All `Text`
values use the schema-9 length-prefixed UTF-8 primitive. Each channel is invoked
three times per row in its scheduled position. A row sample is the minimum of its
three elapsed values. Warm-up receipts are retained but not scored. The median is
ascending measured sample index 10. At least 16 of 20 samples in each channel and
split must satisfy `median*80 <= sample*100 <= median*120`, using arbitrary-size
integer arithmetic.

Validation passes only if
`validation.pgoTuned.medianNs*100 <= validation.pgoOnly.medianNs*102`.
Release passes only if
`release.pgoTuned.medianNs*100 <= release.pgoOnly.medianNs*95`.
All ratios use integer cross multiplication; floating-point comparison and rounded
percentages are non-authoritative.

## 8. Closed report

The UTF-8 JSON report uses integer JSON numbers only within exact U64 range and has
exactly these top-level keys:

```text
schemaVersion, candidateVersion, candidateSha, evidenceDirectory,
toolchain, hardware, recipe, compiler, runner, source, inputs,
profile, manifest, decision, attestation, artifacts, publicationLocks, commands,
correctness, validation, release
```

`schemaVersion` is 1; `candidateVersion` is `0.14.0`; `candidateSha` is 40 lowercase
hex and equals the clean checkout. `evidenceDirectory` is the sibling directory
`v014-predicated-update-<unix-seconds>-<pid>` and contains no symlink.
`toolchain`, `hardware`, `FileIdentity`, `CalibrationAttempt`, and `CallReceipt`
have exactly the definitions in
[`performance-schema-9.md`](performance-schema-9.md). Hardware must pass the same
Linux x86-64-v4 or AArch64-SVE2 gate as its enclosing stable job. The required
`--schema-nine` report is independently checked first; its candidate SHA,
candidate compiler bytes, toolchain object, and hardware object must respectively
equal this report's candidate SHA, `compiler` bytes, toolchain, and hardware.

A `CommandEvidence` has exactly `argv`, `workingDirectory`, `executable`, `inputs`,
`environment`, `outputs`, `status`, `stdout`, and `stderr`. `argv` is the exact
string vector from Section 5 after token substitution; `workingDirectory` is
`repository`; `executable`, inputs, outputs, stdout, and stderr use FileIdentity;
inputs/outputs are path-sorted lists. `status` is U32 zero. `environment` is an
ordered list of objects with exactly `name`, `value`, and `references`. It is empty
except for the exact build/tune compiler-cache entry required in Section 5; that entry has
name `XDG_CACHE_HOME`, a repository-relative value below the evidence directory,
and empty references. Stdout/stderr identities retain exact bytes, including an
empty file. Resolve `argv[0]` relative to `workingDirectory` without following
symlinks; the resolved regular file's bytes and size equal `executable`. Top-level
compiler and runner identities are immutable evidence-root copies, while source,
manifest, recipe, and fixed input identities are repository-root identities;
generated profiles, decisions, artifacts, locks, receipts, and streams are
evidence-root identities. Every file-valued path argument has one matching input
or output identity. The profile-generation directory argument uniquely matches
`profile.directory`. No undeclared file may survive in an artifact, profile, or
publication location; Section 5 is the only cache-scratch exception.

A `DirectorySnapshot` has exactly `entries`, `digest`, and `receipt`. `entries` is
a path-sorted FileIdentity list. Its digest is

```text
P("CK-V014-PRED-DIRECTORY\0", phase Text,
  entries List<FileIdentityValue>)
```

where `phase` is exactly `before` or `after`. `receipt` is an evidence-root
FileIdentity whose UTF-8 bytes are exactly one LF-terminated line:

```text
CKPREDDIR/1 phase=<phase> device=<device> inode=<inode> count=<count> digest=<64hex>\n
```

Numbers are canonical U64 decimal and the digest is the snapshot digest. A
`DirectoryEvidence` has exactly `root`, `path`, `device`, `inode`, `before`, and
`after`. Root is `evidence`, path is `profile/shards`, and device/inode are the U64
Linux `fstat` values from the retained descriptor. The checker requires the path
to resolve no-follow to that same directory identity after collection. `before`
has no entries; `after` has exactly `profile.shards[0]`. Both snapshots are taken
by descriptor, and no directory entry other than that shard may occur.

A `PublicationLock` has exactly `destination`, `destinationId`, and `file`.
`destination` and `file` are evidence-root FileIdentity values. `destinationId`
is D32. The checker recomputes it from the destination using Publication Journal
Schema 1, requires the file basename `.ckc-tune-dest-<destinationId>.lock`, and
checks its exact `CKTLCK01 || destinationId` bytes and owner-only-write regular
no-follow mode. Rows are sorted by lock path and contain exactly one row for every
tuned or replayed decision/artifact destination that that publication protocol
locks; duplicate destinations or lock files fail.

The remaining objects are closed as follows:

- `compiler`, `runner`, `source`, and `manifest` are FileIdentity values;
- `inputs` has exactly `training`, `validation`, and `release`, each a
  FileIdentity;
- `profile` has exactly `directory`, `shards`, `final`, `identityDigest`, and
  `inspection`; directory is the DirectoryEvidence above, shards is a one-element
  FileIdentity list, final and inspection are FileIdentity, and identityDigest is
  the profile identity digest parsed from both final bytes and canonical inspection
  JSON;
- `decision` has exactly `file`, `decisionDigest`, `planDigest`, and `selected`;
  selected is true, file is FileIdentity, decisionDigest equals its SHA-256 digest,
  and planDigest is the selected plan D32 decoded from the decision;
- `attestation` has exactly `tuned`, `replayed`, and `digest`; tuned/replayed are
  FileIdentity values containing the exact line from Section 6, their bytes are
  equal, and digest is
  `SHA-256("CK-V014-PRED-ATTEST\0" || U64_BE(len(line)) || line)`;
- `artifacts` has exactly `generation`, `pgoOnly`, `pgoTuned`, and `replayed`;
  each value has exactly `primary` and `outputs`; primary is a FileIdentity and
  outputs is a nonempty list of objects with exactly `role` and `file`, sorted by
  role `primary`, `header`, then `importLibrary`; primary equals the primary-role
  file, and outputs equal the platform-resolved dynamic-library set;
- `publicationLocks` is the path-sorted nonempty PublicationLock list defined
  above and has exactly the tuned and replayed publication-lock rows;
- `commands` has exactly `profileGeneration`, `trainingRun`, `profileMerge`,
  `profileInspect`, `pgoOnly`, `pgoTuned`, and `replayed`, each a CommandEvidence
  from Section 5;
- `correctness` has exactly `training`, `validation`, `release`, and
  `oracleCommands`; the first three are D32 expected-result digests and
  oracleCommands has exactly those three keys, each containing one empty-environment
  CommandEvidence for the Section 4 oracle protocol with matching n/seed/digest;
- `validation` and `release` are TimingSplit values.

The command graph is closed by exact foreign keys. Profile-generation command
outputs equal `artifacts.generation`; the profile-generation directory argument
equals `profile.directory.path`; training-run inputs include that primary and
header and its sole declared output equals `profile.shards[0]`; profile-merge
consumes that exact shard file and produces `profile.final`; profile-inspect consumes the final profile and
its stdout equals `profile.inspection`. The three PGO-use commands consume the same
profile and source. Their platform outputs equal their same-name artifact rows;
the tuned command additionally produces `decision.file` and its PublicationLock
files, while replay produces its PublicationLock files. Replay consumes that
decision, and tuned/replay outputs with each common artifact role are byte-for-byte
equal. Compiler and runner command executables equal the top-level FileIdentity
values.

A TimingSplit has exactly:

```text
split, n, seed, expectedDigest, calibration,
calibrationCommands, confirmationCommand,
warmupOrder, sampleOrder, warmupCommands, sampleCommands,
warmupReceipts, callReceipts,
callsNs, samplesNs, mediansNs, ratioNum, ratioDen
```

`split` is `validation` or `release-held-out`; `n/seed` equal Section 3;
`expectedDigest` is D32. `calibration` has exactly `channel`, `attempts`,
`selectedIterationsPerCall`, and `confirmation`, with channel `pgoOnly` and the
Section 7 rules. `calibrationCommands` contains one CommandEvidence for every
attempt in order and `confirmationCommand` contains the confirmation invocation;
their runner argv, receipt bytes, iterations, elapsed time, and digest equal the
corresponding calibration records. Orders are 3 and 20 arrays of exact two-channel permutations.
`warmupCommands` and `sampleCommands` have exactly the two channel keys and contain
3 and 20 rows respectively, each row containing exactly 3 CommandEvidence objects
in actual invocation order. Every such command uses the Section 4 performance
protocol, the matching channel artifact, split/n/seed, and the selected iteration
count; its environment and outputs are empty and its stdout contains exactly the
matching receipt.
Receipt/call maps have exactly the channel keys; warm-up contains 3 rows and
measured contains 20 rows, each with exactly 3 CallReceipt or elapsed-U64 entries.
Each callsNs entry equals its receipt elapsed time. Samples are their row minima;
medians are upper medians. `ratioNum` is the pgoTuned median and `ratioDen` the
pgoOnly median without reduction.

Every report FileIdentity rooted at `evidence` names a regular no-follow file
strictly below the evidence directory. The checker derives one path-sorted
inventory from all evidence-root FileIdentity occurrences, requires repeated
occurrences to be byte-identical, and requires that inventory to equal every
regular file below the evidence directory. Repository identities resolve only in
the clean candidate checkout. Path traversal, conflicting duplicate identity,
unlisted file, wrong digest/size/root, unknown key, nonzero status, or mismatched
foreign key fails.

## 9. Checker and CI closure

The checker first invokes the schema-9 checker on the exact `--schema-nine` path
and requires success. It then rederives the recipe, generator golden cells and
frozen result digests, verifies all three retained oracle commands, and checks artifact
paths, command argv/environment, profile identity, decision and plan identities,
attestation equality, tuned/replay output equality, schedule, receipts, samples,
medians, stability, and integer thresholds. Mutation tests independently corrupt
every top-level object, each attestation binding, both ratio operands, order,
receipt counts, profile/decision/artifact foreign keys, and evidence closure.

The existing two stable performance jobs run schema 9 first and this contract
second at the same candidate SHA, passing the just-accepted schema-9 report through
the required `--schema-nine` option. The job uploads both reports and both complete
evidence roots even on checker failure. The existing ten-job topology is unchanged;
no eleventh job, skipped capability, `continue-on-error`, or successful schema-9
result can substitute for this gate.
