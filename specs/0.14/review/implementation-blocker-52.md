# Implementation blocker 52: isolate the retained V0.13 checker identity

## Evidence

Exact-SHA workflow-dispatch run `34182332164`, V0.14 commit
`914db1af0773ab0e2d1ab1f00e6e32f73cf7ce05`, reached schema 9 after both
performance jobs passed the unchanged cumulative schema-7 and schema-8 gates.
The AArch64 job `101923906526` then collected the complete schema-9 report but
rejected its retained V0.13 report with:

```text
schema-9 retained v0.13 historical evidence failed:
GITHUB_SHA does not equal the checked-out candidate SHA
```

The retained checker runs in a detached checkout of exact V0.13 commit
`77e5e0a95b83d0faa8f63ddc8f2451a9b1322a40`, while GitHub exports the outer
V0.14 SHA as `GITHUB_SHA`. The checker correctly compares that environment
identity to its checkout; the schema-9 parent failed to establish the historical
subprocess identity boundary.

The x86-64 job `101923906271` was independently assigned an AMD EPYC 7763 host.
Its complete CPU record has AVX2/FMA/BMI2 but no AVX-512 feature set, so it
truthfully rejected the host as x86-64-v3 before producing schema-9 evidence.
The normative x86-64-v4 requirement remains unchanged; a replacement run must
receive a real v4 worker.

## Repair

`schema9_check_replay` now derives the retained checker's environment from the
current process and binds `GITHUB_SHA` to `v013ReplayBundle.commit` before
launching the checker in its detached checkout. A focused regression test
captures the historical subprocess environment and requires that exact commit.
It failed before the implementation change because no subprocess environment was
provided, then passed with the isolated identity.

This does not rewrite or relax historical evidence. The retained report,
checker, manifest, compiler, archive, file closure, detached checkout, and commit
checks remain intact. No language or public ABI rule, strict-FP or safety
semantic, target ISA, schema-8/schema-9 shape, performance/stability/artifact-size
threshold, timed work, sample count, corpus, platform, or required job changes.

## Acceptance

The focused schema-9 regression and complete schema-9 mutation suite must pass,
followed by the repository's unchanged local gates. A replacement exact-SHA run
must execute all ten jobs. Its AArch64 schema-9 checker must accept the retained
V0.13 evidence under the historical SHA, and its x86-64 performance job must run
on a host that actually satisfies the frozen v4 feature requirement. A v3 host
remains an actionable capability failure and cannot sign the release.
