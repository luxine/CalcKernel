# IntKernel v0.4.0 Release Checklist

[简体中文](zh-CN/RELEASE_CHECKLIST.md)

Use this checklist before publishing or tagging `v0.4.0`. It is an artificial
release gate for human review; do not use benchmark results as language
semantics or as cross-machine performance truth.

## Release Scope

- Confirm the language and project are documented as IK / IntKernel.
- Confirm CLI examples use `ikc`.
- Confirm source examples use `.ik`.
- Confirm `package.json` version is `0.4.0`.
- Confirm the intended git tag is `v0.4.0`.
- Confirm the package metadata exposes only the `ikc` bin entrypoint.
- Confirm v0.4.0 release notes only advertise implemented Phase 14 behavior.
- Confirm floating point support is not advertised: `f64` is not implemented in
  v0.4.0 and belongs to a future Phase 16.
- Do not advertise unsupported f32, implicit int/float conversion, fast-math,
  SIMD, JIT, strings, IO, GC, runtime, or new backend support.

## Required Commands

- Run `pnpm test`.
- Run `pnpm typecheck`.
- Run `pnpm build`.
- Run `npm pack --dry-run`.
- Run `pnpm ikc --help`, or the equivalent installed `ikc --help`, and review
  the output.
- Run `pnpm ikc check examples/pricing.ik`.
- Run `pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h`.
- Run `pnpm ikc emit-mir examples/pricing.ik --out build/pricing.mir`.

## Naming Consistency

- Confirm `tests/naming-consistency.test.ts` passes as part of `pnpm test`.
- Review README, docs, examples, snapshots, and CLI help for canonical naming.
- Confirm package metadata, command examples, and release docs use IK /
  IntKernel, `ikc`, and `.ik`.
- Confirm no compatibility alias or alternate source suffix is introduced.

## C Backend Regression

- Confirm C emitter snapshot tests pass.
- Confirm C scalar expression coverage passes.
- Confirm C if/else and while coverage passes.
- Confirm C function-call coverage passes.
- Confirm C short-circuit `&&` / `||` coverage passes.
- Confirm C ptr/index, struct field, and combined access such as
  `items[i].price` coverage passes.
- Confirm `examples/pricing.ik` C e2e passes.
- Confirm unchecked mode remains the default and keeps the unchecked ABI.
- Confirm checked C mode passes scalar, control-flow, short-circuit,
  function-call, and `examples/pricing.ik` e2e coverage.
- Review checked generated C/header snapshots.

## WASM Backend Regression

- Confirm `emit-wat` tests pass.
- Confirm `emit-wasm` tests pass.
- Confirm WAT snapshots pass.
- Confirm WASM scalar e2e passes.
- Confirm WASM control-flow e2e passes.
- Confirm WASM function-call e2e passes.
- Confirm WASM short-circuit e2e passes.
- Confirm WASM memory / ptr e2e passes.
- Confirm WASM layout tests pass.
- Confirm `examples/pricing.ik` WASM e2e passes.
- Confirm `emit-wat --overflow checked` and `emit-wasm --overflow checked`
  fail with the documented unsupported-mode message.
- Review `docs/WASM_ABI.md` for type mapping, memory layout, examples, and
  safety-boundary accuracy.

## LLVM Backend Regression

- Confirm `emit-llvm` CLI tests pass.
- Confirm `build-llvm` clang command tests pass.
- Confirm `build-llvm --kind object` tests pass.
- Confirm LLVM IR snapshots pass.
- Confirm LLVM scalar e2e passes.
- Confirm LLVM control-flow e2e passes.
- Confirm LLVM function-call e2e passes.
- Confirm LLVM short-circuit e2e passes.
- Confirm LLVM ptr/index/field/store e2e passes.
- Confirm LLVM bool ABI e2e passes.
- Confirm `examples/pricing.ik` LLVM e2e passes.
- Confirm `emit-llvm --overflow checked` and `build-llvm --overflow checked`
  fail with the documented unsupported-mode message.
- Review `docs/LLVM_BACKEND.md` for backend limits and release notes.

## Cross-Backend Baseline

- Confirm C/WASM/LLVM backend regression comparison tests pass.
- Confirm the shared comparison includes scalar, control flow, function calls,
  short-circuit, memory, and `examples/pricing.ik`.
- Confirm no backend output snapshot changed unexpectedly.
- If any snapshot changes, explain the real expected behavior change before
  accepting the update. Do not update snapshots just to make tests pass.

## Optimization Correctness

- Review `docs/OPTIMIZATION.md`.
- Confirm the optimization pipeline defaults to `-O0`.
- Confirm `-O0`, `-O1`, `-O2`, and `-O3` tests pass as part of `pnpm test`.
- Confirm optimizer correctness coverage includes constant folding, copy
  propagation, dead code elimination, CFG simplify, local CSE, address CSE,
  small-function inlining, and loop optimization.
- Confirm checked mode keeps overflow and division checks except for
  documented proven-safe loop induction increments.
- Confirm short-circuit semantics are preserved before and after optimization.
- Confirm `examples/pricing.ik` produces matching results before and after
  optimization.
- Confirm there is no benchmark-specific special case for `examples/pricing.ik`.

## Benchmark Smoke

- Review `docs/PERFORMANCE.md`, `bench/README.md`, and
  `bench/README.zh-CN.md`.
- Run `node --test bench/perf/tests/perf-core.test.mjs`.
- Optionally run `node bench/perf/run.mjs --quick` as a manual benchmark smoke.
- Treat `node bench/perf/run.mjs --full` as an optional tag-time check when
  machine time allows.
- Do not add benchmark thresholds to ordinary `pnpm test`.
- Do not treat local benchmark results as cross-machine absolute baselines.
- If saving a baseline, follow the existing policy: local baselines live under
  ignored `build/perf` output, while `bench/perf/baselines/example.summary.json`
  remains a format example rather than a real threshold file.
- Do not commit machine-local benchmark outputs, real local baselines, caches,
  or temporary files.

## Docs And Examples Review

- Review README and README.zh-CN for v0.4.0 capability accuracy.
- Review language, architecture, MIR, ABI, checked arithmetic, WASM, LLVM,
  optimization, performance, and roadmap docs.
- Confirm docs say IK / IntKernel is a pure computation DSL, not a general
  purpose language.
- Confirm docs say current v0.4.0 computation is primarily integer-focused.
- Confirm docs do not claim floating point, SIMD, JIT, runtime, IO, strings, or
  GC support.
- Review examples under `examples/` for `ikc` and `.ik` commands.
- Confirm `examples/pricing.ik` remains the release e2e fixture.
- Keep documentation bilingual: English remains the default entrypoint, and
  changed docs should update the matching Chinese translation.

## Package Contents Review

- Confirm `npm pack --dry-run` reports package `intkernel@0.4.0`.
- Confirm the package includes `dist/src`, docs, examples, bench files,
  README.md, README.zh-CN.md, and package.json.
- Confirm the package includes the built CLI entrypoint referenced by the
  `ikc` bin mapping.
- Confirm package `exports`, `files`, and scripts match the published IK /
  IntKernel surface.
- Confirm no local build artifacts, benchmark output, real local baselines,
  caches, editor state, or temporary logs are included.

## Tag Gate

- Confirm all required commands and manual reviews above are complete.
- Confirm working tree changes are intentional and understood.
- Confirm release notes summarize only implemented v0.4.0 capability:
  lexer, parser, type checker, MIR, MIR validation, conservative MIR
  optimization levels, C/WASM/LLVM backends, checked C integer arithmetic,
  backend regression coverage, `examples/pricing.ik` e2e coverage, and manual
  performance suite.
- Confirm known limitations are listed: no floating point in v0.4.0, no
  implicit int/float conversion, no fast-math, no SIMD, no JIT, no IO, no
  strings, no GC, no runtime, and no checked WASM/LLVM arithmetic.
- Create tag `v0.4.0` only after the checklist is complete.
