# Implementation blocker 53: bind retained V0.13 replay dependencies

## Evidence

Replacement exact-SHA workflow-dispatch run `34187453075`, V0.14 commit
`0b919d1434d905129744532a385be162686ed654`, proved that blocker 52's detached
checkout identity repair works: the AArch64 performance job `101938700087`
advanced beyond the former `GITHUB_SHA` mismatch. It then rejected the retained
historical report with:

```text
schema-9 retained v0.13 historical evidence failed:
v0.12 replay recipeSha256 does not match pinned identity
```

The complete artifact contains the retained V0.13 report beside its exact
`schema8/replay-v012`, `schema8/replay-v011`, and `schema8/replay-v010` bundles.
All three retained manifests record V0.13's expected recipe SHA-256
`ac4fda10731b259a2eb5ca91cd057c98480a263fe99a2d23e8079074573b31eb`.
The outer V0.14 runtime bundles record the distinct current recipe
`104e7fe1252eb3bb749fcf20895bee5cc71476c9f3cb606ed96f1bbc6748a47e`.
The historical checker inherited the outer `CKC_V012_RUNTIME_BUNDLE`,
`CKC_V011_RUNTIME_BUNDLE`, and `CKC_V010_RUNTIME_BUNDLE` paths, so it validated
the wrong dependency closure.

The x86-64 performance job `101938700084` was again assigned an AMD EPYC 7763
x86-64-v3 host and correctly rejected it before schema-9 evidence collection.
The frozen x86-64-v4 requirement remains unchanged.

## Repair

`schema9_check_replay` now resolves the retained historical report once and
binds each historical runtime-bundle environment variable to the corresponding
absolute sibling directory inside the retained schema-8 closure. The existing
detached-checkout `GITHUB_SHA` binding remains exact. A focused regression test
captures the subprocess environment and requires all four historical identities.
It failed against the old implementation because the three bundle paths were
absent or inherited, then passed after the minimum environment-boundary change.

The complete failed AArch64 artifact was replayed with the repaired checker. It
advanced beyond the reported recipe mismatch and stopped only when the local
Darwin Clang installation could not reproduce the remote Linux profile-runtime
identity, preserving the next platform truth check.

No evidence is rewritten and no check is skipped. The historical report,
checker, manifest, compiler, archive, complete file closure, detached commit,
and nested replay identities remain mandatory. No language or public ABI rule,
strict-FP or safety semantic, target ISA, schema-8/schema-9 shape,
performance/stability/artifact-size threshold, timed work, sample count, corpus,
platform, or required job changes.

## Acceptance

The focused environment regression and complete schema-9 mutation suite must
pass, followed by all unchanged local gates. A replacement exact-SHA ten-job run
must validate the retained V0.13 report against its own three replay bundles on
AArch64. The x86-64 job must receive a real v4 worker; a v3 allocation remains a
hard failure and cannot sign release acceptance.
