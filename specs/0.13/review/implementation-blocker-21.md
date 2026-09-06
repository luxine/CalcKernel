# Implementation blocker 21: dispatcher fact-audit ownership

Date: 2026-09-06

## Finding

Exact V0.13 run `34028252202`, AArch64 Linux native-host job
`101473242935`, failed the required Native suite while building the contracted
void-call multiversion fixture. The pre-LLVM fact audit expected six range and
six assume strengthenings but enumerated four of each. The same failure was
reproduced locally with the existing host-independent dispatcher seam before
changing production code.

The V0.14 exact run `34028600132` independently reproduced the identical
failure in native integration job `101474136852` and x86-64 Linux native-host
job `101474136938`.

## Rediagnosis

`NativeModule::add_multiversion_dispatch` correctly duplicates audit records
for attributes that LLVM copies from the baseline implementation onto the
internal dispatcher. It previously duplicated every record owned by the
public root name, including `llvm.assume`, range, no-wrap, and alias-scope
records attached to instructions in the baseline function body. The dispatcher
contains only its resolver and indirect tail-call body; those instruction-level
strengthenings are not copied. The ledger therefore over-counted two root
contract assumptions even though LLVM IR contained the correct four calls.

This was not missing emitted evidence and must not be repaired by weakening or
bypassing the audit.

## Resolution

Dispatcher construction now duplicates only the strengthening kinds actually
carried by `Function::setAttributes`: parameter alignment, parameter noalias,
readonly, writeonly, and function memory effects. Range/assume, integer
no-wrap, and alias-scope records remain owned only by the function bodies that
contain their LLVM instructions.

A new behavioral regression lowers the contracted
`call_constant_length.ck` fixture, installs a synthetic enhanced dispatcher on
every host, verifies the resulting LLVM module, and requires exact fact-audit
equality. It failed with the remote 6-versus-4 signature before the fix and
passes afterward.

No fact is accepted without a corresponding LLVM strengthening. No language
or ABI rule, target policy, performance or stability threshold, timed work,
sample count, corpus, platform tier, or required CI job changed.

## Verdict

Accepted implementation blocker. The local RED is closed without changing the
independent enumerator or expected-count equality; a new exact-SHA run remains
the authority for all enhanced-tier hosts.
