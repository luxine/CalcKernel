# Implementation blocker 69: runner completion polling quantizes cold tuning

Exact V0.14 `75ffddc4ff43e0e2b9d7e61fc8ae3df80abe801c` run `34319431643`
passed the independently rebuilt V0.13 replay and fresh cumulative schema 7/8
on both performance workers. The x86 job `102362855368` had the required
x86-64-v4 hardware (AMD EPYC 9V45), but schema-9 collection rejected different
independent cold choices for `call-constant-length`. Both complete performance
artifact closures and both failed-job logs were downloaded and inspected.

## Evidence and diagnosis

The two cold runs have the same complete frontier, all eight compiled candidate
object/link identities, and all eight primary content hashes. The frontier
digest is `0dc1ec4b957f865d6bdec5e4c78cb67290949167c37e2416c85bbc11e4ec4573`.
Cold one selects plan `5b978ee06e41752e137892c2eb98f4609fd95432679f867c4c68923b3b2abe79`;
cold two selects `ade5daa50ddf1e65fc96b956fa93f5d24581dc2a13b74d96ddb31e8626a9f6de`.
Warm reuse exactly reproduces cold one. Both cold calibrations use 8,192
iterations after 14 attempts. The candidate ELFs contain scalar recurrences,
not SIMD memory operations; this is not an AVX alignment or object-identity drift.

The two competing candidates' retained samples cluster near 16.48 or 18.54 ms,
while baseline samples cluster near 61.78 ms. The approximately 2.06 ms quantum matches the
parent's `try_wait` / `sleep(2 ms)` loop. In round one, cold one compares
16,486,374 versus 18,507,862 ns; cold two compares 18,520,023 versus 16,480,172 ns.
The fixed selection rules consume these quantized measurements correctly, but
the collection mechanism injects enough delay to reverse the ranking. The
parent also previously sampled elapsed time only after joining output readers.

## Bounded correction

The parent clock starts before process creation. A per-invocation observer
blocks on process completion and immediately freezes the external monotonic
timestamp. POSIX uses `waitid(P_PID, WEXITED | WNOWAIT)`; Windows waits on an
owned duplicate process handle. The observer does not reap or terminate the
child, install a process-wide signal handler, or trust a runner-reported time.
The parent retains the child and its containment, waits for the full deadline,
applies existing termination on timeout/error, joins the observer before reaping,
then validates bounded output with the already frozen successful timestamp.
The retained unreaped child prevents PID reuse during cooperative cleanup.

A source-boundary regression first failed on the fixed polling sleep, then
passed with completion-driven timing, a pre-spawn start, and no post-output
resampling. Real-process tests cover successful/nonzero completion, non-reaping
observation, concurrent independent children, and full-deadline termination and
reaping. Existing runner admission, protocol, correctness and typed-timeout
coverage remains in place. No schema, runner wire record, sample count, timed
work, frontier, ranking, stability or performance threshold changes.

Local verification passed both complete Cargo test configurations, both
all-target Clippy configurations, formatting, 58 Python performance contracts,
the native release build, and actual signed-compiler dependency and JIT audits.
Windows test code cross-checks successfully; its runtime verification remains
part of the required remote host jobs. These local results do not replace the
Linux cold-determinism or performance gates.

## Separate compile-performance blocker remains open

AArch64 job `102362855366` completed schema-9 evidence and failed the tune-use
compile geometric gate. All seven 15-sample compile cohorts were read. Their
geomean is 1.2521939087 against the frozen 1.10 limit. Branch-layout measures
31,100,000 / 20,002,000 ns (1.5548445155), and constant-length call measures
48,091,000 / 20,522,000 ns (2.3433875841), both above the 1.20 per-case limit.
The remaining five case ratios are approximately 1.05–1.07. This confirms the
broader KIR-state/serialization issue described in blocker 68; no fourth
speculative compile optimization or threshold relaxation is bundled here.

The timing correction still requires new exact-SHA CI evidence; it does not
prove deterministic cold selection or final acceptance on Linux. V0.13 remains
pinned to `d85e0c786aaeeaa4dbaab9bffa01fcbd5f7c9f5a` and retains its independent
ten-job acceptance. Guaranteed v4 runner availability and the pending compile
architecture decision remain separate concerns.
