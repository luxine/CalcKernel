# Implementation blocker 66: retain checked tuning replay work

Exact V0.14 `f5e16c49519b4c101d08f2d5b92492a591c090e0`, run `34295522872`,
failed both performance jobs. The x86 job failed historical V0.13 replay on an
AVX-512 Xeon at `trip-unroll-simd` dispatch/ordinary = 1.0471936877. The exact
V0.13 repair `4add225778b867e33227236d178de33139a97d36` is inherited; its
independent acceptance is still required. The replay manifest is repinned to
that SHA with SHA-256
`06ddc50ef42d3497599fa46a6207b1335d2b52306c4f73d4b566d93a474b1012`.

The AArch64 job passed historical replay and fresh cumulative schemas 7/8, then
failed the unchanged schema-9 tune-use compile geometric limit. Per-case
tune-use/ordinary ratios were 4.11514 (branch-layout), 6.44464
(call-constant-length), 1.84569 (compute-bound), 1.80483 (contract-fixed-length),
1.80864 (contract-noalias), 1.86462 (memory-bound), and 1.82371
(trip-unroll-simd). All raw reports, command records and artifacts were retained.

## Root cause and implementation

Explicit replay unnecessarily emitted and linked a complete ordinary artifact,
then discarded it. Every deterministic search expansion re-enumerated the same
immutable tuning space. Plan application also discarded the independently
replayed state and materialized it again; trial construction repeated those
checks yet again.

Replay now retains verified frontend/target/header inputs without an ordinary
artifact. The unbuilt product's unit build type cannot enter ordinary artifact
publication. Opaque `CheckedTuningSpace` authority independently reconstructs a
raw space once, or retains the space directly after complete source-backed
enumeration. The bounded search, expansion trace, cost metrics, complete
frontier and selected-plan lookup are unchanged. `CheckedTuningPlan` retains
the exact independently checked post-state and an immutable plan borrow.
Checked trial construction derives every artifact identity from that authority.
Raw entry points still reject forged space fields, plan digests, choices and
pre/post states. Object-graph/link-recipe and output-role checks remain mandatory.

## Verification boundary

Test-first counters reproduced six identical space enumerations in one small
search and two materializations in one plan application. The regressions require
one of each and passed after the fix. Checked/raw search outputs and independent
plan-state digests match across all three budgets; mutated raw spaces and plan
digests still fail. Real cold/warm CLI publication, byte-identical replay and
stale-source rejection passed. A separate local real-workload tuning diagnostic
failed sampling stability and is retained as invalid, not counted as acceptance.

A separate executable CLI fixture used the same unmodified decision before and
after the optimization. Both replays passed all identity checks and emitted
byte-identical artifacts. With three warmups and fifteen rotating paired
terminated-child CPU samples, tune-use/ordinary medians changed from
15,532,000 / 11,991,000 ns (1.29530) to 14,012,000 / 13,408,000 ns (1.04505).
This is a local macOS compilation diagnostic for one existing CLI test fixture,
not the seven-case Linux release benchmark or runtime-performance acceptance.

No performance or stability threshold, timed work, sample count, corpus,
platform, required job, decision schema, public Native ABI, profile semantics,
frontier rule, or safety check was weakened. Only new exact-SHA remote CI can
establish the final compilation/performance acceptance.
