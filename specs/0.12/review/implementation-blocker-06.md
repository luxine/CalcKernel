# Implementation blocker 06: dead loop-legality analysis exceeded the AArch64 optimizer gate

The exact-SHA AArch64 performance job in run `33757231916` failed after the
sampling repair exposed a stable `example-dijkstra` KIR optimizer result of
`2,134,281 ns` against the frozen V0.10 result of `703,886 ns`.  The resulting
`3.0322x` ratio exceeded the unchanged `3x` individual limit.

Profiling showed that an ordinary conservative Native KIR profile, which has no
legal vector operations, still ran full dependence legality analysis for each
of Dijkstra's three innermost loops.  No later vector transformation could
consume those results.  The pipeline now preserves the named legality pass and
its verification record but does not run candidate dependence analysis when
the target profile makes vector operations unavailable.  A regression asserts
that such a profile reports zero legality candidates.

The frozen baseline and acceptance threshold are unchanged.  On the same local
10,000-iteration `example-dijkstra/kir-o3` benchmark, the median fell from
`1.373 ms` to `1.179 ms` (about `14.1%`).  The complete non-Native test suite,
format check, and Clippy with warnings denied pass after the repair.  Exact-SHA
remote performance evidence remains required for acceptance.
