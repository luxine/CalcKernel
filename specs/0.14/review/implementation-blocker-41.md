# Implementation blocker 41: detached historical checker path ownership

Date: 2026-09-07

## Finding

Exact V0.14 run `34106159689`, AArch64 performance job `101691955647`,
completed the unchanged cumulative schema-7/schema-8 gates and collected the
full schema-9 evidence closure. The final checker then rejected the retained
V0.13 replay with `No such file or directory` for
`target/ckc-perf/.../replay-v013/schema8/v0.13-results.json`.

The uploaded artifact contains that exact report and its complete evidence
tree. The failure is therefore not missing evidence and not a performance
regression.

## Root cause

The outer schema-9 invocation accepted a repository-relative report path, so
its derived evidence root also remained relative. The historical checker is
intentionally executed from a temporary detached V0.13 checkout, but the
retained report argument was passed unchanged. The child process consequently
resolved a valid path against the detached checkout instead of the V0.14
evidence owner.

## Resolution

`schema9_check_replay` now resolves its evidence root to an absolute path before
crossing the detached-checkout process boundary. A regression constructs a
relative evidence root, captures the historical checker command, and requires
the retained report argument to be absolute. Existing identity, byte, tree,
symlink, commit, and historical-checker verification still runs unchanged.

No report is copied, rewritten, relocated, or synthesized by the checker.

## Frozen boundaries

No language or ABI rule, tuning choice, performance or stability threshold,
timed work, sample count, corpus, platform, required job, replay commit, or
evidence identity changed. The correction only makes the already-owned retained
path independent of the verifier child's working directory.

## Verdict

Accepted implementation blocker. The repaired exact-SHA AArch64 and x86-64
performance jobs remain authoritative.
