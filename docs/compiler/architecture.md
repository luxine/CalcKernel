# CalcKernel 0.14 Compiler Architecture

[简体中文](../zh-CN/compiler/architecture.md)

Public behavior is defined by the language, CLI, MIR, compatibility, and ABI
documents. Module names in this document explain implementation ownership.

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker/contracts
    -> semantic MIR lowering/validation
    -> consumer reachability + mode-specific KIR v3 construction
    -> optional CK workload profile (non-proof) + target profile
    -> KIR verifier -> transactional optimizer -> KIR verifier
    -> optional same pre-state CPU variants -> baseline-safe dispatcher
    -> optional explicit offline tuning -> independently replayed exact plan
    +-> C source/header
    +-> WAT/WASM
    +-> structural LLVM -> TargetMachine object -> ORC or in-process LLD
```

`src/frontend/` owns coordinates, stable diagnostics, parsing, checked types,
definite return/unreachable rules, unsafe blocks, and typed closed contracts.
Contracts are metadata over mathematical integers; they are not executable CK.

`src/ir/mir/` owns semantic MIR, deterministic `emit-mir`, and validation.
MIR preserves source evaluation, possible checked failure, and print order. It
is intentionally mode-neutral and no longer has an optional product optimizer.
`MirType::Slice`, `MakeSlice`, `SliceIndex`, and `Subslice` retain descriptor
semantics without adding backend-specific checks.
Structured `break` and `continue` lower to the appropriate innermost target as
`MirTerminator::Jump`; `void` calls/returns use no synthetic value.

`src/ir/kir/` builds consumer-, mode-, target-, and CPU-specific KIR v3. KIR
contains scalar and fixed-vector SSA, block parameters, region Memory SSA,
explicit guards, ordered effects, runtime predicates, and a canonical
`KirTargetProfile`. The profile records the complete fixed query universe,
operation legality/costs, vector lanes, alignment, consumer, target, and CPU
policy; its deterministic digest is part of every native object/cache identity.
The builder first prunes unreachable code for the selected artifact and rejects
an unsupported runtime capability. Library roots are exports, executable roots
are `main`, and inspection roots are their union.

`src/profile/` owns CK workload profiles; this is separate from the static
`KirTargetProfile`. Canonical `CKPART01` shards are merged into one validated
`CKPROF01` terminal profile whose complete compiler/source/KIR/site/target/mode
identity must match before use. The profile is immutable non-proof evidence: it
records frequency and work, but cannot create range, alias, alignment, effect,
bounds, dominance, or other safety facts. A profile mapping across a CFG change
requires an independently checked closed transfer record; ambiguity falls back
to ordinary optimization.

`src/optimizer/` owns scalar/path, natural-loop, access/dependence, alias/region,
Memory SSA, SLP, and interprocedural effect analysis; fact and proof arenas;
pass management; and independent transformation checkers. O3 adds bounded
specialization, loop normalization, unroll/SLP, and Loop SIMD with scalar
epilogues and at most one total alias predicate plus scalar fallback. Every
speculative pass prepares a complete candidate state and audit delta, checks it
against the immutable verified pre-state, and commits both atomically or rolls
both back. Facts distinguish proven analysis from trusted contract instances.
Each unsafe call gets a separately scoped instance. A pass cannot remove a
guard, duplicate a region, or emit a backend fact unless a closed certificate or
auditable contract fact remains valid in the current CFG and Memory SSA state.
O2 first freezes the ordinary machine pipeline and permits profile influence
only in `CkLateProfileLayout`; the accepted delta is block/trace order plus the
closed target repair allowlist, never new LLVM profile metadata. O3 proposes
profile-guided transforms as transactions from one immutable pre-state. The
proposer cannot approve itself, rejected work consumes its budget, and every
candidate either commits a complete verified state or leaves the pre-state
unchanged. Multiversion planning starts baseline and all enhanced variants from
the same pre-state, verifies them independently, and forbids cross-variant LTO.

`src/backend/` consumes verified KIR only. C and Native support the four
overflow/bounds combinations through explicit guards and status flow; WASM is
unchecked-only. C and WebAssembly profiles deliberately disable Vector KIR in
0.14, so both continue from verified scalar KIR while retaining profitable
scalar specialization and cleanup. C contract facts may become portable
restrict/alignment hints. Native structurally lowers checked Vector KIR and the
same scalar facts to LLVM IR, validates metadata with a pre-LLVM fact audit,
verifies the module, and emits object bytes with the host TargetMachine.

`src/backend/llvm/` and `native/bridge/` provide typed ownership across the
Rust/C++ boundary. `native/runtime/` owns entry, checked and sanitizer runtime
diagnostics, and print effects. LLD and ORC execute in process. The public
Native C ABI remains version 1; the private LLVM bridge ABI is version 4 and the
contract-aware runtime ABI remains version 2.

`native/profile_runtime/` exists only in generation artifacts and publishes
completed shards through a directory-anchored transaction. Library users call
the full-identity `ck_profile_flush_*` control symbol only after quiescence.
`native/dispatch_runtime/` owns the baseline-safe detector and process-local
acquire-release publication. Public ABI thunks remain stable; baseline,
variant, and runtime implementations remain hidden named-object members.

`src/tune/` owns explicit offline Auto-Tuning. It parses a closed workload,
captures runner/input bytes without indirection, enumerates canonical choices
from immutable pre-tune KIR, performs bounded search and correctness-first
measurement, then independently replays the selected plan. `CKTUNE01` schema 1
binds compiler/source/mode/target/frontier/plan/object/link identity. Publication
uses persistent destination locks and a recoverable journal; ordinary commands
do not enter tuning or execute the runner.

`src/cli/` owns parsing, dispatch, transactional output, `emit-kir` evidence,
contract-sanitizer selection, isolated run/cache policy, and diagnostics. Cache
and code-generation identity includes KIR v3, consumer, modes, contracts, ABI,
LLVM, target-profile digest, CPU features, optimizer proof/cost schemas, budgets,
and sanitizer configuration. Native entries use `CKCOBJ04` key schema 5 and
manifest schema 5. A multiversion cache hit requires the closed ordered bundle,
every named object, target set, dispatcher/runtime, profile, and physical
artifact identity; generation bypasses cache.

Any malformed KIR, stale proof/fact, invalid effect order, or backend fact-audit
failure stops compilation before artifact commit. The compiler does not fall
back to an unverified MIR path.
