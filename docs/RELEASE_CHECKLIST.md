# IntKernel Release Checklist

[简体中文](zh-CN/RELEASE_CHECKLIST.md)

Use this checklist before publishing or tagging a V0 release.

## Required Verification

- Run `pnpm test`.
- Run `pnpm typecheck`.
- Run `pnpm build`.
- Run `pnpm ikc --help` or the equivalent installed `ikc --help` command and review output.
- Run `pnpm ikc check examples/pricing.ik`.
- Run `pnpm ikc emit-c examples/pricing.ik --out build/pricing.c --header build/pricing.h`.
- Review generated `pricing.c` and `pricing.h`.
- If clang is available, run the e2e clang compile and harness test.
- Review generated C/header snapshot diffs.
- Review docs for V0 language, compiler architecture, ABI, and roadmap accuracy.
- Keep documentation bilingual: English remains the default entrypoint, and new
  or changed docs must update the matching Chinese translation.

## MIR / Default Pipeline Verification

- Run `pnpm ikc emit-mir examples/pricing.ik --out build/pricing.mir`.
- Review MIR output for stable formatting and absence of absolute paths,
  timestamps, or random IDs.
- Confirm MIR validator tests pass as part of `pnpm test`.
- Confirm MIR lowering and MIR C emitter tests pass as part of `pnpm test`.
- Confirm AST vs MIR regression tests pass while the legacy AST backend remains
  in the repository.
- Confirm default `emit-c` and `build` output is produced by the MIR pipeline.

## Checked Mode Verification

- Run the normal unchecked test suite.
- Run checked arithmetic tests and confirm checked e2e cases pass when clang is
  available.
- Review checked generated C/header snapshots.
- Confirm checked `emit-c` and `build` commands use `--overflow checked`.
- Confirm checked dynamic library builds still use strict clang flags.
- Manually run the checked Python `ctypes` example when the local platform has
  a generated checked dynamic library available.
- Manually run the checked Node.js FFI example when the local platform and FFI
  dependency are available.
- Review `docs/CHECKED_ARITHMETIC.md`, `docs/ABI.md`, and README checked-mode
  sections for ABI and safety-boundary accuracy.

## WASM Backend Verification

- Confirm `emit-wat` tests pass as part of `pnpm test`.
- Confirm `emit-wasm` tests pass as part of `pnpm test`.
- Confirm WASM scalar e2e tests pass.
- Confirm WASM control-flow, function-call, short-circuit, and memory e2e tests
  pass.
- Confirm `examples/pricing.ik` WASM e2e tests pass.
- Confirm `emit-wat --overflow checked` and `emit-wasm --overflow checked`
  fail with the documented unsupported-mode message.
- Review WAT snapshot diffs.
- Manually run the Node.js WASM example when `build/pricing.wasm` is available.
- Manually run the browser WASM example through a local HTTP server.
- Manually run `node bench/wasm_pricing_benchmark.mjs` after generating
  `build/pricing.wasm`.
- Review `docs/WASM_ABI.md` for type mapping, memory layout, examples, and
  safety-boundary accuracy.

## LLVM Backend Verification

- Review `docs/LLVM_BACKEND.md` before releasing LLVM backend changes.
- Confirm `emit-llvm` CLI tests pass.
- Confirm `build-llvm` clang command tests pass.
- Confirm `build-llvm --kind object` tests pass.
- Review LLVM IR snapshot diffs for stability and absence of absolute paths,
  timestamps, or random IDs.
- If clang is available, compile generated `.ll` in smoke tests.
- If clang is available, confirm LLVM pricing e2e passes.
- Confirm `build-llvm` reports a friendly error when clang is not available.
- Confirm `emit-llvm --overflow checked` fails with the documented unsupported
  message until checked LLVM lowering is implemented.
- Confirm `build-llvm --overflow checked` fails with the documented unsupported
  message until checked LLVM lowering is implemented.
- Confirm scalar, control-flow, function-call, ptr/index/field/store, and
  pricing LLVM e2e tests pass before release.
- Confirm C/WASM/LLVM backend regression comparison tests pass.
- Re-run C backend and WASM backend regression tests after LLVM backend changes.

## Optional Publishing Checks

- Run `npm pack --dry-run` if the package is intended to be published to npm.
- Confirm the package includes the built CLI entrypoint.
- Confirm examples and docs intended for users are included.
- Confirm no local build artifacts or temporary files are included by mistake.

## Release Notes

Before publishing, summarize:

- language features supported in V0
- known limitations
- ABI compatibility notes
- diagnostics and CLI changes
- any intentionally changed generated C/header output
