# Implementation design correction 12: replay the repaired V0.13 candidate

## Trigger

Exact-SHA run `33757231836` failed in the x86-64 performance job while preparing
the pinned V0.13 schema-8 replay.  The replayed compiler at
`4cbd3d0624a6ccbd7a8a003a04e201201ec019a8` attempted to name a `void` LLVM call
when compiling `call_constant_length.ck`; LLVM's assertion-enabled build aborted.

## Rediagnosis

This is not a V0.14 tuning threshold failure.  The V0.14 compiler already emits
unnamed void calls, but schema 9 intentionally executes the exact pinned V0.13
compiler and requires its schema-8 evidence to pass its own checker.  Keeping the
known-failing revision would therefore make honest V0.14 acceptance impossible.
Skipping the case, patching a detached replay checkout, or accepting an aborted
schema-8 report would weaken the historical boundary and is prohibited.

## Correction

V0.13 fixes the bridge at exact candidate revision
`b61d45831f3f351a486722dcd12560507013db1c` and adds a subprocess regression for
the exact multiversion dynamic-library command.  V0.14 now pins that whole
revision directly.  `benches/baselines/v0_13_replay.toml`, the preparation
script, tests, master-control material, and both normative design languages are
updated together; the manifest SHA-256 is
`02d8f1c3b8f8a8c5f2b2a0d42175d0b12fb0117b475ddd63e5030453ec0d9f84`.

The historical checker, schemas, fixtures, sample statistics, performance
thresholds, and replay requirement are unchanged.  Remote acceptance must build
the repaired exact revision without adapters and must still produce and validate
the complete schema-8 evidence before schema 9 can pass.
