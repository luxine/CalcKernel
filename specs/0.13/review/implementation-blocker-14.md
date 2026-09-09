# Implementation blocker 14: current Darwin SDK lowers `fstat` to `fgetattrlist`

Date: 2026-09-04

## Finding

The final local all-features gate failed four real PGO CLI tests while linking
the generation executable. Embedded LLD reported `_fgetattrlist` as undefined
from `profile-runtime.o`.

## Rediagnosis

The source-owned profile runtime still calls `fstat` to validate the opened
profile directory. The current macOS SDK lowers that call to the stable
`_fgetattrlist` system entry rather than either `_fstat` spelling already
present in the compiler-owned `libSystem.tbd`. Inspection of the exact embedded
profile runtime object confirmed `_fgetattrlist` as its only unexpected import,
and the installed SDK exports that symbol from libSystem for both Darwin target
architectures.

This is a closed import-description drift, not a profile, optimizer, or public
ABI failure.

## Resolution

The fixed Darwin import description now lists `_fgetattrlist` alongside
`_fstat` and `_fstat$INODE64`. A contract test requires all SDK spellings, and
the four previously failing PGO CLI paths link and execute successfully with
the exact embedded runtime.

No arbitrary library path, external linker, dynamic symbol lookup, public
export, runtime ABI, performance threshold, corpus, sample, or platform/job
requirement changed.
