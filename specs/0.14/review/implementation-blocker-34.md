# Implementation blocker 34: exact v0.13 map replay and immutable compiler path

## Verdict

Confirmed implementation blockers. Exact v0.13 run `34077713073`, x86-64
performance job `101607114352`, rejected the unchanged schema-7
`map_u32` SIMD-oracle gate. Exact v0.14 run `34077978310` reproduced that
historical failure on x86-64. Its AArch64 performance job `101607713997`
completed the historical schema-8 replay successfully, then failed preparation
with `baseline compiler changed during library emission`.

Neither result is measurement noise or a reason to change a gate. The v0.13
failure was the standalone-module KIR growth defect recorded in
`specs/0.13/review/implementation-blocker-29.md`. On v0.14, the compiler had
already been copied into the owned replay directory with its digest recorded.
The subsequent historical `cargo bench` commands legitimately rebuilt the
clone's `target/release/ckc`; the final check incorrectly treated that mutable
Cargo output as the immutable baseline even though the copied compiler was
unchanged.

## Correction

V0.14 inherits the exact v0.13 correction at
`5c6220758718b1ceac8ae32aec80c660d7b67b5e` and repins the replay manifest to
that commit. The manifest SHA-256 is
`c6f5242e68907ba511777ab104bb66d0c2126eafd53be8a21e3e4257ac00b86f`.
The compact body shape applies only to ordinary interleavable loops;
v0.14 predicated-update loops retain their independently checked body-parameter
identity, so Floyd tuning-space enumeration and replay remain unchanged.

Replay preparation now names the copied `ckc-v013` file as
`CKC_CANDIDATE_COMPILER` for all historical performance commands and verifies
that exact immutable file after library emission. Cargo remains free to rebuild
its owned build-tree output, which is no longer an execution or identity input.
A RED/GREEN regression mutates the build-tree compiler and proves that the
frozen copy remains accepted, then mutates the frozen copy and proves that the
same integrity failure is still enforced.

## Frozen boundaries

No language rule, public Native ABI, Runtime ABI, target feature set, safety or
strict floating-point rule changed. No workload, sample count, timing method,
corpus, platform, required job, code-growth limit, profitability floor, or
performance/artifact threshold was reduced. V0.13 remains independently
accepted by its own exact-SHA workflow before v0.14 replay acceptance.
