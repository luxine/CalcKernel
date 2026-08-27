# CalcKernel 0.10 Native Toolchain Stage Tasks

[简体中文](../../zh-CN/compiler/native-toolchain-implementation/stage-tasks.md)

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` and execute every checkbox in order. This plan
> must be executed in line without sub-agents.

**Goal:** Convert the approved 0.10 native-toolchain design into small,
test-first implementation tasks with repository-specific files and commands.

**Architecture:** The existing frontend and MIR remain the semantic center.
One structural LLVM backend supplies inspection IR, native objects, libraries,
executables, and ORC execution through a pinned bridge. The CLI coordinates
typed compiler services and transactional output rather than running external
toolchains.

**Tech Stack:** Rust 2024, LLVM/ORC/LLD 22.1.8, C++20 bridge, CMake/Ninja,
GitHub Actions, and platform-native validation tools.

---

Before each task, run the named red test and confirm that it fails for the
missing behavior rather than a setup error. After the minimum code passes,
run the task's focused suite and refactor while it stays green. At every stage
boundary run the matching section of [stage acceptance](stage-acceptance.md).

## Stage 1 — Pinned native dependency and ownership boundary

### Task 1.1: Declare the build profile and exact source manifest

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `build.rs`
- Create: `native/llvm/manifest.toml`
- Create: `scripts/bootstrap-llvm.sh`
- Create: `scripts/bootstrap-llvm.ps1`
- Create: `tests/contracts/native_toolchain.rs`
- Modify: `tests/contracts.rs`
- Modify: `.gitignore`

- [ ] Add a contract test requiring the `native-toolchain` feature, exact
  LLVM `22.1.8`, source tag and SHA-256 fields, host-only targets, static-link
  policy, bootstrap scripts, and ignored bootstrap outputs. Run
  `cargo test --locked --test contracts native_toolchain_manifest` and observe
  the missing-contract failure.
- [ ] Add `cc` as a build dependency, `sha2` as a runtime dependency, and
  target-scoped system API dependencies only where the standard library cannot
  express a required ownership or process operation. Do not add `llvm-sys`,
  Clang, a dynamic loader, or a general linker-command dependency.
- [ ] Implement both bootstrap scripts with explicit input/output paths,
  checksum verification before extraction, a locked CMake configuration, host
  target selection, assertions that Clang is excluded from the release profile,
  and an installed component manifest consumed by `build.rs`. Add a separate
  `oracle` profile that builds Clang 22 from the same verified source into a
  different prefix and prints its exact executable path for CI.
- [ ] Make `build.rs` a no-op when the feature is disabled. With the feature
  enabled, require `CKC_LLVM_PREFIX`, validate the exact manifest/version, build
  the bridge, and emit target-specific static link directives. Reject missing,
  mismatched, or shared-only prefixes with actionable errors.
- [ ] Run the contract test green, then run `cargo build --locked` without the
  feature to prove that ordinary development does not acquire LLVM.

### Task 1.2: Define the exception-safe bridge contract

**Files:**

- Create: `native/bridge/ckc_llvm.h`
- Create: `native/bridge/ckc_llvm.cpp`
- Create: `src/backend/llvm/ffi.rs`
- Create: `src/backend/llvm/error.rs`
- Create: `tests/native.rs`
- Create: `tests/native/bridge.rs`

- [ ] Add compile-time C header assertions and Rust integration tests for ABI
  version, LLVM version, target triple, success/error result ownership, and
  paired string/byte-buffer release. First run the native bridge test and
  observe unresolved or unavailable bridge behavior.
- [ ] Expose only C-compatible opaque handles, integer status codes, byte
  spans, and owned bridge errors. Catch every C++ exception at each exported
  bridge function. Never let `std::string`, C++ containers, exceptions, or
  LLVM classes cross the header.
- [ ] Wrap raw calls in a typed Rust `NativeError` that preserves the failing
  stage. Convert and free bridge-owned errors exactly once.
- [ ] Test deliberate invalid input and injected bridge failure under ASan on
  a native CI build; the Rust test must see a typed error, not abort or unwind.

### Task 1.3: Establish LLVM/ORC lifetime owners

**Files:**

- Create: `src/backend/llvm/context.rs`
- Create: `src/backend/llvm/target.rs`
- Create: `src/backend/llvm/jit.rs`
- Modify: `src/backend/llvm/mod.rs`
- Modify: `src/backend/mod.rs`
- Modify: `src/lib.rs`
- Create: `tests/native/ownership.rs`

- [ ] Add failing tests that repeatedly create/drop contexts, targets, modules,
  objects, and empty JIT instances, including an injected middle-stage error.
- [ ] Implement non-`Clone` safe wrappers whose constructors validate target
  and ownership relationships. Keep opaque pointers private and constrain
  `Send`/`Sync` to what LLVM documents; default to neither when uncertain.
- [ ] Implement reverse-order `Drop` and explicit ORC error consumption. On
  Windows AArch64 select the reserve-enabled RuntimeDyld layer; on the other
  five hosts select JITLink. Report the selection through a typed enum.
- [ ] Run repeated ownership tests under ASan/LSan in native CI and the focused
  Rust suite locally.

### Task 1.4: Report the embedded toolchain and notices

**Files:**

- Create: `src/backend/llvm/notices.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/cli/commands.rs`
- Modify: `src/bin/ckc.rs`
- Modify: `tests/cli.rs`
- Modify: `tests/cli/commands.rs`

- [ ] Add failing CLI tests for `ckc --version`, `ckc --version --verbose`,
  `ckc licenses`, and the single native-unavailable error in a feature-disabled
  developer binary.
- [ ] Embed the compiler, LLVM, Native ABI, runtime ABI, host target, enabled
  code generator, and active ORC object layer at build time. Embed required
  LLVM and third-party notices as bytes and print them without external files.
- [ ] Keep `src/bin/ckc.rs` limited to process argument/exit plumbing; command
  parsing and messages remain in `src/cli`.

## Stage 2 — Entry point, print builtins, effects, and roots

### Task 2.1: Reserve and validate `main`

**Files:**

- Modify: `src/frontend/typeck.rs`
- Modify: `src/frontend/diagnostics.rs`
- Modify: `tests/frontend/checker.rs`
- Modify: `tests/frontend/surface.rs`
- Add fixtures: `tests/fixtures/native/entry/*.ck`

- [x] Add one failing test for each rejected `main` shape: parameters,
  exported declaration, unsupported result, duplicate entry, and entry required
  by an executable consumer. Add accepted void/i32 and library-without-main
  cases. Diagnostics must have stable `CK` identifiers and exact spans.
- [x] Represent entry classification in checked program data instead of
  re-parsing names in the CLI. A valid `main` remains internal for library and
  object roots.
- [x] Preserve V0.9 behavior for ordinary functions and exports, then run the
  complete frontend suite.

### Task 2.2: Add the seven reserved native print symbols

**Files:**

- Modify: `src/frontend/typeck.rs`
- Modify: `src/frontend/ast.rs` only if typed builtin identity needs it
- Modify: `tests/frontend/checker.rs`
- Add fixtures: `tests/fixtures/native/print/*.ck`

- [x] Add failing tests for exact signatures, void-only statement use,
  arity/type errors, user redeclaration, and all seven names.
- [x] Extend compiler builtin metadata with backend availability and observable
  effect identity. Do not model prints as user declarations or allow their
  addresses to escape.
- [x] Ensure `check` and MIR inspection accept valid calls while source errors
  remain ordinary diagnostics.

### Task 2.3: Carry entry and print effects through MIR

**Files:**

- Modify: `src/ir/model.rs`
- Modify: `src/ir/lower.rs`
- Modify: `src/ir/validate.rs`
- Modify: `src/ir/print.rs`
- Modify: `tests/ir/mir.rs`

- [x] Add failing MIR tests for entry metadata, typed print instructions,
  source-order operands, validator rejection of invalid builtin signatures,
  and stable inspection text.
- [x] Introduce an explicit effectful runtime-call MIR instruction or equally
  typed callee identity. Do not encode a print as a normal removable call by
  name.
- [x] Lower arguments before the effect and preserve source evaluation order.
  Keep print calls void even in module-wide checked mode.

### Task 2.4: Make optimization and artifact-root analysis effect-aware

**Files:**

- Modify: `src/optimizer/analysis.rs`
- Modify: `src/optimizer/passes/dce.rs`
- Modify: `src/optimizer/passes/inlining.rs`
- Modify: `src/optimizer/passes/cse.rs`
- Modify: `src/optimizer/passes/loops.rs`
- Modify: `src/optimizer/pipeline.rs`
- Modify: `tests/optimizer/passes.rs`
- Create: `src/ir/reachability.rs`
- Modify: `src/ir/mod.rs`

- [x] Add failing O0-O3 tests proving print calls are not removed, duplicated,
  combined, hoisted, sunk, or reordered. Include calls nested in inlined
  functions and loops.
- [x] Centralize effect classification and make every transforming pass query
  it. Add artifact-root reachability for entry, requested exports, and
  non-executable print rejection.
- [x] Add failing/backend tests proving C/WASM and native library artifacts
  reject reachable prints before any output is written, while unreachable
  print-only functions can be eliminated from library roots.

## Stage 3 — Structural LLVM lowering and verified native objects

### Task 3.1: Replace text concatenation with a structural module builder

**Files:**

- Replace: `src/backend/llvm/emit.rs`
- Create: `src/backend/llvm/module.rs`
- Create: `src/backend/llvm/lower.rs`
- Modify: `src/backend/llvm/layout.rs`
- Modify: `src/backend/llvm/names.rs`
- Modify: `src/backend/llvm/mod.rs`
- Modify: `tests/backend/llvm.rs`
- Create: `tests/native/llvm_ir.rs`

- [x] Migrate one scalar fixture at a time to failing structural tests that
  inspect the verified module printed by LLVM. Cover constants, arithmetic,
  comparisons, branches, loops, phi values, calls, void, structs, pointers,
  slices, index, and sub-slice.
- [x] Build LLVM values and blocks through safe wrappers; never construct IR by
  interpolating strings. Generate target triple and DataLayout from the host
  TargetMachine before laying out types.
- [x] Make `emit-llvm` print this module and reject a normalized non-host target
  before opening its destination.
- [x] Delete obsolete textual emission helpers only after every migrated
  backend test passes.

### Task 3.2: Verify, optimize, and code-generate through typed states

**Files:**

- Create: `src/backend/llvm/verify.rs`
- Create: `src/backend/llvm/passes.rs`
- Create: `src/backend/llvm/object.rs`
- Modify: `src/backend/llvm/context.rs`
- Modify: `tests/native/llvm_ir.rs`
- Create: `tests/native/object.rs`

- [x] Add failing tests for verifier rejection, O0-O3 pipeline selection,
  strict floating-point flags, baseline/native CPU attributes, object magic,
  and host-target rejection.
- [x] Make construction return an unverified module, verification return
  `VerifiedModule`, PassBuilder consume it and return `OptimizedModule` only
  after a second verification, and TargetMachine consume that state for object
  bytes.
- [x] Use the same optimization selection for MIR and LLVM. O3 must not set
  fast-math or contract strict operations.
- [x] Parse emitted object bytes through LLVM before exposing `NativeObject`;
  preserve compiler stage in all errors.

### Task 3.3: Implement unchecked and checked lowering

**Files:**

- Create: `src/backend/llvm/checked.rs`
- Modify: `src/backend/llvm/lower.rs`
- Modify: `src/backend/llvm/module.rs`
- Modify: `tests/native/llvm_ir.rs`

- [x] Add failing tests for all four overflow/bounds combinations, including
  arithmetic overflow intrinsics, signed division/modulo zero and minimum/-1,
  slice index and `start <= end <= len`, first-error ordering, void results,
  and checked result pointers.
- [x] Lower checked control flow to explicit `CK_Status` propagation without
  traps. Keep unchecked code guard-free according to its existing semantics.
- [x] Compare representative checked modules with pinned structural fixtures
  and Clang-derived operation semantics where execution is not required. Defer
  executable value/status differential tests to the stage 4 library harness
  and stages 5-6 entry harnesses, which have a valid in-process execution path.
  Treat Clang absence as an explicit skipped oracle only outside required
  native CI, never as product fallback.

### Task 3.4: Enforce backend availability before output

**Files:**

- Modify: `src/cli/args.rs`
- Modify: `src/cli/commands.rs`
- Modify: `src/cli/output.rs`
- Modify: `tests/cli/commands.rs`
- Modify: `tests/cli/oracle_portability.rs`

- [x] Add failing tests for O3 defaults on `run`/`build`, O0 defaults on
  inspection commands, `-O0` through `-O3`, checked-mode acceptance, CPU
  policy, host target normalization, and absence of partial outputs on errors.
- [x] Parse command-specific options rather than allowing unknown flags or
  irrelevant values to leak between commands. Model artifact kind and CPU as
  enums.
- [x] Remove every product target probe and Clang invocation. Keep Clang command
  helpers under `tests/support/oracle.rs` only.

## Stage 4 — Native C ABI and library artifacts

### Task 4.1: Define target-family ABI classifiers

**Files:**

- Create: `src/backend/native_abi/mod.rs`
- Create: `src/backend/native_abi/model.rs`
- Create: `src/backend/native_abi/sysv_x64.rs`
- Create: `src/backend/native_abi/darwin_x64.rs`
- Create: `src/backend/native_abi/aapcs64.rs`
- Create: `src/backend/native_abi/windows.rs`
- Modify: `src/backend/mod.rs`
- Create: `tests/native/abi.rs`
- Add fixtures: `tests/fixtures/native/abi/*`

- [x] Generate failing table tests for every supported primitive, pointer,
  slice, struct size/alignment boundary, aggregate parameter/return, bool,
  checked result, and target family.
- [x] Implement explicit register class, indirect/by-value, extension,
  alignment, and hidden-result decisions without querying host C layout for a
  foreign family.
- [x] On each release host, compare LLVM function attributes and calling
  sequence fixtures with pinned Clang 22 development-oracle output.

### Task 4.2: Generate and verify Native C ABI export thunks

**Files:**

- Create: `src/backend/llvm/abi.rs`
- Modify: `src/backend/llvm/module.rs`
- Modify: `src/backend/c/emit.rs`
- Modify: `src/backend/c/layout.rs`
- Create: `src/backend/header.rs`
- Modify: `src/backend/mod.rs`
- Modify: `tests/native/abi.rs`
- Modify: `tests/backend/c.rs`
- Create: `tests/native/differential.rs`

- [x] Add failing tests that compare generated native headers with existing C
  commitments and assert no internal LLVM signature is exported.
- [x] Move only shared header/layout concepts into `backend::header` and
  `native_abi`; retain C emitter-specific text in `backend::c`.
- [x] Insert external thunks before O3, preserve source symbol names and
  visibility, and allow LLVM to inline internal implementations without
  deleting the public boundary.
- [x] Compile every generated header in a pinned C harness on all target jobs.
- [x] Through the stage 4 system-FFI loader, differentially execute all exported
  scalar, control-flow, void, call, struct, pointer, slice, and checked-ordering
  fixtures against libraries produced from C emission by `CKC_CLANG_ORACLE`.

### Task 4.3: Create object and static artifacts in process

**Files:**

- Create: `src/backend/artifact/mod.rs`
- Create: `src/backend/artifact/archive.rs`
- Modify: `native/bridge/ckc_llvm.h`
- Modify: `native/bridge/ckc_llvm.cpp`
- Modify: `src/backend/mod.rs`
- Create: `tests/native/artifacts.rs`

- [x] Add failing tests for platform suffixes, object/header pairs, deterministic
  static archives, archive symbol indexes, and rejection of arbitrary input
  objects.
- [x] Expose LLVM archive writing through the trusted bridge. Its Rust API
  accepts only `NativeObject` plus compiler-owned helper identities.
- [x] Validate every staged object/archive before returning it to the CLI.

### Task 4.4: Link dynamic libraries and commit multi-file output

**Files:**

- Create: `src/backend/artifact/lld.rs`
- Create: `src/backend/artifact/platform.rs`
- Modify: `native/bridge/ckc_llvm.h`
- Modify: `native/bridge/ckc_llvm.cpp`
- Modify: `src/cli/output.rs`
- Modify: `src/cli/commands.rs`
- Create: `tests/native/libraries.rs`
- Modify: `tests/cli/commands.rs`

- [x] Add failing tests for trusted LLD arguments, `.so`/`.dylib`/`.dll` plus
  Windows import library, header `CK_API` mode, pre-commit failure, commit-time
  rollback, symlink rejection, and cleanup.
- [x] Bridge `lld::lldMain` with captured diagnostics and an allowlisted argument
  builder. No user object, library, linker script, response file, or raw flag
  enters it.
- [x] Implement same-filesystem staging and per-file atomic replacement with
  backups for a multi-output transaction. A failure before commit leaves all
  destinations unchanged; a commit failure attempts and reports rollback.
- [x] Load the resulting dynamic library through system FFI with an empty tool
  PATH and exercise every exported shape and checked combination.

### Task 4.5: Unify `build` and deprecate `build-llvm`

**Files:**

- Modify: `src/cli/args.rs`
- Modify: `src/cli/commands.rs`
- Remove native product use from: `src/cli/toolchain.rs`
- Modify: `tests/cli/commands.rs`
- Modify: `tests/cli/oracle_readiness.rs`

- [x] Add failing CLI tests for four `--kind` values, dynamic default, CPU
  modes, header rules, exact output paths, empty PATH, and one deprecation
  warning from supported `build-llvm` dynamic/object forms.
- [x] Route `build` directly to native backend services. Reject executable
  without entry and reject reachable print calls for library/object forms
  before staging.
- [x] Delete product Clang discovery, generated `.c`/`.ll` intermediates, and
  fallback messages. Retain `emit-c` as emission only.

## Stage 5 — Minimal runtime and standalone executable artifacts

### Task 5.1: Implement platform write/exit and stable runtime failures

**Files:**

- Create: `native/runtime/include/ckc_runtime.h`
- Create: `native/runtime/common/runtime.c`
- Create: `native/runtime/linux/syscalls.S`
- Create: `native/runtime/darwin/process.c`
- Create: `native/runtime/windows/process.c`
- Create: `native/runtime/provenance.toml`
- Create: `tests/native/runtime.rs`

- [x] Add failing byte-exact tests for `CKR0001` through `CKR0006`, exit codes
  240-245, stdout-failure fallback, no heap symbol imports, and LF on every OS.
- [x] Implement bounded stack writes and platform-specific process APIs only.
  Runtime allocation, locale, libc formatting, CK dynamic runtime, and process
  crash handlers are forbidden.
- [x] Compile runtime/entry objects during bootstrap, record their hashes, and
  embed selected host bytes into `ckc` at Cargo build time.

### Task 5.2: Implement no-allocation numeric formatting

**Files:**

- Create: `native/runtime/common/format_int.c`
- Create: `native/runtime/common/format_float.c`
- Add vendored algorithm sources and licenses under:
  `native/runtime/vendor/`
- Modify: `native/runtime/provenance.toml`
- Modify: `tests/native/runtime.rs`

- [x] Add failing tests for integer extrema, booleans, value functions without
  newline, `print_newline`, finite f64 shortest round trip, halfway cases,
  subnormals, infinities, NaN, and preserved `-0.0`.
- [x] Vendor a permissively licensed, bounded-buffer shortest-round-trip
  algorithm and retain its exact notice. Adapt it without heap, locale, static
  mutable state, or libc formatting.
- [x] Differentially compare every finite generated spelling by parsing it back
  to identical f64 bits, except the documented NaN payload/sign erasure.

### Task 5.3: Build entry wrappers and standalone executables

**Files:**

- Create: `src/backend/llvm/entry.rs`
- Modify: `src/backend/llvm/module.rs`
- Modify: `src/backend/artifact/platform.rs`
- Modify: `src/backend/artifact/lld.rs`
- Add platform link inputs under: `native/runtime/platform/`
- Modify: `tests/native/artifacts.rs`
- Create: `tests/native/executable.rs`

- [x] Add failing tests for void/i32 main, checked entry result pointers,
  propagated runtime diagnostics, application exit values, no-main failure,
  print reachability, and no sibling header.
- [x] Generate a compiler-owned process entry wrapper. Link only the verified
  program object, embedded runtime/entry/helper objects, allowlisted exports,
  and embedded platform import metadata.
- [x] Linux uses its syscall boundary; Windows uses stable import definitions
  and `/noentry` only for DLLs; Darwin supplies the pinned minimal libSystem
  text stub, explicit platform version, and LLD ad-hoc signing.
- [x] Execute artifacts with an empty external-tool PATH and compare stdout,
  stderr, and exit status against the runtime contract.

### Task 5.4: Prove zero-runtime dependencies

**Files:**

- Create: `scripts/audit-native-artifact.sh`
- Create: `scripts/audit-native-artifact.ps1`
- Modify: `tests/native/artifacts.rs`
- Modify: `native/runtime/provenance.toml`

- [x] Add failing audits for ELF `DT_NEEDED`, Mach-O load commands, PE imports,
  exported symbols, forbidden LLVM/LLD/Clang/CK names, compiler helpers, and
  runtime object provenance/hash drift.
- [x] Allow only the platform loader/API dependencies explicitly listed by the
  design. Link any required permissive compiler helper statically and include
  its notice.
- [x] Audit object, static, dynamic, and executable artifacts independently.

## Stage 6 — ORC execution, parent/child isolation, and cache

### Task 6.1: Link and execute the same native object with ORC

**Files:**

- Modify: `src/backend/llvm/jit.rs`
- Modify: `native/bridge/ckc_llvm.h`
- Modify: `native/bridge/ckc_llvm.cpp`
- Create: `tests/native/jit.rs`

- [ ] Add failing tests for eager symbol resolution, entry lookup, embedded
  runtime symbols, unchecked/checked combinations, no lazy hot-function stub,
  and object-layer selection.
- [ ] Feed ORC the same O3 `NativeObject` used by AOT. Use JITLink on five hosts
  and reserve-enabled RuntimeDyld/SectionMemoryManager on Windows AArch64.
- [ ] Resolve every symbol before calling entry and return typed compile/link/
  lookup errors before executing user code.

### Task 6.2: Implement the private child and public run parent

**Files:**

- Create: `src/cli/run.rs`
- Modify: `src/cli/mod.rs`
- Modify: `src/cli/commands.rs`
- Modify: `src/cli/args.rs`
- Modify: `src/bin/ckc.rs`
- Create: `tests/native/run.rs`
- Modify: `tests/cli/commands.rs`

- [ ] Add failing CLI tests proving the public parent self-spawns the exact
  executable in an unforgeable/private child mode, inherits program stdio,
  prints no success text, returns normal status, forwards interrupt, and maps
  identifiable signals/exceptions to exact `CKR0006`.
- [ ] Keep compilation, cache, ORC, and user machine code in the child. The
  parent validates the private protocol and never loads generated code.
- [ ] Distinguish compiler failure, normal checked failure, program exit, output
  failure, and abnormal termination without replacing a more specific status.

### Task 6.3: Define a canonical cache key and validated entry

**Files:**

- Create: `src/cli/cache/mod.rs`
- Create: `src/cli/cache/key.rs`
- Create: `src/cli/cache/entry.rs`
- Modify: `src/cli/mod.rs`
- Create: `tests/native/cache.rs`

- [ ] Add failing unit vectors for the versioned canonical serialization and
  lowercase SHA-256 name. Mutate each required semantic input and prove the key
  changes.
- [ ] Encode lengths and integers in an architecture-independent format; never
  hash debug output, unordered map iteration, paths, timestamps, or host-native
  integer bytes.
- [ ] Store a bounded manifest, object bytes, and digest over both. Validate
  size, version, key, digest, and LLVM object parsing before returning a hit.

### Task 6.4: Secure cache storage, eviction, and clean

**Files:**

- Create: `src/cli/cache/path.rs`
- Create: `src/cli/cache/store.rs`
- Create: `src/cli/cache/evict.rs`
- Modify: `src/cli/commands.rs`
- Modify: `src/cli/args.rs`
- Modify: `tests/native/cache.rs`
- Modify: `tests/cli/commands.rs`

- [ ] Add failing tests for all three OS path rules, missing base directory,
  owner-only creation, unsafe ownership/permissions, symlinks, corruption,
  concurrent writers, atomic rename, 1-GiB soft limit, deterministic
  best-effort LRU, `--no-cache`, and `ckc cache clean` scope.
- [ ] Treat every invalid entry or unsafe cache root as a miss. Disable caching
  when a required base cannot be resolved; do not invent a globally writable
  path and do not fail a valid source run because cache maintenance failed.
- [ ] Use owner-checked same-filesystem temporary files and no-follow/open-new
  semantics where the host exposes them. Clean only the resolved CK cache root.

### Task 6.5: Prove JIT memory-protection behavior

**Files:**

- Extend: `native/bridge/ckc_llvm.h`
- Extend: `native/bridge/ckc_llvm.cpp`
- Modify: `tests/native/jit.rs`
- Create: `scripts/audit-jit-memory.sh`
- Create: `scripts/audit-jit-memory.ps1`

- [ ] Add host tests that observe writable/non-executable allocation during
  relocation and final read/execute code plus non-executable data on Linux and
  Windows, including Windows AArch64 instruction-cache finalization.
- [ ] On Darwin, test `MAP_JIT` plus per-thread write-protection transitions;
  do not reject a mapping merely because its maximum permissions include both
  write and execute.
- [ ] Test signed and hardened macOS release candidates with only the narrowly
  required JIT entitlement.

## Stage 7 — Performance, CI, release, and legal closure

### Task 7.1: Build the strict differential performance harness

**Files:**

- Modify: `benches/ckc_perf.rs`
- Modify: `tests/performance/bench.rs`
- Modify: `tests/performance/oracle_fixtures.rs`
- Add fixtures: `tests/fixtures/performance/native/*.ck`
- Create: `scripts/check-native-performance.py`

- [ ] Add failing harness tests for reference equivalence, warm-up, sample
  stability, geometric mean, individual regression threshold, checked and
  unchecked separation, CPU policy, and rejection of fast-math references.
- [ ] Compile the C reference with pinned Clang strict `-O3`; compile CK through
  native TargetMachine O3 with the same baseline/native selection. Batch FFI
  calls and report compilation, cold run, warm run, memory, artifact size, and
  throughput separately.
- [ ] Gate geometric mean at 95% and each kernel at no more than 10% slower,
  unless a reviewed reproducible target limitation is added to the normative
  release evidence rather than hidden in the harness.

### Task 7.2: Add pinned native integration CI

**Files:**

- Modify: `.github/workflows/ci.yml`
- Create: `.github/actions/bootstrap-ckc-llvm/action.yml`
- Modify: `tests/contracts/ci.rs`
- Modify: `tests/contracts/native_toolchain.rs`

- [ ] Add failing workflow contract tests for exact manifest/checksum use,
  cached host bootstrap, fast non-native quality job, native all-feature lint
  and test job, six-host functional matrix, and controlled x86-64/AArch64
  performance workers.
- [ ] Keep the fast job independent of LLVM and remove its incorrect
  `--all-features`. Make the required native job run fmt, all-feature clippy,
  all tests, bridge sanitizers where supported, artifact/dependency audits,
  JIT permissions, and cache/process suites.
- [ ] Pin every action and external tool version according to repository release
  policy. CI may acquire the checksum-verified LLVM source; it may not accept a
  runner's arbitrary system LLVM.

### Task 7.3: Produce complete six-host release archives

**Files:**

- Modify: `.github/workflows/native-release.yml`
- Modify: `tests/contracts/release.rs`
- Modify: `docs/project/release.md`
- Modify: `docs/zh-CN/project/release.md`
- Modify: `docs/project/release-checklist.md`
- Modify: `docs/zh-CN/project/release-checklist.md`

- [ ] Add failing release contract tests requiring native feature builds,
  exact six archive names, checksum sidecars, verbose version evidence,
  notices, zero-dependency audits, functional run/build smoke tests, macOS
  signing/JIT checks, and immutable GitHub release behavior.
- [ ] Bootstrap target-minimal static LLVM/ORC/LLD on each host, build one
  self-contained `ckc`, and reject any archive with dynamic LLVM/LLD/Clang or
  non-system C++ runtime dependencies.
- [ ] Preserve existing archive names and publish only after all six artifacts
  and checksum verification complete.

### Task 7.4: Close source and license provenance

**Files:**

- Create: `THIRD_PARTY_NOTICES.md`
- Modify: `src/backend/llvm/notices.rs`
- Modify: `native/llvm/manifest.toml`
- Modify: `native/runtime/provenance.toml`
- Modify: `tests/contracts/native_toolchain.rs`

- [ ] Add failing tests that enumerate every embedded or statically linked
  third-party component and compare source hash, license file, notice text, and
  `ckc licenses` output.
- [ ] Make missing, stale, or unreferenced provenance fail both source builds
  and release CI.

## Stage 8 — 0.10 contract and repository freeze

### Task 8.1: Update the normative bilingual contract

**Files:**

- Modify paired English/Chinese files under: `docs/reference/`, `docs/abi/`,
  `docs/compiler/`, `docs/guides/`, and `docs/project/`
- Modify: `docs/index.md`
- Modify: `docs/zh-CN/index.md`
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `CHANGELOG.md`
- Modify: `CHANGELOG.zh-CN.md`
- Modify: `tests/contracts/docs.rs`

- [ ] First update contract tests to require current 0.10 language, CLI, MIR,
  Native C ABI, runtime, compatibility, security, build, performance, and
  release wording, with recursive bilingual mirrors and valid links.
- [ ] Replace superseded V0.9-only promises rather than retaining design-history
  narratives. Keep C and WebAssembly behavior explicit and do not imply print
  support there.
- [ ] Make examples executable where practical and run all documentation
  contract tests.

### Task 8.2: Set version 0.10.0 and freeze compatibility fixtures

**Files:**

- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: compatibility fixtures under `tests/fixtures/`
- Modify: `tests/contracts/repository.rs`
- Modify: `tests/contracts/release.rs`

- [ ] Add failing version consistency tests for Cargo metadata, lockfile,
  README, changelog, `ckc --version`, verbose ABI revisions, release tag rule,
  and archive metadata.
- [ ] Set `0.10.0` once all behavior exists. Add fixtures for every intentional
  compatibility change named by the design and preserve unaffected V0.9 source
  behavior.

### Task 8.3: Execute total acceptance and prepare review branch

**Files:**

- Modify only files required by failures that reveal a real implementation or
  contract defect.

- [ ] Execute [final acceptance](final-acceptance.md) from a clean worktree with
  fresh bootstrap evidence. Do not mark a remote-host item passed based on
  local inference.
- [ ] Run `git diff --check`, inspect the complete diff and commit graph, scan
  for placeholders/ignored tests/forbidden external tool calls, and confirm no
  generated LLVM build output is tracked.
- [ ] Commit the completed branch. Do not merge it, tag it, publish it, delete
  the worktree, or modify `main`; report the branch name, final commit, and
  remaining external CI evidence for owner review.
