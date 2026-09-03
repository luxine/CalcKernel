# Implementation blocker 11: same-core Linux runtime measurement

Date: 2026-09-04

## Finding

V0.12 exact run `33795636751` passed AArch64 performance, but the cumulative
V0.13 run `33795954634` executed the same schema-7 harness and again exposed
the common `slp_quad` two-band behavior. After four unmeasured conditioning
batches, candidate CK, pinned C, and pinned Rust still produced approximately
`4.42 ms` and `8.84 ms` samples. The Rust channel happened to place seven of
twenty stored samples in the faster band and failed the unchanged stability
gate; the preceding V0.12 run placed only two Rust samples there and passed.

The all-channel, exact-factor behavior after a fixed conditioning ramp shows
that scheduler migration between differently conditioned virtual CPUs remains
an uncontrolled benchmark variable. A single successful run therefore did not
close the original instability.

## Resolution

For every Linux three-channel vector/domain case, the harness now reads the
thread's inherited allowed CPU-affinity set, selects the current CPU when it is
allowed (otherwise the first allowed CPU), pins the measurement thread to that
single CPU before runner construction and conditioning, and restores the
original affinity when the case ends. Failure to read or apply the affinity is
a performance-gate failure rather than an unpinned fallback. Non-Linux local
benchmark use remains unchanged; release performance jobs run on Linux.

The existing four-batch same-runner conditioning remains in place. No timed
batch, sample count, upper-median statistic, channel rotation, threshold,
corpus, compiler artifact, or platform gate changes. Fresh exact-SHA Linux
performance jobs are required to prove the repair.
