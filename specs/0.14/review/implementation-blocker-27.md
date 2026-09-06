# Implementation blocker 27: inherited dispatcher fact-audit ownership

Date: 2026-09-06

## Finding

Exact V0.13 run `34028252202`, AArch64 Linux native-host job
`101473242935`, failed its required Native suite with a pre-LLVM fact-audit
mismatch: expected six range and six assume strengthenings, but enumerated
four of each. Exact V0.14 run `34028600132` reproduced the same signature in
native integration job `101474136852` and x86-64 Linux native-host job
`101474136938`.

## Rediagnosis

The failure occurs only when an enhanced tier causes a dispatcher to be
installed. Dispatcher construction copies the baseline implementation's
function and parameter attributes, but its audit ledger previously duplicated
every property owned by the public root name. That incorrectly assigned the
baseline function body's assume/range/no-wrap/alias-scope records to the new
resolver-and-tail-call body, where those instructions do not exist. The
independent enumerator was correct and remains unchanged.

## Resolution

V0.14 imports the V0.13 repair at
`aa155825959e49d61fcea7a953935b179a7a238f`. The dispatcher ledger now
duplicates only parameter alignment, parameter noalias, readonly, writeonly,
and function memory effects—the exact strengthening categories inherited by
LLVM function attributes. Instruction-owned evidence remains attached only to
its original body.

The host-independent dispatcher regression uses the contracted
`call_constant_length.ck` fixture. It reproduced the remote 6-versus-4 failure
before the fix and now requires exact audit equality after verification.

The independently built V0.13 replay is repinned to the repaired commit. The
updated `benches/baselines/v0_13_replay.toml` SHA-256 is
`d8a0b50eba1957c980b4a6acfed0e2e2e481b9cd804cb8e1b44ae7c7b14129f5`;
no adapter is introduced.

No fact-audit equality rule, language or ABI contract, target policy,
performance or stability threshold, timed work, sample count, corpus, tuning
space, platform tier, or required CI job changed.

## Verdict

Accepted inherited implementation blocker. Local full-feature verification
must pass before restarting V0.14; only a new exact-SHA run may establish all
enhanced-tier and remote performance acceptance.
