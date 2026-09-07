# Implementation blocker 36: private tuning-cache fixture mode

## Verdict

Confirmed implementation blocker. Exact v0.14 run `34090234424`, AArch64
performance job `101642274522`, passed cumulative schema 7 and schema 8, then
failed schema-9 collection before the first cold tuning session:

`tuning cache directory is not owner-only`.

The rejected path was the collector-owned
`cache/branch-layout/cold-one/ckc` namespace. No threshold, workload, sample,
statistic, platform, or required job may change.

## Rediagnosis

The schema-9 collector snapshots the cache before invoking `ckc`. Its
`snapshot_cache` helper created the namespace with the process-default `0755`
mode. `TuneCache::open_default` then correctly rejected that already-existing
directory under its frozen owner-only `0700` security contract. The compiler
was not responsible for creating or repairing the unsafe fixture directory.

## Correction

The collector now creates its owned cache namespace with explicit `0700`
permissions and reapplies that mode before every snapshot on POSIX. A RED/GREEN
contract regression constructs the same nested cold-cache path and requires
its final namespace mode to be exactly `0700`.

The compiler's no-follow, ownership, mode, salt, entry, and fail-closed cache
checks remain unchanged. No language or public ABI, tuning choice, cache key,
performance threshold, timed work, workload, sample count, statistic,
platform, or required CI job changed.
