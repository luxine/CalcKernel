# Changelog

All notable user-visible changes to CalcKernel are recorded here.

## 0.14.0 - Release candidate

- Added explicit offline Auto-Tuning with `ckc tune build|inspect` and
  fail-closed `ckc build --tune-use` replay. Ordinary commands remain
  measurement-free and never invoke a workload runner implicitly.
- Added closed schema-1 tuning manifests and `CKTUNE01` decisions, immutable
  no-follow workload snapshots, bounded deterministic search, correctness
  smoke checks, rotated measurements, two validation rounds, and independently
  checked source/artifact replay.
- Added owner-private compile/measurement/completed-decision cache domains below
  `tune-v1`, exact warm reuse, deterministic 4 GiB LRU eviction, and
  journaled crash-recoverable multi-file publication.
- Advanced the private Native object cache to `CKCOBJ04` key/manifest schema 5.
  Public language, Native C ABI 1, Runtime ABI 2, KIR v3, LLVM bridge ABI 4,
  profile schema 1, and multiversion schema 1 remain unchanged.
- Added schema-9 tuning performance evidence and exact candidate-SHA CI gates.
  Formal release still requires both controlled architecture workers and all
  required remote jobs to pass.

## 0.13.0 - Release candidate

- Added deterministic CK-owned `CKPART01` shards and `CKPROF01` workload
  profiles, with directory-safe collection, canonical merge/inspection, and the
  transactional `ckc pgo build` convenience workflow.
- Added non-proof profile analysis and independently checked O2 late machine
  layout plus O3 guarded specialization, inlining, unrolling, SLP, and Loop SIMD
  decisions. Profiles may affect profitability but never establish safety.
- Added explicit `--cpu multiversion` Native builds with one portable baseline,
  bounded verified feature variants, a baseline-safe process-local detector,
  stable public thunks, and executable/dynamic/static named-object assembly.
- Added real library-generation workflows with the full-identity
  `ck_profile_flush_*` control symbol. Final profile-use artifacts contain no
  counters, profile paths, writer, or generation runtime.
- Advanced the private LLVM bridge to ABI 4, KIR to v3, and the Native object
  cache to `CKCOBJ03` key/manifest schema 4. Public Native C ABI 1 and Runtime
  ABI 2 remain unchanged; 0.12 source and observable semantics remain accepted.
- Added closed profile/target/dispatch/cache identities, corruption and mutation
  tests, transactional multi-file output, and release-candidate audits. Schema-8
  performance and exact-SHA CI remain required before a formal release.
- Auto-Tuning, indirect-call promotion, scalable KIR vectors, and adaptive JIT
  PGO remain future work for 0.14 or later.

## 0.12.0 - Unreleased

- Added KIR v2 fixed-vector and mask instructions plus deterministic Native
  `KirTargetProfile` capability/cost data derived from pinned LLVM 22.1.8.
- Added transactional, independently checked O3 specialization, controlled
  full/partial unrolling, SLP, and Loop SIMD frontiers with monotonic analysis
  budgets and stable optimization explanations.
- Added unit-stride Loop SIMD for integer and strict element-wise f64 arithmetic,
  supported casts, pure compare/select diamonds, splats, and contiguous memory;
  strict f64 keeps per-element ordering and never enables fast math.
- Added total runtime alias versioning with the unchanged scalar loop as the
  fallback, scalar epilogues, and exact unchecked modular u32 add/multiply
  reductions. Checked failures, effects, unsupported recurrences, scans, C,
  and WebAssembly remain scalar.
- Advanced the private LLVM bridge to ABI 3, KIR identity to `kir-v2`, and the
  Native object cache to `CKCOBJ02` key/manifest schema 3 while retaining public
  Native C ABI 1, Runtime ABI 2, source syntax, diagnostics, and checked
  first-error behavior.
- Added KIR/pre-LLVM/object structural evidence, fixed-seed O0/O3 differential
  coverage, mutation tests, target-feature containment, and schema-7 performance
  gate inputs. PGO/multiversioning and Auto-Tuning remain future work.

## 0.11.0 - Unreleased

- Added explicit `unsafe fn` contracts for affine range requirements,
  `multiple_of`, `noalias`, alignment, and slice memory-effect ceilings. Unsafe
  calls require an `unsafe { ... }` statement and executable `main` remains safe.
- Added deterministic `emit-kir` inspection, verified facts/effect summaries,
  proof-carrying guard elimination explanations, and opt-in Native contract
  sanitization with `CKR0007`.
- Replaced the former target-neutral MIR optimizer with one verified KIR
  pipeline shared by C, WebAssembly, and Native LLVM. Semantic MIR and stable
  `emit-mir` output remain the source-order and first-error boundary.
- Added scalar/path, region alias and Memory SSA, interprocedural effect, loop,
  GVN/load-forwarding/dead-store, LICM, and evidence-audited backend facts.
- Kept Native C ABI 1 while advancing the private LLVM bridge and runtime ABI
  to 2 and using the KIR v1 native cache/code-generation identity.
- Added fixed-seed differential and mutation suites, pre-LLVM fact audits, and
  performance gates against both pinned Clang and exact CalcKernel 0.10.

## 0.10.0 - 2026-08-27

- Added parameterless internal `main`, `ckc run`, and Native executable output.
- Added deterministic Native output builtins for signed/unsigned integers,
  `f64`, booleans, and newline; library, C, and WASM roots reject reachable print.
- Replaced product Clang subprocesses with pinned LLVM 22.1.8 structural code
  generation, ORC execution, archive writing, and in-process LLD linking.
- Expanded `ckc build --kind` to executable, dynamic, static, and object outputs;
  dynamic remains the default and `build-llvm` is a deprecated compatibility alias.
- Unified Native object/static/dynamic exports under one generated-header Native
  C ABI with target ABI classification and checked status thunks.
- Added Native checked overflow and slice bounds, preserving the C `CK_Status`
  meanings and first-error order.
- Added an isolated `run` child, secure persistent object cache, fixed runtime
  diagnostics/statuses, eager symbol resolution, and audited JIT page permissions.
- Added checked/unchecked C-oracle performance gates and six-host functional,
  artifact, dependency, provenance, and immutable release gates.
- Reserved `main` and the seven print builtin names, limited Native target output
  to the host, retired the standalone LLVM exported-shape promise, and kept
  `emit-c` source-only. See the compatibility policy for migration guidance.

## 0.9.0 - 2026-08-26

- Added `break` and `continue` for structured control inside `while` loops.
- Added explicit `void` procedures, empty `return;`, and procedure-call statements.
- Added non-owning `slice<T>` values, `slice(data, len)`, indexing, `.data` / `.len`,
  and half-open sub-slices written as `items[start..end]`.
- Added optional checked slice bounds to the C backend through `--bounds checked`;
  unchecked bounds remain the default, while WASM and LLVM reject checked bounds.
- Stabilized native C, WebAssembly, and LLVM output paths and their V0.9 ABIs.
- Reorganized the repository around durable compiler, contract, example, benchmark,
  and test responsibilities without changing the compiler's public behavior.
- Froze the V0.9 compatibility boundary: patch releases in the `0.9.x` line preserve
  accepted source, diagnostic identifiers, CLI behavior, textual MIR, and documented
  ABI contracts. A later `0.10.0` may make documented breaking changes with migration
  guidance; long-term compatibility begins with a future `1.0.0` release.
- Added signed-off native `ckc` release archives and SHA-256 checksums for macOS,
  Linux, and Windows on both arm64 and x64.
