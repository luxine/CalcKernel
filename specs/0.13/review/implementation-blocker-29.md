# Implementation blocker 29: standalone four-chain streaming map

Date: 2026-09-07

## Finding

Exact V0.13 run `34077713073`, x86-64 performance job `101607114352`,
failed the unchanged cumulative schema-7 gate:

`vectorSuites/unchecked/map_u32 is below 90% of its faster SIMD oracle`.

The candidate median was 996,367 ns and the faster Rust SIMD median was
834,064 ns, so candidate throughput was about 83.71 percent of the required
oracle. All twenty retained samples were stable. Disassembly showed that CK
processed eight `u32` elements with two independent XMM chains per branch,
while the Rust SIMD oracle processed sixteen elements with four chains.

No threshold, workload, sample count, statistic, platform, or required job may
change.

## Rediagnosis

The target profile exposed the existing closed `UF <= 4` frontier and the
`VF4/UF4` candidate passed legality and profitability. It was rejected solely
by the unchanged aggregate KIR growth ceiling: the standalone 34-unit map
would become 70 units, two above its exact `2x` limit. The same vector shape
fit when an unrelated function padded the module, exposing two redundant
representation costs rather than useful work.

An interleaved non-reduction body has one predecessor, so duplicating its
scalar block parameters and header edge arguments is unnecessary. When the
independently derived minimum trip equals `VF * UF`, separate constants for
the trip threshold and vector chunk width also encode the same value twice.

## Resolution

Interleaved non-reduction vector bodies now consume the dominating vector
header values directly and retain no redundant body parameters. The
materializer also reuses the exact minimum-trip constant when it equals the
chunk width. The independent checker requires the compact body shape,
reconstructs every UF stride from the dominating header induction, and accepts
the three-instruction preheader only when the remaining structural checks and
proofs close.

A RED/GREEN regression uses the standalone generic map with no padding
function. Before the repair it selected `VF4/UF2` after rejecting `VF4/UF4`
for growth; after the repair it selects `VF4/UF4` under the original `2x`
ceiling. A real x86 Native regression requires the pinned baseline target
profile to select the same four-chain shape. The object-affecting cache
identity adds `compact-vector-body-state-v2`.

No CK language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, profitability floor, code-growth budget, performance or
stability threshold, timed work, sample count, corpus, platform, or required
CI job changed.

## Verdict

Accepted implementation blocker. Focused and complete local verification must
pass before publishing a new exact candidate SHA. The real x86-64 performance
job remains authoritative for machine-code shape and throughput, and every
other required job remains mandatory.
