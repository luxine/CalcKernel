# Implementation blocker 43: inherit resolver-entry fact lineage

Date: 2026-09-07

## Finding

Replacement exact V0.13 run `34116187720` failed in native integration,
x86-64 Linux, and AArch64 Darwin before optimization. The generated resolver
entry inherited the baseline's valid `readonly` and `writeonly` attributes, but
the CK fact ledger registered inherited lineage only for the steady dispatcher.
The unchanged fail-closed audit therefore observed four instances while it
expected three.

## Resolution

V0.14 inherits exact V0.13 repair
`dd6239e677720845dee874ac3095710941b58d5b`. The closed inherited property set
is recorded once for the cold resolver entry and once for the steady dispatcher.
No attribute is removed; assume, range, no-wrap, and alias-scope body evidence
remains excluded from both copies. The complete V0.13 delta was integrated, and
the V0.14-specific profile-runtime contract tests were preserved during the
mechanical conflict resolution.

The V0.13 replay manifest is repinned to that exact commit with SHA-256
`cc16808de13642e65c843668e60ac93ca2db0d0c6b4d1247346544fe5bcfcea3`.

## Frozen boundaries

No fact-audit equality, language or ABI rule, tuning choice, target ISA,
performance or stability threshold, timed work, sample count, corpus, platform,
or required job changed. V0.13 and V0.14 remain independently accepted by
their own exact-SHA workflows.

## Verdict

Accepted inherited implementation blocker. Both replacement workflows remain
authoritative.
