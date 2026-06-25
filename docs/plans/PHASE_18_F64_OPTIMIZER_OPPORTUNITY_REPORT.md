# Phase 18 f64 Optimizer Opportunity Report

Date: 2026-06-25

Status: analysis only. This report does not implement optimizer changes, does
not update snapshots, and does not change IK / IntKernel language semantics.

## Scope

This report evaluates whether the current MIR optimizer can improve strict
`f64` execution efficiency without changing the locked Phase 16/17 floating
point semantics.

Hard constraints:

- Keep strict `f64` semantics.
- Keep `ikc` command behavior unchanged.
- Keep `.ik` source compatibility unchanged.
- Do not enable fast-math.
- Do not add `f32`.
- Do not add implicit or explicit numeric conversion.
- Do not change C, LLVM, or WASM ABI.
- Do not change checked integer arithmetic semantics.
- Do not add benchmark thresholds to ordinary `pnpm test`.

## Current f64 Optimizer State

Current status is conservative and mostly correct for strict float:

- `f64` constant folding is disabled.
- `f64` reassociation is disabled.
- `f64` algebraic simplification is absent.
- `f64` operand sorting in local CSE is disabled.
- LICM refuses `f64` arithmetic and `const_float`.
- induction analysis only records integer constants and integer updates.
- copy propagation can rewrite `f64` values as a type-agnostic value rewrite.
- DCE can remove unused pure `f64` temporaries.
- small-function inlining can clone `const_float` and `f64` scalar instructions.
- address CSE works on places and can materialize `ptr<f64>` indexed places.

The current design is intentionally more conservative than native compiler
optimizers. That is appropriate for Phase 18 because strict float correctness is
more important than MIR-level micro-optimizations.

## Current Optimizer Pipeline

Source of truth: `src/opt/pipeline.ts`.

### O0

Pipeline:

```text
O0: <validator only>
```

Behavior:

- Runs no MIR optimization passes.
- Keeps lowered MIR as the review baseline.
- Best mode for debugging strict `f64` lowering and emitted MIR stability.

### O1

Pipeline:

```text
constant-folding -> copy-propagation -> dead-code-elimination -> cfg-simplify
```

Important f64 behavior:

- constant folding only handles integer and bool constants, not `f64`.
- copy propagation is type-agnostic and may rewrite `f64` temp uses.
- DCE may remove unused pure `f64` temp definitions.
- CFG simplify can remove unreachable blocks.

### O2

Pipeline:

```text
constant-folding -> copy-propagation -> inline-small-functions ->
constant-folding -> copy-propagation -> local-cse -> copy-propagation ->
address-cse -> dead-code-elimination -> cfg-simplify ->
dead-code-elimination
```

Important f64 behavior:

- inlining can clone small non-exported `f64` helper functions.
- local CSE currently skips all f64 binary, unary, and compare expressions.
- address CSE can optimize `ptr<f64>` place addressing for C/WASM targets.
- repeated cleanup passes can remove temps introduced by inlining/CSE.

### O3

Pipeline:

```text
constant-folding -> copy-propagation -> inline-small-functions ->
constant-folding -> copy-propagation -> loop-analysis ->
loop-invariant-code-motion -> induction-simplify -> constant-folding ->
copy-propagation -> local-cse -> copy-propagation -> address-cse ->
dead-code-elimination -> cfg-simplify -> dead-code-elimination
```

Important f64 behavior:

- larger inlining threshold than O2.
- loop analysis itself does not mutate MIR.
- LICM refuses `f64` arithmetic, `const_float`, loads, stores, calls, and
  divisions.
- induction simplify records integer induction metadata only and skips f64-like
  updates.
- C/LLVM native build commands may use clang `-O3`, but IK MIR remains strict.

## Pass-by-pass f64 Classification

Legend:

- A: already safe and useful for f64.
- B: can safely support f64, but needs tests before any expansion.
- C: can partially support f64 with strict limits.
- D: must continue to skip f64.
- E: not recommended for f64.

| Pass | Current f64 behavior | Class | Notes |
| --- | --- | --- | --- |
| constant folding | Does not remember `const_float`; `MirValue.const_float` resolves to unknown; only int/bool fold paths exist. | D | Keep disabled. Folding even obvious-looking f64 constants risks changing NaN, Infinity, signed zero, and target behavior. |
| copy propagation | Rewrites MIR value uses regardless of type; clears at `store` and `call`. | A | Already safe for f64 because it preserves evaluation order and does not evaluate or combine expressions. |
| DCE | Treats `const_float`, `binary`, `unary`, and `compare` as removable only when the target temp is unused; never removes loads/stores/calls/terminators. | A | Safe for unused pure f64 temps. Needs continued edge tests around branch conditions and returns. |
| CFG simplify | Removes unreachable blocks; at O2/O3 rewrites known bool branches and jump chains. | A | Type-neutral. f64 only matters if previous comparisons produce bool. It does not evaluate f64. |
| local CSE | Skips f64 binary, unary, and compare entirely; integer path sorts commutative operands. | C | Same-order f64 CSE could be considered, but no operand sorting and no reassociation. Current full skip is safest. |
| address CSE | Rewrites load/store/address places, including `ptr<f64>` indexed places; clears at `store` and `call`. | A | Safe for `ptr<f64>` addressing because it changes address calculation shape, not f64 values. It must keep store/call invalidation. |
| small-function inlining | Allows `const_float`, move, binary, unary, compare; rejects memory/calls inside candidates. | A | Safe when limited to single-block pure helpers. It clones evaluation in call position without algebraic assumptions. |
| loop analysis | Computes natural loop metadata and returns `changed: false`. | A | Analysis-only, type-neutral. Safe as long as consumers remain strict-aware. |
| LICM | Refuses `const_float` and any binary with f64 target/operand; also refuses loads/stores/calls/division. | D | Keep f64 hoisting disabled for now. Speculative f64 movement can alter division-by-zero timing and strict behavior. |
| induction simplify | Collects only `const_int`; recognizes only integer-style updates; returns `changed: false`. | D | Keep f64 skipped. f64 loop counters have NaN, signed zero, rounding, and non-monotonic edge behavior. |

## Explicitly Forbidden Optimizations

These must remain prohibited unless IK later adds an explicit non-strict float
mode:

- f64 constant folding.
- f64 reassociation.
- f64 algebraic simplification.
- `x * 0.0 -> 0.0`.
- `x / x -> 1.0`.
- `x + 0.0 -> x`.
- operand sorting for f64 `+`, `*`, `==`, or `!=`.
- speculative LICM hoist for f64 division.
- speculative LICM hoist for other f64 arithmetic.
- f64 induction simplification.
- LLVM fast-math flags.
- C fast-math compiler flags.
- target-specific fast-float modes.

Rationale:

- NaN can make identities false.
- Infinity can make identities false.
- `-0.0` can be observable through later operations.
- f64 division by zero is ordinary strict float behavior and must not be
  converted to checked integer division behavior.
- Backend differences are allowed within strict IEEE-like behavior; MIR should
  not add extra non-portable assumptions.

## Strict-safe Optimization Opportunities

### 1. Strengthen f64 copy propagation coverage

Current status: already implemented and tested for basic f64 temp rewrite.

Opportunity:

- Add broader tests for f64 copy propagation through compare operands, unary
  operands, branch conditions, return values, and `ptr<f64>` indexes.

Expected performance:

- Low to moderate for MIR cleanliness.
- More useful for WASM and MIR snapshots than for native C/LLVM, where clang can
  clean many trivial copies.

Risk:

- Low if the pass continues to clear at `store` and `call`.

Recommendation:

- Do tests first. No semantic change needed unless tests expose missed plumbing.

### 2. Strengthen f64 DCE coverage

Current status: DCE can remove unused pure f64 temp definitions.

Opportunity:

- Add explicit tests for unused `const_float`, f64 binary, unary, and compare.
- Add negative tests proving it keeps f64 branch conditions, return values, loads,
  stores, and calls.

Expected performance:

- Low to moderate. Reduces unused temp churn after inlining and CSE.

Risk:

- Low if pure/removable set stays unchanged.

Recommendation:

- Safe Phase 18.5 candidate as tests-only or small cleanup if gaps appear.

### 3. f64 CFG simplify tests

Current status: CFG simplify is type-neutral and only sees bool control flow.

Opportunity:

- Add tests where f64 comparisons feed branches and unreachable blocks are
  removed only when the bool is already a `const_bool`.
- Avoid folding f64 comparisons into bool constants.

Expected performance:

- Low. Mostly guard coverage.

Risk:

- Low.

Recommendation:

- Add tests if Phase 18.5 is a hardening phase.

### 4. f64 small-function inlining coverage

Current status: small-function inlining supports f64 scalar helpers.

Opportunity:

- Expand tests for f64 compare helpers and unary helpers.
- Add backend-neutral MIR tests showing inlining does not introduce f64
  reassociation, folding, or operand sorting.

Expected performance:

- Moderate when kernels use small helper functions.
- Useful for C/LLVM/WASM because it removes call overhead in generated IR.

Risk:

- Low to medium. Inlining duplicates expression evaluation at the call site, so
  candidates must remain pure and single-block. The current candidate filter is
  appropriately narrow.

Recommendation:

- Good Phase 18.5 candidate.

### 5. Same-order f64 local CSE

Current status: local CSE completely skips f64 value expressions.

Potential strict-safe variant:

- Only CSE expressions with exact same op, exact same type, and exact same
  left/right operand order.
- Do not sort operands.
- Do not CSE across `store` or `call`.
- Do not CSE loads.
- Do not CSE comparisons unless tests lock NaN comparison behavior.

Example that may be safe:

```ik
let a: f64 = x + y;
let b: f64 = x + y;
return a + b;
```

The second `x + y` can reuse the first only if both operands are the exact same
MIR values in the exact same order and no intervening effect can change them.

Expected performance:

- Low to moderate. It may reduce repeated expression work in hand-written kernels.
- C and LLVM backends may already optimize this after lowering.
- WASM compute-only may benefit more because the WAT backend is simpler.

Risk:

- Medium. Same-order CSE is much safer than commutative CSE, but it still changes
  the number of f64 operations executed. For strict languages this is usually
  acceptable for common subexpression elimination of pure expressions, but tests
  must cover NaN, Infinity, `-0.0`, and repeated division.

Recommendation:

- Do not implement first.
- If implemented, start with f64 add/sub/mul on already-evaluated operands and
  exclude division and comparisons until semantics are reviewed.
- Keep it behind focused tests, not a broad expression key reuse.

### 6. const_float plumbing cleanup

Current status: several passes already know `const_float` exists, but only to
skip or clone it.

Opportunity:

- Add tests that `const_float` is cloned by inlining, ignored by folding, seen by
  DCE, and not hoisted by LICM.
- Avoid changing representation or canonicalizing through JavaScript `Number`.

Expected performance:

- None directly.
- Helps prevent regressions before later optimizer work.

Risk:

- Low.

Recommendation:

- Good hardening item.

### 7. no-fast-math snapshot guard

Current status: LLVM tests already assert no `fast` and no fast-math flag tokens
for f64-sensitive expressions.

Opportunity:

- Keep tests backend-neutral where possible.
- Add one release checklist item requiring no-fast-math guard before float
  optimizer changes.

Expected performance:

- None.

Risk:

- Low.

Recommendation:

- Keep and extend only if new LLVM output paths are added.

## Backend Impact

### C Backend

Clang `-O3` can already optimize many low-level patterns after C emission:

- trivial copies;
- local temporaries;
- simple inlined helper bodies;
- address calculations;
- dead stores/unused temps when visible in C.

IK MIR can still contribute:

- smaller emitted C before clang sees it;
- lower C code volume after small helper inlining;
- better WASM parity where no clang cleanup exists;
- more predictable snapshots and debug output.

Avoid:

- adding `restrict` or noalias hints unless IK semantics explicitly guarantee no
  aliasing;
- adding fast-math flags to C compiler invocations;
- relying on C-specific behavior to justify MIR strict-float transforms.

Expected C performance gain from extra f64 MIR optimization is likely limited.
Clang is the main optimizer for C output.

### LLVM Backend

LLVM/clang `-O3` can optimize:

- trivial SSA-like arithmetic;
- local temp copies;
- simple helper bodies after inlining;
- address arithmetic when aliasing permits.

Constraints:

- Do not add fast-math flags.
- Do not change fcmp predicates.
- Do not fold f64 constants in MIR.
- Do not promise cross-backend bit-identical output.

LLVM backend already has a direct SSA-like lowering path for simple scalar
straight-line functions at O2/O3, including f64. Additional MIR f64 CSE may not
meaningfully improve LLVM native performance unless it reduces calls or exposes
simpler control flow.

### WASM Backend

WASM has two different bottleneck classes:

1. compute-only kernel execution;
2. JS host memory marshal / readback / setup cost.

Phase 18.1 and Phase 18.3 split these costs. For total f64 benchmark time, host
marshal path is often more important than MIR arithmetic optimization. The
Float64Array low-copy path is therefore a higher-impact area than aggressive
MIR f64 algebra.

MIR-level f64 optimization may still help WASM compute-only:

- copy propagation can reduce stack-like temp churn;
- DCE can remove unused pure work;
- small-function inlining can remove call overhead;
- same-order local CSE might help repeated scalar expressions.

But host-side changes can dominate total timing:

- DataView per-element write/read can be expensive.
- Float64Array views over WASM memory reduce bulk marshal cost.
- output readback strategy matters for array-producing kernels.

Recommendation: prioritize benchmark/interop path clarity before MIR f64 CSE.

## Recommended Implementation Order

### First: tests and hardening only

1. Add explicit f64 DCE tests:
   - unused `const_float`;
   - unused f64 binary/unary/compare;
   - negative tests for return/branch/load/store/call.

2. Add broader f64 copy propagation tests:
   - unary operand;
   - compare operands;
   - branch condition;
   - return value;
   - `ptr<f64>` index value.

3. Add small-function inlining f64 tests:
   - unary helper;
   - compare helper;
   - helper returning `const_float`;
   - no reassociation/folding after inlining.

4. Add no-fast-math guard to release checklist if not already present.

### Second: limited implementation if tests expose gaps

1. Fix any type-agnostic copy propagation gaps.
2. Fix any DCE coverage gaps for pure unused f64 temporaries.
3. Fix any inline cloning gaps for `const_float` or f64 compare/unary.

These are plumbing improvements, not new float algebra.

### Third: consider same-order local CSE

Only consider after tests are in place.

Allowed initial subset:

- exact same ordered operands;
- exact same op;
- exact same MIR type;
- no stores or calls between expressions;
- no loads;
- no operand sorting;
- likely start with `+`, `-`, `*`, unary neg only.

Defer:

- f64 division CSE;
- f64 comparison CSE;
- any cross-block CSE;
- any value numbering through memory.

### Defer

- f64 LICM.
- f64 induction simplification.
- f64 constant folding.
- f64 reassociation.
- f64 algebraic identity simplification.
- fast-math mode.
- alias/noalias annotations.

### Do Not Do

- `x * 0.0 -> 0.0`.
- `x / x -> 1.0`.
- `x + 0.0 -> x`.
- sorting operands of f64 `+`, `*`, `==`, or `!=`.
- hoisting f64 division out of loops.
- adding LLVM fast-math flags.
- adding C fast-math flags.

## Testing Plan

### Correctness tests

- MIR optimizer tests for O0/O1/O2/O3 f64 stability.
- f64 copy propagation tests across all operand positions.
- f64 DCE positive and negative tests.
- f64 small-function inlining tests for const, unary, binary, and compare.
- same-order local CSE tests only if that feature is implemented.

### NaN-sensitive tests

- `x * 0.0` remains multiplication.
- `x / x` remains division.
- `0.0 / 0.0` remains a runtime f64 operation.
- NaN comparisons are not folded.

### Signed-zero-sensitive tests

- `x + 0.0` remains addition.
- `0.0 + x` is not reordered.
- unary `-0.0` remains representable through existing syntax/lowering.

### Infinity-sensitive tests

- `1.0 / 0.0` remains division.
- overflow-to-Infinity patterns are not folded.
- Infinity arithmetic remains backend runtime behavior.

### Backend regression

- C f64 e2e.
- LLVM f64 e2e.
- WASM f64 e2e.
- cross-backend f64 behavior matrix.
- pricing integer regression.
- checked integer arithmetic regression.

### Benchmark smoke

- `node bench/perf/run.mjs --quick`
- Compare compute-only and total separately.
- Do not commit machine-local baseline.
- Do not add performance thresholds to ordinary `pnpm test`.

## Risk Assessment

### Low risk

- More f64 copy propagation tests.
- More f64 DCE tests.
- More f64 inlining tests.
- no-fast-math snapshot/checklist guard.
- const_float plumbing tests.

### Medium risk

- same-order f64 local CSE.
- broader f64 inlining if candidate shape expands beyond single-block pure
  helpers.
- any optimization that changes the number of f64 operations executed.

### High risk

- f64 constant folding.
- f64 reassociation.
- f64 algebraic identity simplification.
- f64 LICM.
- f64 induction simplification.
- fast-math.
- alias/noalias assumptions.

## Recommendation

Phase 18.5 should be a strict-safe optimizer hardening phase, not an aggressive
performance phase.

Recommended Phase 18.5 scope:

1. Add tests for f64 copy propagation, DCE, small-function inlining, and
   const_float plumbing.
2. Add or preserve no-fast-math guards.
3. Fix only type-agnostic plumbing gaps found by those tests.
4. Keep f64 constant folding, reassociation, operand sorting, LICM, induction
   simplification, and fast-math disabled.

Expected performance gains:

- Native C/LLVM: modest, because clang/LLVM already optimize many patterns.
- WASM compute-only: possible modest improvement from cleaner MIR and inlining.
- WASM total: larger gains are more likely from host memory marshal path
  improvements such as Float64Array over WASM memory.

The safest near-term value is regression prevention and preserving strict float
semantics while keeping the door open for carefully tested same-order local CSE.
