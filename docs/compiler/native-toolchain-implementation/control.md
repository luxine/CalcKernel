# CalcKernel 0.10 Native Toolchain Implementation Control

[简体中文](../../zh-CN/compiler/native-toolchain-implementation/control.md)

> **For agentic workers:** REQUIRED SUB-SKILL: Use
> `superpowers:executing-plans` to execute this plan task by task. Track every
> task with its checkbox and execute it in line; this implementation does not
> permit delegated or sub-agent execution.

**Goal:** Deliver the approved self-contained CalcKernel 0.10 native toolchain
on the six release hosts without weakening the language, Native C ABI,
runtime, performance, or release gates.

**Architecture:** Preserve the current frontend, MIR, optimizer, C backend,
WebAssembly backend, thin CLI, and responsibility-based test layout. Replace
the textual-only LLVM path with one structural LLVM module builder that feeds
verification, PassBuilder, TargetMachine, ORC, archive writing, LLD, and
`emit-llvm`. Keep the LLVM/LLD C++ surface behind a narrow C ABI. Keep runtime
and platform link inputs compiler-owned and embedded in the release binary.

**Tech Stack:** Rust 2024, LLVM/ORC/LLD 22.1.8, a narrow C++20 bridge, CMake and
Ninja for the pinned native dependency build, GitHub Actions, platform object
and dependency inspection tools, and pinned Clang only as a development
oracle.

---

## Authority and scope

The approved [native toolchain design](../native-toolchain-design.md) is the
semantic and architectural authority. These implementation documents refine
that design into executable work; they cannot expand or weaken it. If code
reveals a genuine contradiction, revise the design and both language versions
first, record why the previous requirement was impossible or unsafe, and then
repeat the affected acceptance gate. A failing implementation is not grounds
for lowering a gate.

The direct implementation authorization for this work requires the control,
execution, and acceptance documents to be committed even though the ordinary
repository convention keeps transient plans local. These files are therefore
maintainer-facing execution contracts for the 0.10 line, not dated review
notes or historical narratives. Remove or convert them only as part of the
0.10 contract freeze.

The work is performed on `feat/native-toolchain-0.10` in the isolated
`.worktrees/native-toolchain-0.10` worktree. No stage may merge, tag, publish,
or modify `main`. The completed branch remains available for owner review.

## Non-negotiable execution rules

- Work in the stage order below. A later stage may start only after the current
  stage acceptance commands pass with fresh output.
- Use test-driven development for every behavior: add the smallest meaningful
  failing test, observe the expected failure, implement the minimum production
  change, observe the test pass, then refactor under green tests.
- Never make production code depend on Clang, `clang`, `llvm-config`, LLD
  executables, a platform linker, `ar`, or first-run network downloads.
- A repository bootstrap may execute build tools while producing `ckc`; a
  released `ckc` may not execute or load those tools at runtime.
- Keep `src/bin/ckc.rs` thin. Compiler work stays in `src/frontend`, `src/ir`,
  `src/optimizer`, `src/backend`, and `src/cli`; native bridge and embedded
  runtime sources stay under `native/`.
- Do not use `unwrap` or `expect` in production paths unless failure is proven
  impossible by a local invariant. Every `unsafe` block requires a precise
  safety justification and a focused test at the nearest safe boundary.
- Do not leave placeholders, ignored acceptance tests, disabled gates, broad
  lint suppressions, or untracked work products.
- Commit only after the stage gate passes. Use focused stage commits and keep
  the worktree clean between stages.

## Build profiles and dependency boundary

The crate exposes a `native-toolchain` Cargo feature. Ordinary frontend,
MIR, C, WebAssembly, and contract tests build without that feature and do not
bootstrap LLVM. A native-enabled source build requires `CKC_LLVM_PREFIX` to
point at the repository bootstrap install for exactly LLVM 22.1.8. `build.rs`
validates the version and statically links only the host components named by
the bootstrap manifest. It never silently accepts a system LLVM.

Release archives are always built with `--features native-toolchain`; a binary
without the feature is a developer compiler and must reject `run`, native
`build`, and native `emit-llvm` with one explicit availability error. It still
supports `check`, `emit-mir`, `emit-c`, `emit-wat`, and `emit-wasm`. No archive
or published release may contain the developer-only form.

The repository owns:

- `native/llvm/manifest.toml`: LLVM tag, source URL, archive SHA-256, CMake
  switches, target-specific component allowlists, and notice inputs;
- `scripts/bootstrap-llvm.sh` and `scripts/bootstrap-llvm.ps1`: deterministic,
  checksum-verified host bootstrap into an explicit prefix;
- `native/bridge/`: the exception-contained C++ bridge and C header;
- `native/runtime/`: no-heap runtime, entry objects, platform import metadata,
  export lists, source provenance, hashes, and licenses.

The manifest also defines a separate `oracle` bootstrap profile that builds the
Clang 22 driver from the same checksum-verified source for required ABI,
differential, and performance tests. Its explicit `CKC_CLANG_ORACLE` path is
accepted only by test/benchmark support. It is never searched on `PATH`, linked
into `ckc`, copied to a release prefix, or needed by a product command. The
release profile continues to assert that Clang is excluded.

Bootstrap outputs live under ignored `build/` and are never committed. The
bootstrap accepts an already-downloaded source archive for offline builds;
network access is an explicit developer/CI acquisition step, never implicit in
Cargo or in `ckc`.

## Repository mapping

| Responsibility | Existing anchor | 0.10 destination |
| --- | --- | --- |
| Source rules and builtins | `src/frontend/typeck.rs` | entry validation and seven reserved print symbols |
| MIR effects and roots | `src/ir/model.rs`, `src/ir/lower.rs`, optimizer passes | print effects, entry/library reachability, preservation |
| LLVM lowering | `src/backend/llvm/` | structural builder, checked lowering, ABI thunks, target/object/JIT owners |
| C ABI authority | `src/backend/c/layout.rs`, C header emitter | shared target ABI model and native header generation |
| Artifact assembly | `src/cli/commands.rs`, `src/cli/output.rs` | backend artifact API plus transactional multi-output commit |
| Run/cache/process | `src/cli/` | parent/child protocol, cache, signal/exception mapping |
| Native foreign boundary | none | `native/bridge/` and safe Rust wrappers |
| Runtime/link inputs | none | `native/runtime/` embedded by the native build |
| Integration evidence | responsibility tests in `tests/` | `tests/native/`, extended CLI/backend/contracts/performance suites |
| Release evidence | two GitHub workflows | pinned native integration and six-host release jobs |

The existing C layout and checked-status behavior are test oracles, not code to
copy blindly. Shared ABI concepts should be extracted only when both backends
need the same invariant; backend-specific emission remains separate.

## Stage graph

Stages are intentionally sequential because each consumes an artifact or
contract stabilized by the previous stage.

| Stage | Deliverable | Entry dependency | Exit evidence |
| ---: | --- | --- | --- |
| 1 | pinned LLVM bootstrap, bridge, safe ownership wrappers | V0.9 baseline | bridge smoke tests and static component audit |
| 2 | `main`, print builtins, MIR effects and artifact roots | stage 1 types available | frontend/MIR/optimizer semantic suite |
| 3 | structural LLVM lowering, verification, optimization and codegen | stages 1-2 | structural IR, object validity, and checked-CFG suite |
| 4 | Native C ABI thunks and object/static/dynamic builds | stage 3 object emission | six-family ABI oracle and executable library differential suite |
| 5 | minimal runtime and standalone executables | stage 4 LLD/artifact assembly | run-equivalent executable suite and dependency audit |
| 6 | ORC child execution and persistent cache | stage 5 runtime object | process, cache, memory-protection and output suite |
| 7 | performance, CI, release packaging and notices | stages 1-6 | controlled performance and six-host release matrix |
| 8 | 0.10 contract freeze | all implementation stages | complete final acceptance and version consistency |

The exact red/green work units and file lists are in
[stage tasks](stage-tasks.md). Stage exit commands and required evidence are in
[stage acceptance](stage-acceptance.md). The branch is finished only after
[final acceptance](final-acceptance.md) passes.

## Stable internal boundaries

The implementation establishes these internal APIs before adding higher-level
behavior:

- `backend::llvm::NativeToolchain`: a host-only compiler owner with explicit
  context/module/target/JIT lifetimes and typed errors;
- `backend::llvm::CodegenOptions`: optimization, checked modes, CPU policy,
  artifact intent, and host triple, with no free-form target or linker flags;
- `backend::llvm::VerifiedModule`: constructible only after LLVM verification;
- `backend::llvm::OptimizedModule`: constructible only by running the selected
  PassBuilder pipeline and re-verifying;
- `backend::llvm::NativeObject`: verified object bytes plus target and ABI
  metadata, never an arbitrary user object;
- `backend::native_abi`: the explicit target-family classifier and header
  contract shared by export thunks and integration fixtures;
- `backend::artifact`: object/archive/LLD assembly from compiler-owned inputs;
- `cli::run`: public parent and private child protocol;
- `cli::cache`: canonical key, validated entry, atomic store, eviction, and
  clean operations.

Opaque LLVM and ORC pointers remain private. Safe wrappers are non-`Clone`,
carry the ownership relationship in their types, and release in reverse order.
The C++ bridge catches every exception and returns an owned error message
through a paired release function. Rust never unwinds across the bridge.

## Evidence and commit discipline

For every red/green task, preserve the command and the reason for the red result
in the local execution log. The committed evidence is the test, not a transcript.
At the end of each stage run its focused gate, then:

```text
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
git diff --check
```

Native stages additionally run the same checks with `--features
native-toolchain` and the pinned prefix. `cargo clippy --all-features` is owned
by native integration CI, where the prefix exists; the fast job must not pretend
to validate an unavailable native bridge.

Use commit subjects `native(stage-N): <completed capability>`. Documentation
changes that repair a real contract issue are committed before the code that
depends on them. The final commit contains only integration/freeze changes, not
unrelated cleanup.

## Stop conditions

Stop implementation and repair the governing documents when any of these is
observed:

- an approved behavior cannot be implemented on one of the six hosts with the
  pinned LLVM/ORC/LLD interfaces;
- a platform requires a runtime or SDK dependency forbidden by the design;
- generated Native ABI behavior cannot be reconciled with the documented C ABI;
- W^X, signing, cache ownership, transactional output, or abnormal-child rules
  cannot be proven by an automated platform test;
- a performance result fails the gate after benchmark noise and reference
  equivalence have been ruled out.

A missing local release host is not permission to waive evidence. Host-specific
acceptance remains pending until the required CI worker reports it.
