# Same-Process Frozen-Compiler Runtime Replay Implementation Plan

> Execute with `superpowers:executing-plans`, entirely inline. No subagents,
> new task, main merge, tag or Release. This supplements stage 11 only.

**Goal:** Compare 0.11 with the exact accepted 0.10 compiler on the same hardware
and in the same sampling window, while preserving every numeric performance gate.

**Architecture:** Prepare hashed libraries using an independently built, pinned
0.10 compiler; load them alongside candidate Native and pinned C-oracle libraries
in the existing Rust benchmark process. Interleave both safety modes and both
compiler versions. Keep historical measurements unchanged as provenance, not as
a substitute for the same-process replay samples.

**Tech stack:** Existing Rust benchmark/dynamic loader and SHA-256, Python 3
standard library, Git, Rust 1.90.0, existing pinned LLVM/Clang 22.1.8 prefixes.
No compiler/runtime dependency, package dependency or release ABI is added.

## Reviewed counterexample and scope

I14's preserved x86 report and independent same-worker V0.10 replay show that
the unchanged V0.10 itself fails the historical normalized ratio. The complete
integer-kernel ELF files are byte-identical between candidate and V0.10 on that
worker. Different CPUs need not change the Native/C timing ratio by a common
factor; multiplying by a frozen Clang ratio cannot generally remove that effect.

I19's checked/unchecked measurements are separated by entire suites. Both Native
and Clang, including the unchanged V0.10 diagnostic, show the cross-mode decrease.
This supports eliminating that sampling separation, without claiming a particular
temperature or scheduler cause has been proven.

The selected repair runs both fixed compilers' artifacts in one process and one
interleaved sampling window. A dedicated immutable physical runner would also
address hardware identity, but is not available within this repository workflow.
Historical baseline numbers remain untouched. The original failures remain failed.

Unchanged requirements:

- Exact four runtime kernels and six optimizer cases, source digests and domains.
- Fixed V0.10 commit `df816502876fba41676f9ebc190e4fadd18cd5a5`, compiler identity,
  existing four checksum-pinned adapters, strict FP, portable target CPU policy.
- Three warm-up rounds, twenty samples, minimum of seven calls per sample,
  twenty-million-iteration/input batch, upper median and stability checks.
- Native/Clang throughput >=95% geometric, <=10% individual time regression;
  Clang-normalized candidate/V0.10 time <=1.03 geometric and <=1.08 individual;
  **raw** checked/unchecked proof throughput >=97%; optimizer <=2x/3x.
- No historical runtime/optimizer baseline refresh and no optimizer timer change.

## Task 0: Freeze the reviewed protocol before implementation

- [x] Synchronize both language versions of the specification and performance guide,
  master/stage-11/final acceptance and report schema with this plan, then commit them
  separately before changing measurement or checker code. Keep every numeric gate.

## Task 1: Baseline preparation and integrity boundary

**Files:** new `scripts/prepare-performance-replay.py`,
`benches/runtime_replay.rs`, tests under `tests/performance/`.

- [ ] Add tests before implementation for absent/incomplete bundle, wrong commit,
  compiler version/LLVM/target/CPU, changed source/adapter bytes, duplicate or missing
  case/mode, malformed hash, escaping path, and modified library/compiler bytes.
  Every case must fail before loading a library or taking a timing sample.
- [ ] Preparation creates a fresh owned directory under ignored `target/` (or an
  explicit new output directory); it must refuse an existing nonempty target.
  Make an independent local clone of this repository and detach the exact pinned
  commit. Do not modify the existing 0.10 worktree, candidate checkout or main.
- [ ] Verify the clone is clean, validate the four adapter hashes already frozen
  in `v0_10_compiler.toml`, and apply exactly those patches. Record the resulting
  tracked diff digest and check it again after building/emitting; no compiler source
  changes, additional patches or generated current-compiler C input are allowed.
- [ ] Build with `cargo +1.90.0 build --release --locked --features native-toolchain
  --bin ckc`, using the selected pinned LLVM prefix. Validate actual verbose compiler
  version and LLVM identity. Copy the resulting compiler into the owned bundle and
  hash the actual executable, rather than accepting a user-written version label.
- [ ] Use that compiler to emit the four fixed runtime fixtures in both checked
  and unchecked modes, O3 and baseline CPU. Verify each fixture digest against the
  unchanged baseline manifest; record every actual library's SHA-256 and byte count.
  Preparation does not run benchmarks or choose a baseline from timing results.
- [ ] Write a strict version-1 TSV bundle manifest (not a general TOML parser):
  first line `ckc-v010-runtime-replay\t1`; unique required scalar records for commit,
  compiler identity/hash, LLVM version, target, CPU policy, preparation recipe hash,
  adapter-set hash and source-diff hash; exactly eight `artifact` records containing
  mode, case, fixed basename, size and hash. Unknown/duplicate fields are errors.
  The recipe hash binds the preparation script and replay implementation. The
  adapter set must be the already-pinned four adapters, not a caller-supplied list.
- [ ] The Rust loader independently validates that strict manifest, expected identity,
  fixed safe basenames, exact file sizes/hashes and current preparation recipe before
  `DynamicLibrary::open`. Keep compiler and libraries for CI audit. Source provenance
  comes from the trusted pinned-checkout preparation log; hashes are not a signature
  that could authenticate an arbitrary untrusted compiler bundle.

No external download of a prebuilt compiler or mutable branch reference is allowed.
Existing dev-only Git/Python/pinned toolchain requirements are sufficient.

## Task 2: Balanced same-process sampling

**Files:** `benches/ckc_perf.rs`, `benches/runtime_replay.rs`,
`tests/performance/bench.rs` and a focused sampling test module.

- [ ] Add a pure scheduler test before implementation. There are eight channels:
  candidate Native unchecked/checked, current Clang unchecked/checked, V0.10 Native
  unchecked/checked, and replay Clang unchecked/checked. For round `r`, visit channel
  `(r + offset) % 8` for offsets 0 through 7. Assert each appears once per round and
  each occupies each position either two or three times across twenty rounds.
  The schedule is deterministic and never depends on measured durations.
- [ ] Split existing case preparation from measurement. Prepare both modes and all
  libraries before warm-up; the two Clang copies independently compile the same
  digest-pinned V0.10 C source with the unchanged strict Clang command/CPU flags.
  Candidate code must not produce either C calibration source or V0.10 Native code.
- [ ] Build the existing deterministic inputs once per kernel and share those same
  inputs across all eight channels. Preserve the checked status/out-slot and slice
  ABIs, seed 17, batch length and proof values. Verify all results against the frozen
  C oracle before measurement and on every timed call; nonzero status is failure.
- [ ] Warm all channels three times using the fixed schedule, then execute twenty
  scheduled rounds. Keep the original seven-call minimum for each channel/sample.
  Record all eight sample arrays, schedule identity and exact round order. This
  removes whole-suite mode separation; it does not prove the host is exclusive.
- [ ] Keep the existing six-case optimizer timing code, frozen MIR medians, counts
  and preparation/timer boundary unchanged. Preserve existing compile/cold/memory/
  artifact-size quantities as separate measurements, not runtime denominators.
- [ ] Preserve the actual measured libraries and their hashes in an ignored evidence
  directory referenced safely by the report. This replaces the diagnostic's empty
  `.text` extraction as primary code evidence: whole files are always nonempty.
  Disassembly may still be diagnostic; never infer equality from an empty section.

## Task 3: Schema 6 and independent acceptance

**Files:** `scripts/check-native-performance.py`, `benches/summary-schema.md`,
`tests/performance/bench.rs`, related test fixtures.

- [ ] Keep `baselineV010` and existing `v010MedianNs`/`v010ClangMedianNs` as immutable
  historical provenance, still checked against the unchanged schema-2 manifest.
  Add actual replay Native/Clang sample arrays and upper medians per runtime case.
  Add bundle/compiler/recipe identity and exact artifact digests, plus the fixed
  sampling protocol and complete schedule. Increment only native report schema to 6;
  general compiler summary remains schema 1 and frozen baseline stays schema 2.
- [ ] The checker verifies replay bundle identity/files against report metadata,
  recomputes all four stream medians per mode, and rejects unstable/missing/duplicate
  arrays. Candidate/Clang still uses candidate streams. The V0.10 gate now uses
  `(candidateNative / currentClang) / (replayV010Native / replayClang)`.
  Raw proof throughput still uses candidate unchecked/checked medians only.
- [ ] Add the actual I14 numerical counterexample as a synthetic regression:
  candidate and replay V0.10 identical on the new worker must not be called a
  regression because of the historical Native/Clang ratio. Independently inject
  >3% geometric and >8% individual actual replay regressions and require rejection.
- [ ] Retain every existing negative gate (95%, 10%, 97%, 2x, 3x, identity, corpus,
  stability, source and frozen-history tampering). Add forged replay medians/sample
  counts, changed hashes, wrong protocol/order, wrong baseline compiler, candidate
  substituted for baseline, non-finite timings, and missing replay/artifact evidence. No unchecked or
  uncalibrated fallback path may return a passing report.
- [ ] A changed artifact after preparation must be detected by both loader and
  checker. Missing dev tooling/preparation must produce an actionable hard error,
  not silent use of historical numbers. Small `--quick` runs remain investigative
  and cannot pass the strict twenty-sample checker.

## Task 4: Workflow and synchronized contracts

**Files:** both language versions of `specs/0.11/fact-driven-optimizer.md` and
`docs/guides/performance.md`, master/stage-11/final acceptance, `.github/workflows/ci.yml`,
`scripts/diagnose-native-performance.sh`, CI and documentation contract tests.

- [ ] Both existing performance jobs prepare the pinned compiler bundle before the
  original full benchmark/checker step; no job becomes optional. Upload the preparation
  provenance, exact bundle, actual measured libraries and first report on success/failure.
  Preparation failures remain job failures. Do not relabel old failed runs as passing.
- [ ] Preserve failure diagnostics, but avoid rebuilding another different baseline
  or exporting an empty `.text` as evidence. Use the already-measured, hashed complete
  artifacts; require nonempty executable sections if producing section-only diagnostics.
- [ ] Update source-order CI tests and Python/checker invocation tests, keeping the
  quality/native-integration/six-host/two-performance matrix and cache policy unchanged.

## Task 5: Validation and first-run evidence

- [ ] Run targeted negative tests and actual preparation; verify the actual baseline
  compiler is 0.10.0 and the candidate 0.11.0, with unchanged source/adapter identities.
- [ ] Run default/all-feature tests sequentially, release library/IR mutation tests,
  all-target/all-feature Clippy, fmt and diff checks. Real Windows SDK CI remains
  separate from local macro simulation. Do not run builds while timing performance.
- [ ] Commit implementation and run the first complete native performance gate on
  that SHA. Retain all raw measurements even on failure; do not reroll for green.
- [ ] Dispatch the unchanged ten-job matrix on the final candidate after prior cold
  Windows bootstrap has completed/saved its cache. All required jobs must be green
  for the same final SHA; earlier partial successes cannot be combined.
- [ ] Audit phases 01–11 and `99-final-acceptance.md`, commit final evidence, retain
  the feature worktree and wait for user review without merging main.

## Adversarial self-review

- Identity comes from an independent fixed compiler build, not candidate-emitted C
  or a second call to the candidate compiler. Hashes bind every actual input artifact.
- Both compilers and both modes execute on the same input in the same process; the
  deterministic schedule cannot select samples or alter the fixed statistic.
- Historical numbers and original failures remain auditable. No threshold, source,
  algorithm, safety mode, compiler identity, optimizer budget or timer is relaxed.
- Preparation never mutates user-owned baseline/main worktrees. Failure is explicit.
- The change repairs the demonstrated comparison protocol; it does not assert that
  every future hardware/scheduling effect is removed or that any new gate has passed.
- No blocker found in this supplemental design. Implementation and real evidence are
  still required before I14/I19 or stage 11 can be closed.
