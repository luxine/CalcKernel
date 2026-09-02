# CK 0.14 Design Adversarial Review 06

Review target: commit 5980d4c4c00276f65f2d23845b9dad3f42416df1

Reviewer: fresh read-only gpt-5.6-sol, ultra reasoning

Verdict: BLOCKED

Blockers: 3

## Blocking findings

### B1. `CK_TUNE_INPUT_MAP` count width is undefined

The map names a big-endian count but not its width, and does not explicitly bind
`Text` to one primitive framing or reject trailing bytes. A producer using U16 and
a runner using U32 can both follow the prose while disagreeing at the first row.

Minimum correction: freeze `U32_BE(count)`, record concatenation, `Text` framing,
and exact EOF.

### B2. Tuning CLI names conflict

The product CLI freezes `--config`, while the exact schema-9 tuned build recipe
requires `--workload`. Since unknown arguments fail closed and hidden aliases are
not authorized, a conforming product cannot execute its mandatory release recipe.

Minimum correction: use `--config` consistently in the performance template, or
normatively introduce one common alias contract.

### B3. Peak RSS lacks an authoritative high-water source

The supervisor accepts one or more sampled RSS rows and calls their maximum the
peak. A large transient between samples is invisible. The ordinary-build baseline
has no corresponding receipt at all and can be inflated to pass the ratio.

Minimum correction: freeze an authoritative per-process OS high-water observation
and units, retain its receipt, and measure tuned and ordinary processes with the
same protocol.

## Confirmed closures

The review confirmed destination aliases and staged-file identities, smoke/state
ordering, validation derivations, environment confidentiality, Windows argv,
historical and cumulative schema-8 separation, equal external work, expected
correctness, the build/decision/artifact graph apart from B2, cold/warm cache
provenance apart from B3, deterministic archive identity, publication recovery,
and general English/Chinese alignment.

Final verification found the requested HEAD and a clean index/worktree. The
reviewer made no edits.
