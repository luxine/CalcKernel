# CK 0.13 Implementation Blocker 02: Host Runtime Closure and V0.12 Rebase

## Decision

Exact-SHA run `33577761299` failed because of two profile-runtime portability defects. During the
same failure review, CK 0.12 run `33581641456` proved that the v0.12 replay pinned by CK 0.13 was no
longer a valid final candidate. All three findings are accepted blockers. None permits a language,
ABI, PGO, performance-threshold, corpus, statistic, or required-job relaxation.

## Rediagnosis

- Linux Native builds compile the repository-owned profile runtime with GCC and `-Werror`. The
  hexadecimal helper's conditional expression mixed promoted signed character arithmetic with an
  unsigned arm, triggering `-Wsign-compare` on both Linux hosts. Explicit branches and integer casts
  preserve the exact byte result and compile warning-clean.
- Apple Clang maps `fstat` to `_fstat$INODE64` for the x86_64 deployment target. The embedded LLD
  link uses the repository-owned `native/runtime/platform/libSystem.tbd`, which exported `_fstat`
  but omitted the inode64 spelling. Adding the real system symbol to the target-shared import list
  closes the link without changing CK's public ABI or adding a runtime implementation.
- CK 0.13 pinned v0.12 commit `1c2596da11242704cc6d875e969fc45cf58ea21d`. Subsequent v0.12
  cross-host CI found the defects documented by `specs/0.12/review/implementation-blocker-04.md`.
  The v0.13 design already requires a rebase when the v0.12 candidate changes, so every exact replay
  owner now pins `1009bae18d1a1ebd37ee9ee095cab9a965e69df8` and its recomputed manifest digest.

## TDD and evidence

1. A contract test first failed on the Linux ternary, then passed only after the warning-clean
   branch form was present. The source digest in profile-runtime provenance was recomputed from the
   resulting bytes.
2. A contract test first failed because the Darwin TBD lacked `_fstat$INODE64`, then passed after
   the export was added. A local Apple Clang cross-compilation for x86_64 produced an object whose
   actual undefined symbol list contains `_fstat$INODE64`.
3. The schema-8 pin contract first failed after changing its expected v0.12 SHA, then passed only
   after the replay scripts and schema documentation adopted the same exact commit. Repository and
   bilingual user documentation carry the identical pin.
4. The v0.12 blocker-04 commit was cherry-picked so v0.13 inherits its DLL lifetime, checked cold
   branch, full-domain oracle, target-specific reduction, and short-kernel conditioning fixes.

## Acceptance boundary

- CK source semantics, Native ABI 1, Runtime ABI 2, profile formats, PGO safety boundary, dispatcher
  behavior, and artifact formats remain unchanged.
- Schema-8 thresholds, training/held-out split, vector and domain corpora, sample counts, statistics,
  size limits, and ten required CI jobs remain unchanged.
- The new v0.12 replay SHA is exact and immutable; a branch name or locally cached compiler cannot
  substitute for it.
- Local AArch64 checks do not sign Linux GCC or Darwin x86_64 release jobs. A new exact-SHA v0.13
  workflow must pass all required jobs before the branch can be merged or called complete.

## Self-review

The Linux change removes a warning without altering output bytes. The Darwin change exports the
symbol emitted by the supported compiler rather than adding a shim. The replay migration follows an
existing design requirement and keeps every gate cumulative. No prior CI result is reusable for the
new candidate.

Verdict: the implementation blockers are closed in design; final acceptance remains pending the new
exact-SHA workflow.
