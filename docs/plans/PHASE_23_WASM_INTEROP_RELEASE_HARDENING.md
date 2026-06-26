# Phase 23: WASM Interop Release Hardening

This plan tracks release hardening after Phase 22 WASM-JS interop performance
optimization. It does not add CK / CalcKernel language features and does not
change pricing, f64, C backend, or LLVM backend semantics.

## Scope

- Stabilize the public boundary of `CKWasmArena`.
- Document that `CKWasmArena` is a JS/WASM interop helper, not a CK runtime.
- Make heap-base and memory ownership explicit for external consumers.
- Verify package exports, emitted declarations, and fresh-install import
  behavior.
- Provide official runnable examples for `f64-sum`, `f64-axpy`, and
  `pricing-soa`.
- Update release notes, release checklist, README, WASM interop docs, and
  performance docs with conservative performance claims.

## Completed Hardening Areas

| Area | Release-facing result |
| --- | --- |
| Public API | `CKWasmArena` and `createCKWasmArena` are package-root exports in `calckernel`. |
| API stability | Documented as experimental during the v0.8.x release-hardening window. |
| Heap base | `createCKWasmArena` resolves explicit `heapBase`, `__ck_heap_base`, then `__heap_base`; it does not silently guess. |
| WASM metadata | CK / CalcKernel WASM exports additive `__ck_heap_base` metadata without changing existing ABI behavior. |
| Package smoke | Fresh install covers `ckc`, JS import, TypeScript import, and WASM interop. |
| Examples | `examples/wasm/f64-sum`, `examples/wasm/f64-axpy`, and `examples/wasm/pricing-soa` are runnable correctness examples. |
| Docs | README, `docs/wasm-interop.md`, `docs/PERFORMANCE.md`, and release notes use the conservative WASM performance wording. |

## Release Claims

Allowed claims:

- CK WASM compute paths are competitive on the measured workloads.
- TypedArray views over WASM memory are the recommended interop shape for
  homogeneous buffers.
- Resident memory can reduce JavaScript-WASM boundary and marshaling cost.
- SoA layout is recommended for mixed-width batch data when host data can be
  reshaped.
- Output view is the recommended fast path when caller ownership allows it.
- `copyOutF64` is an explicit JS-owned copy path and can dominate total time.
- DataView fallback/debug paths can be much slower than JavaScript typed-array
  baselines.

Rejected release claims:

- blanket JavaScript comparisons based on local CK WASM wins
- broad claims against all JavaScript typed-array workloads
- claims that emitting WASM automatically improves workload speed
- claims that `DataView` is the high-throughput interop path

## Non-Goals

Phase 23 does not add f32, SIMD, fast-math, runtime allocation, GC, IO, strings,
implicit int/float conversion, additional casts, f64 math intrinsics, checked
WASM arithmetic, checked LLVM arithmetic, benchmark thresholds in `pnpm test`,
automatic commit, automatic tag, or npm publish.

## Validation Gates

- `pnpm typecheck`
- `pnpm build`
- `pnpm test`
- `npm pack --dry-run`
- real package fresh-install smoke with `npm pack`
- `ckc` CLI smoke
- JS import smoke for `CKWasmArena` and `createCKWasmArena`
- TypeScript import smoke against emitted declarations
- WASM interop consumer smoke
- package content check for no local benchmark outputs, local baselines,
  tarballs, temporary directories, `node_modules`, coverage, or cache files
- manual review of `docs/releases/v0.8.0.md`

## Remaining Release Risk

`CKWasmArena` is useful and covered by tests, but it is still a young public API.
Keep it documented as experimental before v1.0 and avoid expanding it into a CK
runtime. Future work should preserve the current separation: generated CK /
CalcKernel WASM owns computation, while host JavaScript owns memory placement,
resident data strategy, output ownership, and benchmark interpretation.
