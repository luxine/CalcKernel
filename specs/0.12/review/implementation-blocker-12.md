# Implementation blocker 12: Linux runtime wall clock included shared-host descheduling

Date: 2026-09-04

## Finding

V0.14 exact CI run `33808562098`, AArch64 performance job `100825742078`,
rebuilt and measured exact V0.13 candidate
`1a7f89b841ce3033063ef4a8ac458aa695c8a8c0`. Its cumulative schema-7 checker
again rejected `vectorSuites/unchecked/slp_quad candidateSamplesNs` as unstable
around the median. The same worker reproduced the established approximately
4.42 ms / 8.84 ms two-band behavior after four conditioning batches and after
the measurement thread had been pinned to one inherited-allowed vCPU.

This dynamically falsifies the previous claim that cross-vCPU migration was
the remaining variable. Same-core affinity cannot prevent a shared hosted
runner from descheduling or throttling its virtual CPU, and a monotonic
wall-clock interval includes that unrelated time.

## Rediagnosis

The four-element SLP case executes the unchanged 20,000,000-work-item native
call loop. Its code, inputs, output digest, channel rotation, seven-repetition
upper median, and thresholds are unchanged. Repeated all-channel evidence and
the new same-core failure localize the uncontrolled value to hosted scheduling,
not CK optimizer semantics. Selective reruns, fewer samples, a wider stability
window, or reduced timed work would conceal the invalid evidence and remain
forbidden.

## Resolution

On authoritative Linux performance workers, `measure_once` now reads
`CLOCK_THREAD_CPUTIME_ID` immediately before and after the unchanged native
call loop. The clock retains all CPU time consumed by the current measurement
thread while excluding periods in which the shared host does not schedule it.
The existing fail-closed same-core affinity and four-batch short-kernel
conditioning remain. Non-Linux developer runs retain the monotonic wall-clock
fallback.

A structural RED/GREEN regression requires the Linux current-thread CPU clock
and forbids direct `Instant::now` use inside the authoritative sample region.
No timed work, sample count, channel order, upper-median statistic, stability or
performance threshold, corpus, compiler output, or platform gate changed. A
fresh exact-SHA CI run remains the dynamic authority.
