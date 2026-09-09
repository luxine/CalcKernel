# Implementation blocker 67: inherit bounded v4 integer tails

Exact V0.14 `d964d7ddbfd234cf87bf462489214695187611e7` run `34307207415`
failed its independent V0.13 replay at memory-bound dispatch/ordinary =
20,954 / 20,337 = 1.0303387914 on Xeon 6973P-C. The selected-direct median was
20,963 ns. The full stable cohort and all artifacts were retained.

The V0.13 independent run also failed memory-bound combined performance. Its
retained objects show the same four-way interleaving of a 16-lane vector body
and purely scalar remainder. The precise diagnosis, test-first correction and
local verification boundaries are recorded in
`specs/0.13/review/implementation-blocker-50.md`.

V0.14 inherits exact V0.13 repair
`d85e0c786aaeeaa4dbaab9bffa01fcbd5f7c9f5a`. Only the existing restricted v4
integer-map schedule acquires interleave 1, bounding its scalar tail to fewer
than 16 elements. The V0.14-only AArch64 fixed-map codegen identity is retained.
The replay manifest and both executable pin checks now name that V0.13 SHA;
the manifest SHA-256 is
`fe5233fec2525726db8e46a864cca5fe179eef91988baf295fd070d9957d068c`.

No threshold, timed work, sample count, corpus, platform or required job was
changed. Checked-space/checked-plan replay reuse and all mandatory identity,
legality and publication checks remain intact. Both branches require new
independent exact-SHA CI; neither local diagnostics nor the older run sign off
the final runtime-performance or compilation-performance acceptance.
