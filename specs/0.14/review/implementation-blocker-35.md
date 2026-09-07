# Implementation blocker 35: accepted modular-reduction corpus contract

## Verdict

Confirmed implementation blocker. Exact v0.14 run `34087252444`, AArch64
performance job `101633869632`, passed replay preparation and then rejected the
unchanged schema-8 gate because `vectorSuites/checked/modular_reduction` reached
only `9,037,976 / 10,527,912 = 85.85%` of the faster SIMD oracle, below the
unchanged 90% floor. All twenty candidate samples were tightly grouped between
10,519,720 ns and 10,534,152 ns, so this was not scheduler noise.

The downloaded machine code and KIR evidence identified one exact integration
omission. Accepted v0.13 revision
`5c6220758718b1ceac8ae32aec80c660d7b67b5e` defines the corpus kernel as an
unsafe contracted function with `requires n <= a.len` and `effects read(a)`.
V0.14 still carried the older uncontracted source. It therefore retained a
per-iteration out-of-bounds exit and emitted two loop counters. On the same
worker, the independently rebuilt accepted v0.13 source used its verified
contract, emitted one loop counter, and completed in 8,253,184 ns.

## Correction

V0.14 now preserves the accepted v0.13 source bytes exactly and repins the
existing corpus manifest entry to SHA-256
`00e9cf6faf936e510929c1d4352bbaa41d3a24cd837194dbd651e5059f141025`.
A repository regression freezes those exact bytes, and the Native corpus test
requires the trusted contract to eliminate the per-iteration out-of-bounds
failure while retaining checked overflow behavior. Local AArch64 object
inspection confirms the resulting loop has the same one-counter checked-add
shape as the independently rebuilt v0.13 artifact.

## Frozen boundaries

This restores rather than changes the accepted corpus. No language rule,
public ABI, strict floating-point rule, safety mode, oracle, workload, sample
count, timing method, platform, required job, performance threshold, or
artifact threshold changed. Callers of this unsafe corpus fixture remain
responsible for the already-frozen `n <= a.len` precondition.
