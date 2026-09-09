# Implementation blocker 07: repaired V0.12 replay identity

Date: 2026-09-04

## Finding

The V0.13 acceptance chain still replayed V0.12 commit
`1009bae18d1a1ebd37ee9ee095cab9a965e69df8` after the V0.12 branch had fixed
its native compile-timing and cross-platform code-model defects. Keeping the
superseded replay would let V0.13 pass against a compiler that no longer
represents the exact V0.12 candidate submitted for acceptance.

The V0.12 x86-64 failure also exposed an LLVM Large code-model default that is
shared by V0.13's target-machine helper. V0.13 therefore needed the same
unconditional PIC plus Small policy rather than only changing replay metadata.

## Resolution

The V0.12 replay is pinned to repaired exact commit
`d83805075b0ac8986c895b7a287c84eac509b7f9`; its manifest digest and every
executable contract now bind that identity. V0.13's shared target-machine
helper selects Small on all supported native object formats, and a structural
test rejects restoration of a Mach-O-only guard.

No language, ABI, PGO policy, performance threshold, sample count, corpus, or
platform gate changes. V0.13 must rebuild the exact replay and pass the
existing schema-7 and schema-8 gates on a fresh exact-SHA CI run.
