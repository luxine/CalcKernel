# IntKernel Performance Profile

Date: 2026-06-24

This report profiles the current pricing benchmark before Phase 14 optimization
work. It records what is slow and why, without changing compiler optimization
logic.

## Benchmark Context

Latest full run:

- Items: 100000
- Iterations per command: 1000
- Hyperfine runs: 20
- Warmup runs: 3
- Platform: macOS arm64
- CPU: Apple M4 Pro
- Node: v24.14.0
- Clang: Apple clang 17.0.0

| Case | Category | Opt | Mode | Median ms | vs C O3 |
| --- | --- | --- | --- | ---: | ---: |
| pricing-c-unchecked-O0 | native | O0 | unchecked | 621.854 | 10.77x |
| pricing-c-unchecked-O2 | native | O2 | unchecked | 58.140 | 1.01x |
| pricing-c-unchecked-O3 | native | O3 | unchecked | 57.754 | 1.00x |
| pricing-c-checked-O3 | native | O3 | checked | 80.968 | 1.40x |
| pricing-llvm-unchecked-O0 | native | O0 | unchecked | 615.368 | 10.66x |
| pricing-llvm-unchecked-O3 | native | O3 | unchecked | 57.779 | 1.00x |
| pricing-wasm-unchecked-total | wasm | n/a | unchecked | 2699.638 | 46.74x |
| pricing-wasm-unchecked-compute-only | wasm | n/a | unchecked | 220.336 | 3.82x |
| pricing-wasm-unchecked-memory-only | memory | n/a | unchecked | 4754.650 | 82.33x |
| pricing-wasm-unchecked-call-overhead | call-overhead | n/a | unchecked | 24.090 | 0.42x |
| pricing-js-number | js | n/a | host | 263.457 | 4.56x |
| pricing-js-typedarray-number | js | n/a | host | 123.421 | 2.14x |
| pricing-js-bigint | js | n/a | host | 182.220 | 3.16x |

## Lightweight Profiling Evidence

The `time` tool is available on this machine. It is too coarse for the fastest
native command at the quick scale, but it confirms the broad shape:

- `pricing-c-unchecked-O3`, 100000 items x 100 iterations: below visible
  `time -p` resolution.
- `pricing-wasm-unchecked-compute-only`, same work size: about 0.05 seconds.
- `pricing-wasm-unchecked-memory-only`, 100000 items x 10 iterations: about
  0.12 seconds.
- `pricing-js-bigint`, 100000 items x 100 iterations: about 0.04 seconds.

`sample` is present at `/usr/bin/sample`, and Instruments can be used manually
for deeper native or Node profiling. They are optional and are not required by
the benchmark runner.

For LLVM, clang was used to inspect optimized IR:

```sh
clang -O3 -S -emit-llvm build/perf/generated/pricing.ll \
  -o build/perf/generated/pricing.optimized.ll
```

The unoptimized generated LLVM IR has 22 `alloca`, 36 `load`, and 24 `store`
operations. After clang `-O3`, the hot function has no remaining allocas, four
loads from `items`, one store to `out`, and a tight loop with a phi induction
variable. This confirms that clang can clean up the current alloca/load/store
LLVM lowering for the pricing kernel.

## Why C Unchecked Is Fastest

The unchecked C path is the best current native hot path:

- It emits direct integer arithmetic and direct struct field loads/stores.
- Clang `-O2` and `-O3` both reduce the MIR-style temps and gotos to a compact
  loop.
- It has no status propagation, overflow branches, or host boundary cost.
- It keeps the whole batch inside one native call.

`O2` and `O3` are effectively tied for this benchmark. `O0` is roughly 10.8x
slower, which shows that the raw generated C is correct but relies heavily on
the C compiler optimizer for hot-path performance.

## Where C Checked Spends Time

Checked C is about 1.40x the unchecked C O3 median in the full run.

The generated checked loop performs the useful pricing arithmetic plus:

- two `__builtin_mul_overflow` checks
- one `__builtin_sub_overflow` check
- two `__builtin_add_overflow` checks, including `i + 1`
- a division-by-zero branch for the constant divisor `1000000`
- a signed min divided by `-1` overflow branch
- early-return branches for every checked operation
- the checked ABI return pointer check and final `ck_return` write

Some checks are semantically required. Some are provably redundant in this
specific lowered code, such as divisor zero checks for literal `1000000`, but
they must only be removed by a general, correctness-tested pass.

## Where WASM Spends Time

WASM is currently slow in two separate layers:

1. **Host memory marshaling** is very expensive when measured in the hot loop.
   `pricing-wasm-unchecked-memory-only` is much slower than compute-only because
   it repeatedly writes `Item` fields and reads checksums through `DataView`.
2. **WASM compute-only is still slow** compared with C and JS BigInt. The WAT
   uses a block dispatcher:

   - an `ik_bb` local
   - a `br_table` dispatcher loop
   - nested block cases
   - repeated `local.get` / `local.set`

   Each source loop iteration returns to the dispatcher instead of using a
   structured WebAssembly loop. The body also recomputes addresses such as
   `items + i * 32` for each field load.

The tiny call-overhead case is faster than the pricing cases, so JS-to-WASM
boundary cost is not the main explanation for pricing being slower than JS
BigInt. It can still matter for fine-grained APIs, but this benchmark batches
work.

## Where JS BigInt Spends Time

The JS BigInt baseline is exact for `i64`-style arithmetic, but it pays for:

- BigInt multiplication, subtraction, addition, and division.
- `BigInt64Array` element conversion.
- BigInt checksum accumulation.

It is slower than the Number typed-array case, but faster than current WASM
compute-only. That suggests the current WASM code shape, not merely i64
arithmetic, is a major bottleneck.

The plain Number array case is slower than the typed-array Number case, which
shows that JS data layout materially affects this benchmark.

## Benchmark Harness Overhead

Harness overhead is intentionally visible in the decomposed cases:

- `total` includes host memory write, compute, and checksum read.
- `compute-only` prewrites memory and measures repeated `calc_items` calls.
- `memory-only` isolates host `DataView` memory work.
- `call-overhead` isolates a tiny JS-to-WASM call loop.

The original `pricing-wasm-unchecked` style mixed compute and host memory work.
For backend optimization, `compute-only` is the most useful WASM signal. For
host integration guidance, `total` and `memory-only` are still important.

## Generated C Review

Current unchecked generated C contains many temporary variables and goto labels:

- `ik_tmp0` through `ik_tmp14`
- explicit `goto bb1`, `bb2`, `bb3`
- temporary assignments for literals such as `0`, `1`, and `1000000`

Clang `-O3` handles this well. Still, MIR-level cleanup would make all C build
modes easier to inspect and would reduce dependence on the C optimizer at
lower optimization levels.

Potential improvements:

- eliminate single-use move temps
- fold literal temps into their use sites
- keep loop induction updates simple
- compute repeated pointer/index addresses once per field group when safe
- eventually expose aliasing information only if the language gains a sound
  contract for it

Do not add `restrict` by default without a language-level non-aliasing rule.

## Generated Checked C Review

Checked mode correctly preserves safety, but the pricing loop contains
checks that a general optimizer could prove redundant:

- divisor literal `1000000` cannot be zero
- divisor literal `1000000` cannot be `-1`
- return literal `0` does not need arithmetic checks

The `i + 1` overflow check is semantically valid. Removing it would require a
range analysis proving `i < len` and `len <= INT32_MAX`, and that analysis is
not present yet.

## Generated WAT Review

The WAT shape is the largest backend-specific issue:

- multi-block functions use a block dispatcher rather than structured loops
- every loop iteration updates `ik_bb` and goes through `br_table`
- the body uses many `local.get` / `local.set` pairs
- `items + i * 32` is recomputed for price, qty, discount, and tax rate
- host-side `DataView` reads/writes dominate total/memory-only runs

The highest-impact WASM optimization is replacing dispatcher output for
structured patterns with native WebAssembly `loop` / `block` control flow.

## Generated LLVM Review

LLVM v1 emits alloca/load/store form, which is simple and correct but verbose.
For pricing, clang `-O3` successfully removes the alloca-heavy shape and emits
a tight loop. This is why LLVM O3 and C O3 are essentially tied.

Potential future LLVM improvements:

- add direct SSA-like lowering for simple expressions and loops
- emit alignment information for loads/stores and GEP-derived pointers
- keep alloca lowering at `-O0` for debugability

Because clang already optimizes the pricing kernel well, LLVM backend
optimization is not the Phase 14 first target.

## Optimization Priority

| Priority | Item | Expected Impact | Notes |
| --- | --- | --- | --- |
| P0 | Add MIR pass manager and opt-level plumbing | Required foundation | Every later optimization needs `-O0` off switch and validator integration. |
| P0 | Split backend/perf signals in docs and runner | Already started | Avoid optimizing for a mixed host-memory benchmark by mistake. |
| P1 | Structured WASM loop/control-flow emission | High | Directly targets compute-only WASM being about 3.8x C O3. |
| P1 | MIR temp/move/literal cleanup | High | Helps C O0, WAT, LLVM O0, readability, and future passes. |
| P1 | Common address calculation cleanup | High | Pricing reloads and recomputes `items + i * sizeof(Item)` repeatedly. |
| P1 | Checked constant divisor simplification | Medium/high | Removes redundant div-zero and min/-1 checks for literal divisors. |
| P2 | Checked arithmetic range analysis | Medium | Could remove loop-index overflow checks, but needs proof infrastructure. |
| P2 | LLVM direct SSA-like lowering | Medium | Useful long term; clang already recovers pricing O3 today. |
| P2 | Emit alignment metadata in LLVM | Medium | Current optimized IR uses conservative alignment for i64 loads/stores. |
| P3 | Host memory marshaling redesign | Deferred | Requires API/usage guidance or future memory helpers, not compiler-only. |
| P3 | `restrict` or noalias assumptions | Deferred | Needs a sound language-level aliasing contract. |
| P3 | SIMD/threading/PGO | Out of scope | Explicitly excluded from Phase 14. |

## Recommended Next Step

Implement Phase 14.4 as infrastructure, not an optimization:

1. Add `OptimizationLevel` and CLI plumbing.
2. Add a MIR pass manager with no-op `O0`.
3. Run the MIR validator after each pass.
4. Add pass-level tests and snapshots before changing backend output.

Then start with safe MIR cleanup passes and structured WASM loop emission.
