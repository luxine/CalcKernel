# Implementation blocker 63: inherit exact Windows PGO and target-setup repairs

## Evidence and exact base

V0.14 run `34281039339` on commit
`abad55eff21d0b0fb8dfa3afef8b6e9ef6e80a86` failed AArch64 replay preparation in
job `102246357595`: the exact V0.13 source-to-object multiversion geometric mean
was `2.5196146028`, above the unchanged `2.5` maximum with all samples retained.
The base V0.13 run `34257508363` additionally exposed incomplete Windows
verbatim-prefix queries and MSVC ARM64 outlined atomic imports.

V0.14 inherits the precise V0.13 repair
`f5dd9989245fd6d9e70babc95dcdf7af17ecb42f`. Its accepted-base identity, replay
preparer, manifest, current normative documents and tests are repinned together.
The replay manifest SHA-256 is
`229fecf4dd95610ae67309b683142b53f014abf4db655611b188e8330aca11f3`.
Historical blocker records retain the identities they originally diagnosed.

## Integrated changes

- Profile merge/read/output validation does not query an incomplete Windows
  drive prefix; complete root and ordinary components retain no-follow checks.
- Windows ARM64 profile-runtime compilation explicitly disables MSVC's default
  outlined interlocked functions, and bootstrap audits real undefined symbols.
  The existing C++20 atomic implementation, owner-only publication fix and
  Unix unused-function repair are retained.
- Native multiversion setup reads only closed tier descriptors and reuses the
  baseline TargetMachine. Complete real target profiles and digests are checked
  against explicit reconstruction; every platform's fixture descriptors match.

All language/public ABI, safety, strict-FP, eligibility, schema-8/schema-9
recipe-revision-2 thresholds, timed work, samples, corpus and required jobs are
unchanged. A replacement run must independently rebuild the exact V0.13 pin
and collect complete V0.14 evidence.

The old V0.14 x86 performance job `102246357195` again reports a real
infrastructure failure: its AMD EPYC 7763 lacks the five required AVX-512
features for x86-64-v4. This is not a compiler regression or a passing result;
acceptance still requires a genuinely capable worker.
