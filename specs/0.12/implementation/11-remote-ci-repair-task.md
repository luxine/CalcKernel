# V0.12–V0.14 Remote CI Repair Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking. The user requires inline execution;
> do not dispatch subagents.

**Goal:** Replace the unsound V0.12 performance measurement/admission logic,
propagate its exact identity through V0.13 and V0.14, repair the newly exposed
Windows build failures, and start one replacement exact-SHA CI per version.

**Architecture:** V0.12 interleaves the seven raw CK/C/Rust calls within each
retained row and evaluates the four-item SLP stability stream after removing
only the per-row common multiplicative factor. Its x86 vector admission floor
counts four actual vector operations instead of four UF-sized groups. V0.13
imports that exact repair and uses stable Win32 handle identity APIs; V0.14
imports the repaired replay identity and calls `FreeLibrary` from its actual
`windows-sys` module.

**Tech Stack:** Rust 1.90, Cargo, Python 3 schema checker tests, LLVM 22.1.8
native backend, GitHub Actions, `windows-sys` 0.61.

---

### Task 1: Interleave oracle raw calls

**Files:**
- Modify: `tests/performance/runtime_replay.rs`
- Modify: `benches/runtime_replay.rs`
- Modify: `benches/vector_perf.rs`

- [ ] **Step 1: Write the failing sampler test**

Add a test that calls the wished-for API and asserts the exact 21-call retained
row schedule and per-channel upper medians:

```rust
#[test]
fn oracle_upper_median_rows_should_interleave_every_raw_channel_call() {
    let mut calls = Vec::new();
    let samples = replay_api::sample_three_channels_upper_median::<(), 7>(
        0,
        1,
        |channel, warmup| {
            assert!(!warmup);
            calls.push(channel);
            Ok((calls.len() * 10 + channel) as u128)
        },
    )
    .unwrap();
    assert_eq!(calls, vec![0, 1, 2, 1, 2, 0, 2, 0, 1, 0, 1, 2, 1, 2, 0, 2, 0, 1, 0, 1, 2]);
    assert_eq!(samples.channels.iter().map(|row| row.len()).collect::<Vec<_>>(), vec![1, 1, 1]);
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test --locked --test performance oracle_upper_median_rows_should_interleave_every_raw_channel_call -- --nocapture
```

Expected: compilation fails because
`sample_three_channels_upper_median` does not exist.

- [ ] **Step 3: Implement the minimal sampler**

Add this public helper in `benches/runtime_replay.rs`; reuse
`rotating_round`, preserve fail-fast behavior, and retain exactly one upper
median per channel per row:

```rust
pub fn sample_three_channels_upper_median<E, const REPETITIONS: usize>(
    warmup: usize,
    iterations: usize,
    mut call: impl FnMut(usize, bool) -> Result<u128, E>,
) -> Result<RuntimeSamples<3>, E> {
    assert!(REPETITIONS > 0, "an upper median requires at least one sample");
    let mut result = RuntimeSamples {
        warmup_order: Vec::with_capacity(warmup),
        sample_order: Vec::with_capacity(iterations),
        channels: std::array::from_fn(|_| Vec::with_capacity(iterations)),
    };
    for round in 0..warmup {
        let order = rotating_round(round);
        for channel in order {
            call(channel, true)?;
        }
        result.warmup_order.push(order);
    }
    for round in 0..iterations {
        let mut raw: [Vec<u128>; 3] =
            std::array::from_fn(|_| Vec::with_capacity(REPETITIONS));
        for repetition in 0..REPETITIONS {
            for channel in rotating_round(round.wrapping_add(repetition)) {
                raw[channel].push(call(channel, false)?);
            }
        }
        for channel in 0..3 {
            raw[channel].sort_unstable();
            result.channels[channel].push(raw[channel][REPETITIONS / 2]);
        }
        result.sample_order.push(rotating_round(round));
    }
    Ok(result)
}
```

Change `measure_case` to use the new helper. Warmup calls invoke
`measure_once`; retained calls also invoke `measure_once`, because the helper
now owns the seven-call grouping. Remove `condition_short_kernel`, its runner
state, its probe helper, and all upper-band constants.

- [ ] **Step 4: Verify GREEN**

Run the focused sampler tests and expect PASS:

```bash
cargo test --locked --test performance runtime_replay -- --nocapture
```

### Task 2: Check common-mode SLP stability without weakening thresholds

**Files:**
- Modify: `tests/performance/runtime_gate_test.py`
- Modify: `scripts/check-native-performance.py`

- [ ] **Step 1: Write failing checker tests**

Add one mutation where all three `slp_quad` streams share the same alternating
`100/200` factor and assert the full report still passes. Add a second mutation
where only `rustSimdSamplesNs` alternates and assert rejection with
`common-mode` in the message. Keep all other streams at their valid fixture
values.

- [ ] **Step 2: Verify RED**

Run:

```bash
python3 -B -m unittest tests.performance.runtime_gate_test
```

Expected: the common three-channel frequency shift is rejected by the old
absolute stability check.

- [ ] **Step 3: Implement common-mode validation**

Split structural stream validation from stability validation. For
`slp_quad`, validate raw lengths, positivity, and stored upper medians, then
compute each row scale as the geometric mean of the three channel durations.
For each channel require at least 16 of 20 normalized ratios to remain within
75%..125% of that channel's normalized median. Continue using raw medians for
all throughput comparisons. All other cases continue through the existing
absolute `stable_samples` path.

- [ ] **Step 4: Verify GREEN**

Run the same Python suite and expect PASS.

### Task 3: Bind the new performance protocol

**Files:**
- Modify: `benches/oracles/manifest.toml`
- Modify: `benches/vector_perf.rs`
- Modify: `scripts/check-native-performance.py`
- Modify: `tests/performance/vector_oracles.rs`
- Modify: `specs/0.12/fact-driven-vector-optimizer.md`
- Modify: `specs/0.12/zh-CN/fact-driven-vector-optimizer.md`
- Modify: `specs/0.12/implementation/10-performance-ci-acceptance.md`
- Modify: `specs/0.12/implementation/99-final-acceptance.md`

- [ ] **Step 1: Write the failing source-contract assertion**

Change the oracle contract test to require
`interleaved-upper-median-three-channel-v2`, the new sampler call, and absence
of `bounded-upper-band-v1`, `SLP_CALIBRATION_PROBES`, and
`condition_short_kernel`.

- [ ] **Step 2: Verify RED**

Run the focused `vector_oracles` test and expect its old protocol assertion to
fail.

- [ ] **Step 3: Update manifest and documentation**

Pin the new protocol in the oracle manifest, delete obsolete settling fields,
recompute `ORACLE_MANIFEST_SHA256`, and update the checker constant and paired
English/Chinese normative text. Keep batch size, sample rows, calls, statistic,
and thresholds unchanged.

- [ ] **Step 4: Verify GREEN**

Run the focused Rust and Python performance-contract suites and expect PASS.

### Task 4: Correct x86 short-loop vector admission

**Files:**
- Modify: `tests/optimizer/vectorize.rs`
- Modify: `src/optimizer/analysis/vectorize.rs`
- Modify: `src/optimizer/vectorize_check.rs`

- [ ] **Step 1: Write the failing discovery test**

Require every x86 candidate to satisfy at least four vector operations using
`ceil(4 / UF) * VF * UF`, and require the exact 16-element noalias fixture to
contain a `VF4/UF4` candidate with `minimum_trip == 16`. Preserve the existing
three-element exact-trip rejection test.

- [ ] **Step 2: Verify RED**

Run:

```bash
cargo test --locked --test optimizer loop_simd_ -- --nocapture
```

Expected: the exact 16-element x86 assertion fails because the old minimum is
64.

- [ ] **Step 3: Implement proposer and independent checker rules**

In both independent pricing implementations, use
`4_u32.div_ceil(u32::from(uf))` as the x86 starting group count and keep `2`
for all other targets. Do not share the calculation between proposer and
checker.

- [ ] **Step 4: Verify GREEN**

Run focused optimizer and native vector tests. Expect PASS and independent
checker agreement.

### Task 5: Complete and publish V0.12

**Files:** all files changed by Tasks 1–4.

- [ ] **Step 1: Run local gates**

Run `cargo fmt --check`, default/all-feature Clippy with `-D warnings`, default
and all-feature tests, Python performance checker tests, and the available
native performance diagnostic. Generated x86 artifacts must contain SIMD for
both domain fixtures. Any unchanged threshold failure becomes a new blocker;
do not edit the threshold.

- [ ] **Step 2: Commit and push**

Commit the implementation separately from design commit `beb2c41`, push
`feature/v0.12-vector-optimizer`, and dispatch `ci.yml` for its exact remote
HEAD. Record the new SHA and run ID.

### Task 6: Repair and publish V0.13

**Files:**
- Import: exact V0.12 repair files and replay identity
- Modify: `tests/profile/generation.rs`
- Modify: `src/profile/generation.rs`
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: V0.13 replay manifests/checkers/spec acceptance identities

- [ ] **Step 1: Verify the Windows RED build**

Run a Rust 1.90 Windows target `cargo test --all-features --locked --no-run` and
confirm `E0658 windows_by_handle` at the two metadata extension calls.

- [ ] **Step 2: Implement stable handle identity**

Pass `path` into `directory_identity`. Under Windows, open the directory with
`OpenOptionsExt::custom_flags(FILE_FLAG_BACKUP_SEMANTICS)`, call
`GetFileInformationByHandle`, and combine `nFileIndexHigh/Low`. Add the
target-specific `windows-sys` Foundation/FileSystem dependency. The existing
same-directory identity test is the behavioral regression test.

- [ ] **Step 3: Verify and publish**

Run focused profile tests, Windows cross-target no-run build, full local gates,
and cumulative performance contracts. Commit, push
`design/v0.13-pgo-multiversion`, and dispatch one exact-SHA CI.

### Task 7: Repair and publish V0.14

**Files:**
- Import: repaired V0.13 replay identity and inherited performance files
- Modify: `benches/tune/runner.rs`
- Modify: V0.14 replay manifests/checkers/spec acceptance identities

- [ ] **Step 1: Verify the Windows RED build**

Run the Rust 1.90 Windows target no-run build and confirm `E0425` for
`System::LibraryLoader::FreeLibrary`.

- [ ] **Step 2: Correct the Windows API module**

Call `windows_sys::Win32::Foundation::FreeLibrary(self.0.cast())`. Existing
features already include `Win32_Foundation`; do not add a redundant feature.

- [ ] **Step 3: Verify and publish**

Run the Windows cross-target no-run build, focused tuning-runner tests, all
V0.14 local acceptance commands, and cumulative replay contracts. Commit,
push `design/v0.14-offline-autotuning`, and dispatch one exact-SHA CI without
merging main.

### Task 8: Hand remote monitoring back to the heartbeat

- [ ] **Step 1: Verify exact remote identities**

For each branch, prove remote HEAD equals the pushed local HEAD and latest CI
`headSha` equals that exact value.

- [ ] **Step 2: Leave long CI asynchronous**

The heartbeat `calckernel-v0-12-v0-14-ci` rediscovers branch heads every 30
minutes. Do not block locally on long jobs; it must repair any later failure
and notify only on meaningful state changes.

### Task 9: Repair x86 cross-UF scheduling exposed by exact CI

**Files:**
- Add: `specs/0.12/review/implementation-blocker-19.md`
- Modify: `src/optimizer/kir_passes/vectorize.rs`
- Modify: `tests/optimizer/vectorize.rs`

- [ ] **Step 1: Prove RED on the exact selected shape**

Construct the x86 `VF4/UF2` noalias map trial and require both vector loads to
precede either vector store. The old per-chunk materializer order must fail.

- [ ] **Step 2: Schedule only dependency-ready instructions**

For x86-64 `UF > 1`, use local SSA and MemorySSA readiness with stable
load/setup/vector/store priority. Preserve same-partition memory dependencies
and renumber effects monotonically. Do not change candidate choice or cost.

- [ ] **Step 3: Verify and publish the replacement chain**

Run focused optimizer and Native LLVM tests, then every required local gate.
Push V0.12, import and repin it into V0.13, import and repin V0.13 into V0.14,
and dispatch exact-SHA CI for all three branches. The unchanged x86 domain
performance gate is authoritative for the performance repair.
