# Implementation blocker 28: stripped multiversion products and single publication

Date: 2026-09-07

## Finding

Exact V0.13 run `34051103711`, AArch64 performance job `101534772807`,
passed the unchanged cumulative schema-7 gate and all schema-8 runtime gates,
but failed the schema-8 aggregate artifact-size gate. Dead-section elimination
from blocker 27 reduced the five multiversion artifacts to 4,048, 4,008,
4,032, 4,160, and 4,336 bytes, but their 20,584-byte total remained
`2.13704` times the 9,632-byte ordinary total, above the required `2.0`.
V0.14 exact replay job `101535896793` reproduced the same authoritative
failure while preparing its pinned V0.13 performance bundle.

No threshold, workload, sample count, statistic, platform, or required job may
change.

## Rediagnosis

The final ELF products still retained the complete local symbol and string
tables even though only the requested CK dynamic exports are loader-visible.
The selected-direct evidence path was the sole consumer of one local dispatch
slot name, so retaining every compiler-private local name in every shipped
library was not a product requirement.

The runtime also cached the normalized capability bitset in a second global
atomic state machine. Every generated public dispatcher already owns an
acquire/release function-pointer slot: concurrent first callers may detect in
parallel, but the resolver publishes exactly one verified compatible pointer,
and later calls use that pointer without querying capabilities. The second
cache therefore duplicated publication and increased one-shot resolver code.

This was final-product metadata and private-runtime size debt, not a reason to
weaken multiversioning or its size gate.

## Resolution

ELF shared links now use LLD `--strip-all` after retaining the dynamic export
table and place the single generated resolver slot in a dedicated private
`.ck_dispatch_slot` `NOBITS` section. The selected-direct collector reads the
public address from `.dynsym` and the exact pointer-width/aligned slot from that
section in a byte-identical copy of the shipped artifact. Missing, duplicated,
mis-sized, TLS, non-writable, non-allocated, or malformed slot evidence fails
closed. No private slot becomes an export or public ABI symbol.

Capability detection is now one-shot work owned by each unresolved dispatcher;
the generated acquire/release slot remains the only publication layer. The
private detector is compiled size-first (`-Oz` on Clang/GCC and `/O1` on MSVC)
because it is outside steady-state execution. The target checks, Linux AArch64
startup-stack and `/proc/self/auxv` sources, baseline failure policy, resolver
ranking, public thunk, and steady-state atomic load/tail call are unchanged.

Applying the production link shape to the failed job's exact retained AArch64
archives produced a five-case multiversion total of 14,792 bytes against an
8,544-byte ordinary total (`1.73127`). This local reconstruction includes a
longer development LLD provenance string than the hosted artifact and remains
comfortably below the unchanged `2.0` gate. Authoritative Linux AArch64 size
and performance remain the CI gate.

No CK language or public ABI, strict floating or safety rule, target ISA,
candidate frontier, profitability floor, code-growth budget, performance or
stability threshold, timed work, sample count, corpus, platform, or required CI
job changed.

## Verdict

Accepted implementation blocker. Complete local verification must pass before
publishing a new exact candidate SHA. The Linux AArch64 performance job and all
other required jobs remain mandatory.
