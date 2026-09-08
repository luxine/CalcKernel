# Implementation blocker 45: authorize full-width v4 compute loops

## Evidence

V0.14 exact-SHA workflow-dispatch run `34212249513`, commit
`b829284fdcfd635db86401a2bc908de069e9f027`, independently rebuilt accepted
V0.13 commit `6258089cf44ebc317247e3fc38e2c765e424132e` on an AMD EPYC 9V74 worker
advertising the complete `x86-64-v4` capability set. The retained schema-8
compute-bound medians were 34,832 ns for the combined CK path and 30,025 ns for
the Rust PGO oracle, so CK reached about 86.2% of oracle throughput and missed
the unchanged 90% minimum. All twenty samples were present and stable.

The independent V0.13 x86 performance job in run `34210836533` rebuilt the
same commit but landed on a v3-only worker, where combined CK measured 42,983
ns against Rust PGO at 42,663 ns and passed. The retained v4 object selected the
correct v4 member, but disassembly showed four 256-bit YMM arithmetic chains;
the Rust oracle used four 512-bit ZMM chains.

## Root cause and rejected shortcuts

Blocker 35 made the profitable v4 member retainable and dispatchable, but the
generic LLVM `x86-64-v4` cost model still chose AVX2 width for the compute-dense
strict-`f64` map. The independent V0.13 run therefore could not reveal this
v4-only loss when GitHub scheduled it on a v3 host.

Lowering the throughput threshold, reducing samples or timed work, accepting
the v3-only run as v4 coverage, adding fast math, or forcing AVX-512 globally
is rejected.

## Repair

A regression contract was added first and observed failing. During O3 native
lowering, only an `x86-64-v4` target with explicit `+avx512f` may receive a
vector-width-eight loop authorization. The independent analysis clone must
identify a scalar memory map with both non-local load and store, at least eight
strict scalar `f64` arithmetic operations, no fast-math flag, and no existing
loop schedule. All other targets and loops retain LLVM's ordinary cost-model
choice. The multiversion object-cache codegen identity is advanced so an older
YMM-shaped object cannot be reused.

This changes no language or public ABI, strict-FP or safety semantics, target
eligibility, profile schema, dispatcher rule, performance/stability/size
threshold, timed work, sample count, corpus, platform, or required job.

## Acceptance

The focused RED/GREEN contract, native bridge build, formatting, lint, complete
locked tests, release native build, oracle audits, and replacement exact-SHA
ten-job workflow must pass. The x86 performance job must exercise schema 8
unchanged; a v4-capable worker is authoritative for the repaired ZMM lowering
and compute-bound threshold.
