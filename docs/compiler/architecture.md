# CalcKernel 0.11 Compiler Architecture

[简体中文](../zh-CN/compiler/architecture.md)

Public behavior is defined by the language, CLI, MIR, compatibility, and ABI
documents. Module names in this document explain implementation ownership.

```text
.ck -> source/diagnostics -> lexer -> parser/AST -> type checker/contracts
    -> semantic MIR lowering/validation
    -> consumer reachability + mode-specific KIR construction
    -> KIR verifier -> fact-driven KIR optimizer -> KIR verifier
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

`src/ir/kir/` builds consumer- and mode-specific KIR. KIR contains scalar SSA,
block parameters, region Memory SSA, explicit guards and ordered effects. The
builder first prunes unreachable code for the selected artifact and rejects an
unsupported runtime capability. Library roots are exports, executable roots
are `main`, and inspection roots are their union.

`src/optimizer/` owns scalar/path, natural-loop, alias/region, Memory SSA, and
interprocedural effect analysis; fact and proof arenas; pass management; and the
independent evidence verifier. Facts distinguish proven analysis from trusted
contract instances. Each unsafe call gets a separately scoped instance. A pass
cannot remove a guard or emit a backend fact unless a closed certificate or
auditable contract fact remains valid in the current CFG and Memory SSA state.

`src/backend/` consumes verified KIR only. C and Native support the four
overflow/bounds combinations through explicit guards and status flow; WASM is
unchecked-only. C contract facts may become portable restrict/alignment hints.
Native lowers the same facts to LLVM attributes/metadata, validates them with a
pre-LLVM fact audit, builds structural IR through the owned bridge, verifies it,
and emits object bytes with the host TargetMachine.

`src/backend/llvm/` and `native/bridge/` provide typed ownership across the
Rust/C++ boundary. `native/runtime/` owns entry, checked and sanitizer runtime
diagnostics, and print effects. LLD and ORC execute in process. The public
Native C ABI remains version 1; the private LLVM bridge and contract-aware
runtime ABI are version 2.

`src/cli/` owns parsing, dispatch, transactional output, `emit-kir` evidence,
contract-sanitizer selection, isolated run/cache policy, and diagnostics. Cache
and code-generation identity includes KIR v1, consumer, modes, contracts, ABI,
LLVM, target, CPU features, and sanitizer configuration.

Any malformed KIR, stale proof/fact, invalid effect order, or backend fact-audit
failure stops compilation before artifact commit. The compiler does not fall
back to an unverified MIR path.
