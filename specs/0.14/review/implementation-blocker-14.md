# Implementation blocker 14: superseded v0.13 runtime base

Date: 2026-09-04

## Finding

The v0.14 candidate at `9620855aa43323cdf13049d7dc587760c6b8a503`
still pinned v0.13 candidate `b61d45831f3f351a486722dcd12560507013db1c`.
Exact v0.13 CI run `33782668586` subsequently exposed two independent
AArch64 Linux defects: architecture-incompatible directory flags in the
freestanding profile runtime and an LLVM name assigned to a `void`
multiversion dispatch call. Therefore the old v0.14 replay base and its CI
run `33784396848` cannot serve as final evidence.

## Resolution

- v0.14 integrates the two source fixes and their regression contracts.
- The accepted v0.13 base advances exactly to
  `d5a2491672477634070b0c36b77cb8ad4bf7df56`.
- The schema-9 replay manifest is regenerated with digest
  `cc33d80608fea92bb90e4c42ebf1977736bf0c251adb3d0f465449b69f87a51e`.
- The v0.14 profile runtime provenance is recomputed from its own source
  closure instead of copying the v0.13 provenance record.

No language, ABI, tuning policy, performance threshold, sample count, or
benchmark corpus changes. The replacement candidate requires a fresh exact-SHA
CI run; the superseded run must not sign the final acceptance record.
