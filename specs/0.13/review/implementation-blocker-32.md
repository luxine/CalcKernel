# Implementation blocker 32: internal function-entry call-site batching

Date: 2026-09-07

## Finding

Exact V0.14 run `34100659848`, x86-64 performance job `101674461128`,
independently rebuilt exact V0.13 commit
`002100719bdefdabb0fece50a363e1b797c464d2` and failed the unchanged schema-8
generation-overhead gate:

`branch-layout generation execution exceeds generationOverhead=5.0`.

The generation median was `782,368 ns` and the ordinary median was
`152,662 ns`, a `5.1248x` overhead. All twenty retained samples were stable.
The complete failed job log and `performance-x86-64` artifact retained the
report, KIR, generated objects, profiles, and full replay evidence. No evidence
supports changing the threshold, work, sampling, or corpus.

## Rediagnosis

The retained generation object showed two calls to the atomic
`__ck_profile_increment` helper in every `branch-layout` loop iteration. They
were the exact function-entry observations for the internal `add_path` and
`subtract_path` helpers after LLVM inlined their arithmetic into `kernel`.
Edge and candidate observations were already locally batched; initialization
was already kept out of the hot path. The two remaining per-iteration atomic
publications, rather than the observed algorithm, caused the narrow but stable
overhead failure.

Every CK call is statically named and CK has no function-pointer entry path.
Therefore an internal, non-entry, non-exported callee's function-entry event
can be counted exactly at each static call site. Exported functions and the
module entry must continue to publish their own entry event because external
callers are not represented in KIR.

## Resolution

Profile-generation lowering now relocates only internal function-entry
increments to caller-local saturating counters. A caller allocates one counter
per referenced internal entry site, increments it at every static call, and
publishes the exact accumulated value through the existing atomic bulk-add
helper on every normal or checked-failure return. Exported and module-entry
functions retain direct entry publication. Recursive and repeated calls remain
exact because every executed static call contributes once; local and global
overflow behavior remains saturating.

A RED/GREEN structural contract requires call-site entry storage, update, and
flush. A Native runtime regression
executes an internal helper 8,000 times through two exported calls and verifies
the exact function-entry counts `[2, 8000]`. Profile-generation products are
transactional direct outputs and are never read from either native object cache,
so no cache identity can splice an older generation object.

The site table, function-entry observation meaning, profile schema, runtime
ABI, language and public ABI, safety rules, optimization result, target ISA,
performance/stability threshold, timed work, sample count, corpus, platform,
and required CI job remain unchanged.

## Verdict

Accepted implementation blocker. Focused and complete local verification must
pass before publishing a replacement exact candidate. Fresh exact-SHA x86-64
CI remains authoritative for the unchanged generation-overhead gate.
