# Implementation blocker 08: compile-time wall clock measured worker descheduling

## Failure

The exact x86-64 remote performance gate for candidate
`b00e7261608ef9781ad23054209d1c4fec8e69c7` passed the Native integration job and
restored `integer_accumulate` to within approximately 0.47% of the exact V0.11
replay. The performance checker then rejected the source-to-object corpus because
multiple candidate and replay sample streams were unstable around their medians.

Both compilers showed isolated wall-clock samples between roughly 60 ms and 220 ms
among ordinary 18 ms to 35 ms samples. The noise affected both sides, varied by
case and mode, and did not correspond to compiler output or runtime regressions.

## Root cause

`compile_ck` measured `Command::output()` with the parent process's monotonic wall
clock. A hosted worker can deschedule the benchmark or compiler process during that
interval. The elapsed value therefore mixed compiler cost with unrelated host
scheduling stalls, even though the benchmark runs candidate and replay serially and
alternates their order.

The frozen stability rule correctly rejected that contaminated evidence. Relaxing
the rule, reducing the corpus, or changing the 1.5x/2x thresholds would conceal the
measurement defect and is not permitted.

## Correction

On the Linux/Unix performance workers, every source-to-object sample now uses the
difference in cumulative `RUSAGE_CHILDREN` user-plus-system CPU time immediately
before and after the compiler subprocess. This includes actual compiler CPU work
and the CPU usage of waited descendants while excluding time when the hosted worker
does not schedule them. Non-Unix development builds retain a wall-clock fallback;
the authoritative x86-64 and AArch64 performance workers are Linux.

The report still uses fresh outputs, disabled caches, three warm-up pairs, fifteen
measured pairs, alternating candidate/replay order, and the upper median. Sample
stability, individual 2x, geometric-mean 1.5x, runtime, size, semantic, and safety
gates are unchanged. A structural contract prevents the authoritative path from
returning to parent wall-clock timing.

## Acceptance boundary

Local performance contracts, the schema checker regression suite, formatting,
Clippy, and the Native benchmark build must pass. Exact x86-64 and AArch64 CI remain
the authoritative dynamic confirmation that both candidate and replay compile-time
streams are stable under the unchanged checker.
