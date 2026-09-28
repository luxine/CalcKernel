# CalcKernel Performance Guide

[简体中文](../zh-CN/guides/performance.md)

CalcKernel 0.15 retains the fail-closed performance report schema 8. A formal release
requires complete reports from fixed x86-64 and AArch64 workers; a local build
or release-candidate identity does not sign those gates. Measurements bind the
candidate SHA, exact 0.12 replay SHA, LLVM/Clang 22.1.8, Rust 1.90.0, hardware
and capability manifests, compiler/oracle/source/recipe digests, training and
held-out corpora, profile shards/final profile, target sets, variant objects,
artifact bytes, sample order, and every raw sample.

The exact ordinary-regression replay is CalcKernel 0.12 commit
`e1bcea461492a5a2619cdb960ea00dd668847f0a`. Clang and Rust PGO oracles receive
the same training/evaluation split and source-level preconditions as CK, disable
fast math/contraction, and pass differential plus undefined-behavior audits.
Training data is never timed as held-out evidence. Correctness also includes a
separate adversarial corpus.

The 0.10, 0.11, and 0.12 replay source trees are checksum-pinned product
archives under `benches/baselines/sources/`. A fresh clone does not need the old
repository history: the preparer validates each archive and its frozen input
digests, then reconstructs a temporary local baseline tree for the approved
adapter and source-diff checks. Replay reports retain the original source
commit identities from the baseline manifests.

## Cross-language comparison

The standalone [cross-language benchmark](../../benches/cross-language/README.md)
compares checked-in CalcKernel, C++, Rust, JavaScript, Java, and NumPy kernels on
the same deterministic inputs. Its runner verifies output hashes before timing
and records source hashes, tool versions, flags, sample order, and raw
measurements. Run it from a fresh compiler checkout using the dependencies
listed in that benchmark's README.

## Sampling protocol

Every timed channel uses identical source mode, input, batch, process, and CPU
policy. Dynamic loading, symbol lookup, and dispatch resolution occur before
steady-state timing; the report proves that resolver execution happened once.
Channels rotate through fixed warm-up and sample schedules, retain every actual
order/sample, use the upper median, and apply the closed stability rule. A
stability failure invalidates the evidence; it does not authorize arbitrary
reruns or deletion of a case. Missing/unknown/extra/mismatched report fields,
digests, streams, tiers, or capabilities fail the checker.

The cumulative 0.12 vector/domain replay uses
`interleaved-upper-median-three-channel-v3`. All three channels use one shared data
workspace so allocation placement cannot masquerade as a code-performance difference. Every retained row interleaves seven
rotations of candidate/C/Rust and stores each channel's upper median. For
`slp_quad` only, the unchanged 16-of-20 stability band is evaluated after
per-row common-mode normalization; throughput still uses raw retained durations.

Hand-written oracles use architecture-specific baseline flags, disable fast
math and contraction, and may not use a CPU feature absent from CK's baseline
profile. They receive every equivalent source-language precondition and must
pass differential and undefined-behavior auditing over the fixed declared valid
domain. Missing, invalid, or post-measurement-excluded competitors fail the gate.

## Cumulative release gates

- Ordinary no-PGO candidate baseline/native versus exact 0.12 replay: geometric-mean
  slowdown at most 2%, individual slowdown at most 5%.
- PGO use versus the matching candidate ordinary CPU policy: geometric-mean improvement
  at least 5%, with held-out individual slowdown at most 3%. Generation
  execution is at most 5x ordinary on the fixed instrumentation corpus.
- Eligible multiversion dispatch versus portable baseline: geometric-mean
  improvement at least 8%, individual slowdown at most 3%. Dispatch achieves at
  least 98% of direct calls to the exact resolved hidden member in a separate
  byte-identical artifact and is at most 5% slower per case.
  On ELF, the collector reads the public entry from `.dynsym` and the exact
  published pointer from the private `.ck_dispatch_slot` section, so shipped
  products can omit the full local symbol table without changing this proof.
- Combined PGO+multiversion is no more than 2% slower in geometric mean and 5%
  individually than the faster matching PGO-only/multiversion-only channel.
- Combined CK reaches at least 95% of the faster equivalent Clang/Rust PGO
  geometric mean and at least 90% on every accepted kernel.
- PGO/multiversion/combined source-to-object geometric-mean ratios are at most
  1.5x/2.5x/3.5x ordinary and individual ratios at most 2x/3x/4x.
  Artifact aggregate ratios are at most 1.25x/2x/2x and individual ratios at
  most 1.5x/2.5x/2.5x. The distributed `ckc` archive is at most 15% larger than
  exact 0.12. Source-to-object samples use terminated-child user-plus-system CPU
  time, excluding hosted-worker descheduling without removing compiler work.
- All cumulative 0.12 gates remain: Native reaches at least 95% of pinned Clang
  geometric mean, no item is more than 10% slower, checked proof loops reach at
  least 97% of unchecked throughput, and optimizer
  latency retains the prior 2x suite/3x individual ceilings.
- On each architecture and safety mode, vector kernels reach at least 95% of
  the geometric mean of the faster valid C/Rust SIMD oracle for each kernel,
  and every kernel reaches at least 90% of its oracle. The domain-fact suite
  exceeds the faster generic Clang/Rust oracle geometric mean by at least 5%.
- The unchanged scalar corpus is no more than 3% slower in geometric mean and
  no individual case more than 8% slower than independently replayed 0.11.
  Native object size is at most 35% larger in aggregate and no individual object
  exceeds 2.5x its replay counterpart;
  baseline O3 source-to-object compilation ratios are at most 1.5x in geometric
  mean and 2x individually against that same fixed replay.

Runtime throughput, generation overhead, source-to-object time, artifact size,
compiler archive size, memory, cold/warm execution, and cache behavior are
separate quantities. No threshold authorizes weaker diagnostics, evaluation
order, modular integer behavior, strict floating semantics, checked first-error
order, print/effect order, semantic MIR, public ABI, or contract domain.

## Bounded checked-kernel diagnostic

Linux/AArch64 CI enables `CKC_OBSERVE_CHECKED_RUNTIME=1` to retain optional
observations around the **original** checked `specialized_length` gate calls.
The unchanged timer, kernel invocation loop, corpus and sampler still determine
the report. Each measurement evidence directory gets
`checked-runtime-observations.jsonl`: library hashes, actual entry/input/output
addresses, input/result digests, process maps, and all 429 original raw calls
(9 warmups and 20 × 7 × 3 measured calls). Snapshots bracket each call with wall
and thread CPU clocks, user/system CPU accounting, faults and context switches.
Rows are preallocated and written only after sampling; observation failures do
not replace the original result. Unsupported metrics are null, not zero.

`python3 scripts/check-runtime-observations.py target/ckc-perf/results-baseline.json`
verifies retained library bytes and reconstructs every stored sample and median
from that report's sidecar. This checks evidence consistency, not release
acceptance. Outer snapshots also include result hashing and boundary work, are
not atomic, and can perturb the surrounding process state. An incomplete sidecar
is invalid evidence; missing historical observations cannot be recovered.

Only when the workflow's `performance_diagnostics` input is explicitly enabled,
after the original gates Linux/AArch64 CI separately compares the hash-verified
checked `specialized_length` CK/C/Rust instruction bodies at their original
addresses and three fixed copied layouts. Copies are read-execute, never
write-execute, and must preserve each original's normal and error-prefix behavior.
One shared input/output workspace and the fixed full sampling schedule are used
across all layouts. Raw rows, mapped addresses, CPU affinity, resource snapshots
and available user-only hardware counters are retained under
`target/performance-diagnostics/checked-aarch64-layout`.

This is a code-placement intervention, not a remeasurement or replacement of
release evidence. It does not recover historical mappings, change any gate, or
automatically establish a root cause. Counter intervals include clock-boundary
work; unavailable or multiplexed counters are not treated as zero. A different
instruction body is explicitly reported as outside this bounded comparison.

## Commands and evidence

Local schema/checker/correctness checks precede expensive stable-worker runs:

The general harness entry point is `cargo bench --bench ckc_perf`; it writes
`build/perf/latest.summary.json` and `build/perf/latest.summary.md`. Native and
PGO measurements add the feature and task selectors shown below.

```sh
cargo test --locked --test performance -- --nocapture
python3 -m unittest discover -s tests/performance -p '*_test.py'
python3 scripts/prepare-performance-replay.py --baseline 0.12 \
  --out target/performance-runtime-replay-v012
python3 scripts/prepare-performance-replay.py --baseline 0.11 \
  --out target/performance-runtime-replay-v011
python3 scripts/prepare-performance-replay.py --baseline 0.10 \
  --out target/performance-runtime-replay
cargo bench --features native-toolchain --bench ckc_perf -- \
  --case proof --task check --cpu baseline
cp target/ckc-perf/results.json target/ckc-perf/results-baseline.json
python3 scripts/check-native-performance.py target/ckc-perf/results-baseline.json
cargo bench --features native-toolchain --bench pgo_perf -- \
  --task collect --out target/ckc-perf/v0.13-results.json
python3 scripts/check-native-performance.py target/ckc-perf/v0.13-results.json
```

The Native commands require the pinned `CKC_LLVM_PREFIX`, `CKC_CLANG_ORACLE`,
`CKC_CANDIDATE_COMPILER`, `CKC_V012_RUNTIME_BUNDLE`,
`CKC_V011_RUNTIME_BUNDLE`, and `CKC_V010_RUNTIME_BUNDLE` paths. The same worker
must produce and check both reports; a copied or cross-worker schema-7 report is
not release evidence.

The report is canonicalized and hashed before the independent checker reads it.
The benchmark cannot declare itself passing. Diagnostics inspect only the actual
report/artifacts and do not rebuild or remeasure a required gate. Changing a
source, corpus, profile, target/capability, oracle precondition, threshold,
statistic, exclusion, or checker is a reviewed contract change.

## WebAssembly runtime observations

The standalone Node/V8 runner measures WebAssembly artifacts separately from
the Native release gates above. Build `ckc` once, then run the same CK examples
at O0 and O3 with fixed inputs. The runner accepts `--wasm-features
baseline|simd128`, defaulting to `baseline`; use separate output directories
when recording both profiles:

```sh
cargo build --release --locked --bin ckc
node benches/wasm/bench.mjs --ckc target/release/ckc --wasm-features baseline \
  --out build/wasm-perf/baseline --samples 20 --warmup 10 --batch 100 --size 1024
node benches/wasm/bench.mjs --ckc target/release/ckc --wasm-features simd128 \
  --out build/wasm-perf/simd128 --samples 20 --warmup 10 --batch 100 --size 1024
CKC=target/release/ckc node --test benches/wasm/bench.test.mjs examples/wasm/host/*.test.mjs
```

The runner writes `wasm-runtime-report.json` and emitted modules under the
selected `--out` directory (default: `build/wasm-perf`), which should remain
ignored build output. It checks outputs before timing and records
the exact source, compiler, runner, and artifact identities along with raw
samples and host/runtime details. It also records the requested feature profile,
canonical profile digest, and `ck.wasm.target` metadata for every artifact, and
checks that the requested profile, digest, and emitted metadata agree before
timing. The O3 `f64_map`, `i32_map`, `u32_compare_select`, `i32_to_f64`,
`u32_to_f64`, `u32_alias_map`, `u32_reduce_sum`, and `u32_reduce_product` cases
exercise the independently verified SIMD128 paths when selected; the baseline
profile and O0 artifacts provide scalar comparisons. Use the separate overlap
and address-boundary tests when assessing the alias fallback. Keep correctness
results and instruction shape alongside timing.
`u32_cursor_copy` exercises the checked O3 address cursor, while
`u32_field_offset` exercises proof-backed field displacement in a memarg.
`u32_fill` exercises the guarded Bulk Memory fill path, and `pricing_batch`
compares one Wasm call for many records with one call per record using the same
persistent arena.
Compare these paths against O0 under the same v0.15 profile and include the
emitted instruction shape. A P8 compiler is a historical comparison with the
older schema-1 baseline; its profile digest and allowed features differ.

CK emission, module compilation,
instantiation, warm-up, steady kernel calls, and host preparation/readback are
separate observations. `--emission-samples` repeats compiler emission and keeps
each raw duration; the compatibility `ck_emission` field is their median. The
artifact record includes total bytes, section payload bytes, code bytes,
function count, and local count so direct binary emission can be assessed
alongside code growth. Compare the same source/profile/optimization level on
the old and new compiler in alternating runs, labeling their profile-schema
boundary explicitly. Bulk copy/fill results also need
short and long ranges plus overlap and trapping fallbacks; the batch case
reports logical rows and actual JS-to-Wasm crossings separately from the
runner's `--batch` repetition count.

Module compilation is the first compile of each artifact
in that Node process; it is not a browser cold-start measurement. The per-round
end-to-end sample covers preparation,
calls, and readback on an already instantiated module; it excludes module
compilation, instantiation, and memory growth. Reuse identical options when
comparing compiler revisions, and keep the full reports; a single local run
does not establish a portable speedup or a release threshold. The current
runtime channel is Node/V8, so its results must be labeled accordingly.

One local array-map comparison on Apple M5 Max with Node 24.14.0 used
preallocated `Int32Array`/`Float64Array` inputs and outputs, O3 `simd128` Wasm,
and the same JavaScript map loops. With JS elapsed time set to 1.0, the measured
Wasm speedups at 4,096, 16,384, and 65,536 elements were 3.09x, 5.95x, and
6.01x for `i32`, and 2.22x, 2.79x, and 2.79x for `f64`. These are local hot-kernel
observations; they exclude compilation, instantiation, and data preparation,
and do not predict other algorithms, machines, or runtimes.

PGO and bounded multiversioning shipped in 0.13 and remain subject to these
gates in 0.15. Offline Auto-Tuning is deferred. The local WebAssembly map
observations above are separate from the native release gates. Indirect-call
promotion, scalable KIR, and adaptive JIT PGO remain future work.
