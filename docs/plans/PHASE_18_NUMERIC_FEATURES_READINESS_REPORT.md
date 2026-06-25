# Phase 18 Numeric Features Readiness Report

Date: 2026-06-25

Scope: analysis only. This report evaluates next numeric work for IK /
IntKernel after Phase 16 f64 strict mode and Phase 17 hardening. It does not
implement features, change compiler behavior, update tests, update snapshots,
or change package metadata.

## 1. Current f64 Strict Mode Status

IK / IntKernel currently supports:

- `f64` primitive type.
- Float literals with stable source-text preservation in AST and MIR.
- f64 arithmetic: `+`, `-`, `*`, `/`.
- unary `-f64`.
- f64 comparisons: `==`, `!=`, `<`, `<=`, `>`, `>=`.
- `ptr<f64>`.
- struct fields containing `f64`.
- MIR `f64` primitive and `const_float`.
- C backend mapping `f64` to `double`.
- LLVM backend mapping `f64` to `double`, using `fadd`, `fsub`, `fmul`,
  `fdiv`, `fneg`, `fcmp`, `load double`, and `store double`.
- WASM backend mapping `f64` to WASM `f64`, using `f64.*`, `f64.load`, and
  `f64.store`.
- JavaScript WASM interop using `Number` for f64.
- f64 benchmarks for axpy, dot product, sum, and scale.
- f64 strict optimizer guard: no fast-math, no f64 constant folding, no f64
  reassociation, no f64 operand sorting, no unsafe f64 LICM, and no f64
  induction simplification.

Current documented limits:

- no `f32`
- no implicit int/float conversion
- no explicit numeric casts
- no `f64 %`
- no fast-math
- no SIMD
- no runtime
- no standard library
- no math intrinsics
- no NaN or infinity literal syntax
- no float suffix literal syntax
- no cross-backend bit-identical floating point guarantee
- f64 does not participate in checked integer overflow

Important release note: Phase 17.4 found a package-bin fresh-install issue:
installed `node_modules/.bin/ikc` can no-op when invoked through the npm
symlink. That is not a numeric feature, but it should be fixed before starting
large Phase 18 implementation work.

## 2. Current Numeric Language Gaps

The biggest numeric gaps after f64 strict mode are:

1. No way to intentionally convert integers to f64.
2. No way to intentionally convert f64 to integer.
3. No standard math operations such as `sqrt` or `abs`.
4. No f32 for memory-bandwidth-focused kernels.
5. No unsafe/fast floating point mode for users who intentionally give up NaN,
   infinity, and signed-zero guarantees.
6. No SIMD or vector types.
7. WASM total benchmark time is still strongly affected by host memory
   marshaling rather than generated compute alone.

The gaps have very different risk profiles. Some are language-semantics work;
some are backend codegen work; some are host/example/benchmark work.

## 3. Explicit Numeric Casts

### Candidate API

Possible function-style builtins:

- `to_f64(i32) -> f64`
- `to_f64(u32) -> f64`
- `to_f64(i64) -> f64`
- `to_f64(u64) -> f64`
- `to_i32(f64) -> i32`
- `to_i64(f64) -> i64`
- `to_u32(f64) -> u32`
- `to_u64(f64) -> u64`

These should be explicit builtins, not implicit conversions. They should not
make integer literals materialize to f64 automatically.

### Integer to f64

Readiness: high.

Backend mapping:

| Source | C | LLVM | WASM |
| --- | --- | --- | --- |
| `i32 -> f64` | `(double)x` | `sitofp i32 to double` | `f64.convert_i32_s` |
| `u32 -> f64` | `(double)x` | `uitofp i32 to double` | `f64.convert_i32_u` |
| `i64 -> f64` | `(double)x` | `sitofp i64 to double` | `f64.convert_i64_s` |
| `u64 -> f64` | `(double)x` | `uitofp i64 to double` | `f64.convert_i64_u` |

Semantic notes:

- `i32` and `u32` values are exactly representable in f64.
- `i64` and `u64` values above `2^53` may round because f64 has 53 bits of
  integer precision.
- This rounding must be documented and tested.
- Checked mode should not reject integer-to-f64 rounding; it is a representable
  f64 conversion, not integer overflow.

Implementation difficulty: low to medium.

Required compiler shape:

- represent builtins in parser/checker as call targets or as dedicated AST/MIR
  cast instructions
- prefer a dedicated MIR `cast` instruction over lowering to an ordinary
  function call, because backends need target-specific instructions
- add MIR validator rules for allowed cast pairs
- add C/WASM/LLVM emission
- add cross-backend e2e tolerance tests

### f64 to integer

Readiness: low without a design decision.

The hard part is out-of-range, NaN, and infinity behavior:

- C conversion from NaN or out-of-range f64 to integer is not a safe portable
  contract.
- LLVM `fptosi` / `fptoui` has poison/undefined-like risks for invalid inputs.
- WASM normal truncation instructions trap on NaN or out-of-range values.
- WASM saturating truncation instructions exist, but saturating semantics would
  not match C or LLVM without explicit codegen.

Possible strategies:

| Strategy | Meaning | Risk |
| --- | --- | --- |
| unchecked precondition | user must ensure finite in-range input | fastest but unsafe and hard to test uniformly |
| saturating cast | clamp to integer min/max, NaN to 0 or reject | portable but semantically surprising |
| checked cast | invalid input returns checked error | only fits checked C today; WASM/LLVM checked modes are unsupported |
| trapping cast | invalid input traps or aborts | conflicts with no runtime and backend differences |

Recommendation: do not implement f64-to-integer casts in the first explicit-cast
slice. First write a cast semantics design. If implemented later, prefer a
separate explicit checked conversion design rather than silently relying on C or
LLVM invalid conversion behavior.

### Bool Casts

Recommendation: do not allow bool numeric casts in Phase 18.

Reasons:

- `bool -> i32` looks simple, but it opens the question of numeric truthiness
  and `i32 -> bool`.
- IK / IntKernel currently keeps bool separate from numeric types.
- Numeric casts should not weaken this boundary.

### Checked Mode Interaction

Recommended first-slice behavior:

- integer-to-f64 casts are allowed in checked C mode and cannot return
  `IK_ERR_OVERFLOW`
- f64-to-integer casts are not implemented until invalid-input semantics are
  designed
- no new checked error code should be added in the first slice

### Explicit Casts Verdict

Explicit casts are a good Phase 18 candidate only if staged:

1. Phase 18 design report for all cast semantics.
2. Implement integer-to-f64 only.
3. Defer f64-to-integer until a checked/trapping/saturating policy is chosen.

Full bidirectional casts are too risky as a single Phase 18 feature.

## 4. Minimal Math Intrinsics

### Candidate Set

Reasonable first candidates:

- `abs`
- `sqrt`
- `floor`
- `ceil`
- `min`
- `max`

Candidates to defer:

- `round`
- `sin`
- `cos`
- `exp`
- `log`

### Portability

| Intrinsic | C | LLVM | WASM | Risk |
| --- | --- | --- | --- | --- |
| `abs(f64)` | `fabs` or builtin | `llvm.fabs.*` | `f64.abs` | low |
| `sqrt(f64)` | `sqrt` or builtin | `llvm.sqrt.*` or call | `f64.sqrt` | medium due C libm/linking |
| `floor(f64)` | `floor` or builtin | intrinsic or call | `f64.floor` | medium due C libm/linking |
| `ceil(f64)` | `ceil` or builtin | intrinsic or call | `f64.ceil` | medium due C libm/linking |
| `min/max(f64)` | `fmin`/`fmax` | tricky intrinsic choice | `f64.min`/`f64.max` | medium-high due NaN and signed zero |
| `round(f64)` | `round`/`nearbyint` variants | multiple choices | `f64.nearest` | high due tie behavior |

### Runtime and libm Risk

IK / IntKernel currently has no runtime and generated C is self-contained for
ordinary arithmetic. Math intrinsics can change that:

- C `sqrt`, `floor`, `ceil`, `sin`, `cos`, `exp`, and `log` may require libm on
  some platforms.
- `build` may need to link `-lm` on Linux.
- Host users compiling emitted C manually would need documented link flags.
- LLVM can use intrinsics for some operations, but final native linking still
  depends on target lowering.
- WASM supports several core f64 math instructions, but not all libm-style
  functions.

### Fit With IK Positioning

Minimal intrinsics fit IK if they are deterministic, scalar, and kernel
oriented. `abs` and `sqrt` are plausible. Trigonometric and transcendental
functions are too broad for the next phase because they imply a standard math
library surface and cross-platform accuracy questions.

### Math Intrinsics Verdict

Do not make math intrinsics the first Phase 18 implementation. Prepare a
separate design after cast and WASM marshal decisions. If a small slice is
needed, start with `abs(f64)` only because it maps cleanly to C/LLVM/WASM and
does not require libm on WASM.

## 5. f32 Readiness

### Required Language Decisions

Adding `f32` is more than adding another primitive:

- Need a way to write f32 literals or explicitly convert f64 literals to f32.
- If all float literals remain f64, `let x: f32 = 1.0` would currently be a
  forbidden implicit conversion.
- A suffix such as `1.0f32` is currently explicitly unsupported.
- Mixed `f32`/`f64` arithmetic should be rejected unless explicit casts exist.

### ABI and Layout

Expected mapping:

| Backend | f32 mapping |
| --- | --- |
| C | `float` |
| LLVM | `float` |
| WASM | `f32` |
| layout | size 4, align 4 |
| JS interop | still `Number` |

The JS interop mismatch matters: WebAssembly exposes f32 parameters and returns
as JavaScript `Number`, but values are rounded to f32 at the WASM boundary.
Tests must classify this as f32 precision behavior, not as f64 behavior.

### Benchmark Meaning

f32 could matter for memory bandwidth and WASM SIMD later, but the current
benchmark suite is scalar f64 and integer pricing. Without vectorization or
large memory-bandwidth kernels, f32 may add complexity before it shows value.

### f32 Verdict

Defer. f32 should come after explicit casts and possibly after a math/benchmark
design. It increases type-system, literal, ABI, layout, backend, and JS interop
surface area.

## 6. Fast-Math Readiness

### Flag Shape

Fast-math must not be tied automatically to `-O3`. It should be a separate
explicit flag if it ever exists, for example:

```sh
ikc emit-llvm kernel.ik --opt-level 3 --float-mode fast
```

Default must remain strict.

### Semantic Changes

Fast-math can invalidate current Phase 17 semantics:

- NaN behavior may be ignored.
- Infinity behavior may be ignored.
- `-0.0` may be treated as `0.0`.
- Reassociation may change finite results.
- Comparisons may assume no NaN.
- `x * 0.0`, `x / x`, and `x + 0.0` transformations become possible only under
  explicitly weakened semantics.

### Backend Risks

LLVM fast-math flags are powerful but easy to over-apply. C compiler flags such
as `-ffast-math` affect more than local expressions and can change library and
errno assumptions. WASM does not have a directly equivalent high-level
fast-math mode across all engines and hosts.

### Fast-Math Verdict

Long-term defer. Fast-math should not be Phase 18. It requires a separate
semantic mode, clear user opt-in, backend-specific lowering rules, and a
parallel test matrix proving strict mode remains the default.

## 7. SIMD / Vectorization Readiness

### Design Options

Possible directions:

1. Do nothing in the language; rely on C/LLVM auto-vectorization from scalar
   pointer loops.
2. Add vector types such as `vec<f64, 2>` or `f64x2`.
3. Add backend-specific SIMD intrinsics.
4. Add WASM SIMD-specific lowering.

### Current Readiness

The current language has pointer loops, numeric scalars, and O3 backend paths.
That is enough for C/LLVM compilers to attempt auto-vectorization on some
kernels. It is not enough to guarantee vectorization.

Adding vector types would require:

- type system changes
- parser syntax
- MIR vector types and operations
- ABI decisions
- C, LLVM, and WASM backend divergence handling
- new benchmarks with alignment and memory stride controls

WASM SIMD adds additional risk:

- browser and Node engine support variation
- different instruction set from native SIMD
- more complex feature detection
- more complex package and example docs

### SIMD Verdict

Do not implement SIMD in Phase 18. First write a dedicated vectorization report
after the scalar f64 and benchmark story is stable. In the meantime, keep
scalar loops friendly to C/LLVM auto-vectorization without changing IK
semantics.

## 8. WASM Memory Marshal Optimization

### Current Situation

Current docs and benchmark code already separate:

- total time: host memory setup + WASM compute + checksum read
- compute-only time: prewritten memory + repeated WASM calls
- memory-only time: host-side memory work
- call-overhead time

The current f64 WASM benchmark uses `DataView.setFloat64` and
`DataView.getFloat64` for explicit little-endian access. This is semantically
clear and portable, but it can be slower than typed-array bulk access.

### Candidate Optimizations

1. Use `Float64Array` views over `instance.exports.memory.buffer` for f64
   input/output buffers.
2. Use `BigInt64Array` / `BigUint64Array` views where appropriate for integer
   benchmark paths, while preserving exact i64/u64 semantics.
3. Use zero-copy input/output patterns where host code writes directly into
   WASM memory views.
4. Reuse memory regions and typed-array views in compute-only loops.
5. Recreate typed-array views after `memory.grow`, because growing memory can
   detach the old buffer.
6. Add examples that show offset-to-index conversion:
   `float64Index = byteOffset / 8`.

### Why This Is Attractive

This direction has high value and low language risk:

- no language syntax changes
- no type-system changes
- no MIR changes
- no backend semantic changes
- directly targets known benchmark bottlenecks
- improves examples and host API guidance
- preserves strict f64 semantics

### Risks

- Typed arrays use platform endianness, while WebAssembly memory is
  little-endian. On mainstream release targets this is little-endian, but docs
  should keep `DataView` as the most explicit portable reference.
- Typed-array views require alignment. `ptr<f64>` offsets should be 8-byte
  aligned.
- `memory.grow` invalidates old views.
- For integer pricing paths, `Number` typed arrays are not equivalent to exact
  `i64` semantics. Do not replace BigInt paths with Number paths.

### WASM Marshal Verdict

This is the strongest Phase 18 first step. It improves real usability and
benchmark clarity without changing IK language semantics.

## 9. Risk Assessment

Low risk:

- WASM f64 host examples using `Float64Array` views over memory.
- Benchmark runner support for typed-array memory-only variants.
- Additional docs for host memory patterns.
- `to_f64(i32)` and `to_f64(u32)` once cast representation is designed.

Medium risk:

- `to_f64(i64)` and `to_f64(u64)` because large values round.
- `abs(f64)` as a minimal intrinsic.
- `sqrt(f64)` if C build/link behavior is fully specified.
- f32 layout and backend support without mixed arithmetic.

High risk:

- f64-to-integer casts without invalid-input semantics.
- `min` / `max` without NaN and `-0.0` semantics locked.
- `round` because tie behavior differs across possible definitions.
- sin/cos/exp/log because they imply libm/runtime/accuracy policy.
- fast-math.
- SIMD/vector types.

## 10. Recommended Phase 18

### First Recommendation: WASM Memory Marshal Optimization

Reason:

- It addresses a known bottleneck in total WASM benchmarks.
- It does not change language semantics.
- It does not weaken strict f64.
- It improves examples and host API usability.
- It can be tested independently from compiler correctness.

Expected engineering effort: small to medium.

Backend impact: none or minimal. This should mostly touch examples,
benchmarks, docs, and possibly helper code.

Testing needs:

- benchmark runner smoke for typed-array f64 memory path
- correctness checks with tolerance
- memory grow/view invalidation regression
- pricing integer path regression to ensure i64/u64 remain exact
- docs review for host API examples

### Second Recommendation: Explicit Casts, Staged

Reason:

- Explicit casts are a natural next numeric language feature.
- They unblock common f64 kernels that need integer loop values in formulas.
- Integer-to-f64 casts are portable and relatively easy.

Scope recommendation:

1. Phase 18 cast design doc.
2. Implement `to_f64(i32/u32/i64/u64)` only.
3. Defer f64-to-integer casts until invalid-input behavior is designed.

Expected engineering effort: medium.

Backend impact: all three backends plus MIR.

Testing needs:

- type checker accepts only explicit casts
- mixed arithmetic remains rejected
- MIR cast validator
- C/WASM/LLVM cast snapshots
- cross-backend e2e for exact i32/u32 and rounded i64/u64 cases
- checked C regression

### Defer

- Minimal math intrinsics beyond a possible `abs(f64)` design.
- f32.
- f64-to-integer casts.
- `round`.
- sin/cos/exp/log.
- SIMD/vectorization.

### Not Recommended Now

- fast-math
- implicit conversions
- broad standard library
- runtime-dependent math library
- NaN/Infinity literal syntax
- vector types

## 11. Not Recommended Now

Do not implement these in the next phase:

- implicit numeric conversion
- full bidirectional casts in one phase
- bool numeric casts
- f64-to-integer casts with backend-defined invalid behavior
- fast-math as part of `-O3`
- SIMD or vector syntax
- f32 without a literal/cast design
- sin/cos/exp/log
- runtime or standard library
- NaN/Infinity literal syntax

## 12. Suggested Phase Breakdown

Recommended Phase 18 path:

1. Phase 18.0: fix the package fresh-install `ikc` bin blocker from Phase 17.4.
2. Phase 18.1: WASM memory marshal design note and typed-array host API rules.
3. Phase 18.2: f64 WASM benchmark typed-array memory path and examples.
4. Phase 18.3: pricing WASM marshal audit, preserving exact i64/u64 behavior.
5. Phase 18.4: explicit cast semantics design.
6. Phase 18.5: implement integer-to-f64 casts only, if the design is accepted.
7. Phase 18.6: docs/release checklist update.
8. Phase 18.7: Phase 18 final validation report.

Alternative if the priority is language surface rather than benchmark/user-host
experience:

1. cast semantics design
2. MIR cast representation
3. `to_f64` only
4. C backend
5. WASM backend
6. LLVM backend
7. cross-backend cast matrix

## 13. Test Plan Draft

For WASM memory marshal optimization:

- f64 typed-array memory writer smoke
- f64 typed-array output reader smoke
- DataView and typed-array results match for aligned f64 buffers
- memory grow invalidates/recreates views correctly
- f64 axpy/dot/sum/scale benchmark smoke still passes
- pricing i64/u64 path still uses BigInt where required
- no benchmark thresholds added to ordinary `pnpm test`

For integer-to-f64 casts:

- lexer/parser rejects no new implicit syntax
- checker accepts `to_f64(i32/u32/i64/u64)`
- checker rejects `to_f64(f64)` unless identity casts are explicitly allowed
- checker rejects mixed arithmetic without an explicit cast
- checker rejects bool casts
- MIR lower emits dedicated cast
- MIR validator rejects unsupported cast pairs
- C snapshots and e2e
- WAT snapshots and WASM e2e
- LLVM snapshots and e2e
- i64/u64 large-value rounding documented and tested with tolerance
- checked C regression confirms casts do not change integer overflow checks

For future math intrinsics:

- per-intrinsic type checker tests
- backend snapshots
- NaN/infinity/-0.0 tests
- libm/link behavior tests for C build
- WASM instruction support tests
- no runtime dependency unless explicitly approved

For f32 if revisited:

- literal/cast policy tests
- layout size 4 align 4
- C `float`, LLVM `float`, WASM `f32`
- JS Number boundary tests
- mixed f32/f64 rejection tests
- f32 benchmark tolerance policy

## 14. Docs Update Draft

If Phase 18 starts with WASM memory marshal optimization, update:

- `docs/WASM_ABI.md`: typed-array view pattern, alignment, `memory.grow`
  invalidation, and DataView as portable reference.
- `docs/PERFORMANCE.md`: total vs compute-only vs typed-array memory-only
  interpretation.
- `bench/README.md`: new case names and warning against cross-machine
  conclusions.
- README examples if a host API example changes.

If Phase 18 starts explicit casts, update:

- `docs/LANGUAGE_SPEC.md`: builtin cast syntax and strict no-implicit-conversion
  rule.
- `docs/MIR.md`: cast instruction.
- `docs/ABI.md`: backend cast semantics.
- `docs/CHECKED_ARITHMETIC.md`: checked-mode interaction.
- `docs/OPTIMIZATION.md`: casts are not algebraic simplification permission.
- `docs/PERFORMANCE.md`: if new benchmarks use casts.

## 15. Final Recommendation

Phase 18 should start with WASM memory marshal optimization, after fixing the
package fresh-install bin blocker. This gives the best risk-adjusted return:
high practical value, low language risk, no ABI change, and no strict-float
semantic weakening.

The second Phase 18 direction should be explicit numeric casts, but only as a
staged feature. Implement integer-to-f64 first. Do not implement f64-to-integer
casts until NaN, infinity, out-of-range, checked-mode, and backend behavior are
specified.

Fast-math and SIMD should remain out of scope for Phase 18.
