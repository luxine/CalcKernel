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
`0f9af4ae032c0c3248caff60993795e669d3f8b4` and adds a subprocess regression for
the exact multiversion dynamic-library command.  V0.14 now pins that whole
revision directly.  `benches/baselines/v0_13_replay.toml`, the preparation
script, tests, master-control material, and both normative design languages are
updated together; the manifest SHA-256 is
`8f454cac97608432a462d6de89949264d5ab5cc33ee9b94c7cb933829bdb72a0`.

The historical checker, schemas, fixtures, sample statistics, performance
thresholds, and replay requirement are unchanged.  Remote acceptance must build
the repaired exact revision without adapters and must still produce and validate
the complete schema-8 evidence before schema 9 can pass.
