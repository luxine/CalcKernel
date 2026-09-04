# Implementation blocker 17: fixed ramps cannot select the AArch64 execution band

Date: 2026-09-04

## Finding

Exact V0.12 run `33825887411` failed AArch64 performance job
`100878495028`. A fixed 64-batch ramp left all three unchecked `slp_quad`
channels split almost evenly between approximately 4.42 ms and 8.84 ms. The
candidate stream failed the unchanged stability rule. Doubling the preceding
fixed ramp therefore changed which band became the median but did not control
the band selected for the following seven timed calls.

## Rediagnosis

The hosted Neoverse-N2 execution state is persistent across each seven-call
group but transitions independently of a fixed amount of preceding work.
Increasing a blind fixed count again would have no evidence-based stopping
condition and could merely move the transition into another sample. The gate
needs to observe only unmeasured settling probes and fail closed unless the
same conservative duration band is re-entered before timing.

## Resolution

`bounded-upper-band-v1` calibrates each CK, C, and Rust channel independently
from 64 unmeasured full-batch probes. The nearest-rank 9/10 duration quantile is
the anchor and `ceil(anchor * 3/4)` is the sustained-band floor. Immediately
before each retained seven-call sample, at most 256 unmeasured probes may be
used to hit that floor. Exhaustion is an error; no mixed-band sample is silently
discarded or retained, and no probe enters the timed region.

No timed work, twenty-sample count, seven-call count, rotation, upper-median
statistic, stability rule, performance threshold, corpus, target policy, or
required platform/job matrix changed.
