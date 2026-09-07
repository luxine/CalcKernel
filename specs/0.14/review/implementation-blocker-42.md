# Implementation blocker 42: inherit branch-free steady dispatch

Date: 2026-09-07

## Finding

Exact V0.13 run `34106156091`, x86-64 performance job `101691953958`,
failed the unchanged `trip-unroll-simd` dispatch/direct regression gate. The
twenty stable samples measured 14,142 ns for public multiversion dispatch and
13,272 ns for the exact selected member, a ratio of about 1.0656 above the 1.05
ceiling.

Disassembly showed that every public call still performed an acquire load, a
null test, a conditional branch, and an indirect jump after the one-shot target
had already been published. The selected-direct artifact used the byte-identical
library and exact published slot, isolating this steady dispatcher overhead.

## Resolution

V0.14 inherits exact V0.13 repair
`7a292bc74a87591437fc87ceb06df1fbb1bb28fd`. Each private slot is initialized
to a cold, noinline resolver entry with the exact target function type, calling
convention, and attributes. That entry runs the existing one-shot resolver,
publishes through the unchanged acquire-release compare-exchange, and
must-tail-calls the winning member. The steady dispatcher is now only one
acquire load followed by one indirect must-tail call.

The Native codegen/cache identity advances to
`dispatch-resolver-sentinel-v2`. Tests cover the absence of a hot-path null
branch, exact void/non-void call formation, and resolver-entry ABI behavior.
The full V0.13 delta was cherry-picked without adaptation.

V0.14's replay manifest now pins that exact commit; its SHA-256 is
`0cd154de78d8b305e73f9707ee8236fdb6064977b4f0be136a814d1a4fdda6e2`.

## Frozen boundaries

No CK language or public ABI, profile format, member selection, target ISA,
performance or stability threshold, timed work, sample count, corpus, platform,
or required job changed. V0.13 remains independently subject to its own
exact-SHA ten-job acceptance.

## Verdict

Accepted inherited implementation blocker. Both the replacement V0.13 run and
the repinned V0.14 run remain independently authoritative.
