# Implementation blocker 39: inherited internal entry publication overhead

Date: 2026-09-07

## Finding

Exact V0.14 run `34100659848`, x86-64 performance job `101674461128`,
rebuilt accepted V0.13 commit
`002100719bdefdabb0fece50a363e1b797c464d2` and failed the unchanged schema-8
generation-overhead gate for `branch-layout`. The generation median was
`782,368 ns`; the ordinary median was `152,662 ns`; the ratio was `5.1248x`
against the frozen `5.0x` maximum. All twenty retained samples were stable.

The complete job log and `performance-x86-64` artifact preserved the report,
KIR, objects, profiles, and replay evidence. Disassembly of the retained exact
x86 generation object showed two atomic `__ck_profile_increment` calls per hot
loop iteration for the internal `add_path` and `subtract_path` function-entry
sites. LLVM had inlined their arithmetic, but the entry publications remained
as calls. Initialization, edge observations, and candidate observations were
already outside the per-element atomic path.

## Rediagnosis and dependency repair

CK calls are statically named and the language has no function-pointer call
path. Accepted V0.13 repair `280388a396e85f8876300427cd56b627b08b3e45`
therefore moves only non-exported, non-module-entry function-entry observations
to caller-local saturating counters at each static call site. The counters are
published through the existing atomic bulk-add on every normal or
checked-failure return. Exported and module-entry functions continue direct
entry publication so calls originating outside KIR remain observable.

The repair was inherited exactly into V0.14 by cherry-pick
`ea367f28e5a33c1830d9d0baa5567f1e5f007302`. A structural RED/GREEN contract
requires allocation, call-site update, and return-path flush. A native runtime
regression calls an internal helper 8,000 times through two exported calls and
requires exact function-entry counts `[2, 8000]`. Local generated-object
disassembly also confirms that helper entry counting is register-local in the
hot loop and published only on exit.

The accepted replay identity is re-pinned to exact V0.13
`280388a396e85f8876300427cd56b627b08b3e45`; the resulting
`benches/baselines/v0_13_replay.toml` SHA-256 is
`81a052192afc6ae7d124f6ae36e56636e0c0d26b72f29c510a98a41dab559d17`.
Profile-generation products are transactional direct outputs and are not read
from either native object cache, so this profile-only lowering change requires
no unrelated ordinary or multiversion cache invalidation.

## Contract preservation

The site table, function-entry meaning, profile schema, runtime ABI, language
and public ABI, safety semantics, target ISA, tuning choices, performance and
stability thresholds, timed work, samples, corpus, platforms, and required job
matrix are unchanged. V0.14 continues to require its independent schema-8
replay and schema-9/Contract-1 acceptance; no V0.13 result substitutes for it.

## Verdict

Accepted implementation blocker. Complete local gates and a fresh exact-SHA
V0.14 run remain mandatory before final acceptance.
