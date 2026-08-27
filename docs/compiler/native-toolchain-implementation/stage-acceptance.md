# CalcKernel 0.10 Native Toolchain Stage Acceptance

[简体中文](../../zh-CN/compiler/native-toolchain-implementation/stage-acceptance.md)

This document is the mandatory exit gate for each stage in
[stage tasks](stage-tasks.md). Run commands from the repository root. A check
passes only from fresh command output against the current commit; an earlier
run, an inferred platform result, or an ignored test is not evidence.

## Common gate for every stage

For a non-native stage or a feature-disabled compatibility pass:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo build --release --locked
git diff --check
```

For a native-enabled stage, set `CKC_LLVM_PREFIX` to the checksum-verified
22.1.8 bootstrap for the current host and also run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-features --locked
cargo build --release --features native-toolchain --locked
```

No command may access an unpinned system LLVM. Test and release logs must print
the bootstrap manifest digest and bridge-reported LLVM version.

## Stage 1 exit — dependency and bridge

Required commands:

```bash
cargo test --locked --test contracts native_toolchain
cargo test --all-features --locked --test native bridge
cargo test --all-features --locked --test native ownership
./target/release/ckc --version --verbose
./target/release/ckc licenses
```

Acceptance conditions:

- feature-disabled build/test succeeds without `CKC_LLVM_PREFIX`;
- feature-enabled build rejects missing, wrong-version, or shared-only prefix;
- valid build reports LLVM 22.1.8, the normalized host, correct code generator,
  and JITLink or the Windows AArch64 RuntimeDyld layer;
- exception injection and repeated lifetime tests show no unwind, leak,
  double-free, or stale handle under the platform sanitizer configuration;
- the linked `ckc` has no dynamic LLVM/LLD/Clang dependency;
- embedded notices are available when all external files are absent.

## Stage 2 exit — source and MIR semantics

Required commands:

```bash
cargo test --locked --test frontend
cargo test --locked --test ir
cargo test --locked --test optimizer
cargo test --locked --test backend control_void_slice
```

Acceptance conditions:

- only parameterless, non-exported `main -> void|i32` is accepted as an entry;
- all seven print builtins have exact signatures and cannot be redeclared;
- MIR makes print effects and entry metadata explicit and validates them;
- O0-O3 preserve print count and source order through calls, loops, and inline;
- root analysis rejects reachable print for C/WASM/native non-executable
  artifacts before output, while allowing it for run/executable;
- all pre-existing frontend, MIR, optimizer, C, and WebAssembly tests pass.

## Stage 3 exit — structural LLVM and object code

Required commands:

```bash
cargo test --all-features --locked --test backend llvm
cargo test --all-features --locked --test native llvm_ir
cargo test --all-features --locked --test native object
cargo test --all-features --locked --test cli emit_llvm
```

Acceptance conditions:

- every representative fixture verifies before and after PassBuilder;
- `emit-llvm` is LLVM's rendering of the same structural module used for object
  emission and rejects non-host target before writing;
- O0-O3 select matching MIR/LLVM pipelines and O3 contains no fast-math flags;
- baseline objects use only the documented mandatory ISA and native objects
  record the detected complete CPU feature selection;
- all four checked-mode combinations have verified status CFGs, first-error
  order, and expected guard absence/presence; executable differential evidence
  is deliberately owned by stages 4-6;
- product source and executable contain no Clang probe, invocation, or fallback.

## Stage 4 exit — ABI and libraries

Required commands on every release host:

```bash
cargo test --all-features --locked --test native abi
cargo test --all-features --locked --test native artifacts
cargo test --all-features --locked --test native libraries
cargo test --all-features --locked --test native differential
cargo test --all-features --locked --test cli build
```

Acceptance conditions:

- the host's ABI classifier matches pinned Clang 22 fixtures for all exported
  shapes and checked results;
- generated headers compile as C11 and describe the actual exported thunks;
- object, static, and dynamic outputs validate and use correct platform names;
- dynamic libraries load through system FFI under an empty tool PATH and every
  exported shape works in unchecked and checked modes;
- exported scalar, control-flow, void, call, struct, pointer, slice, and checked
  fixtures match the separate pinned Clang 22 oracle libraries;
- LLD sees only compiler-produced and compiler-owned inputs;
- injected pre-commit and commit failures prove no partial file and successful
  rollback semantics;
- `build` defaults to dynamic, all four kinds parse, and `build-llvm` emits one
  deprecation warning only for its supported compatibility forms.

## Stage 5 exit — runtime and executables

Required commands on every release host:

```bash
cargo test --all-features --locked --test native runtime
cargo test --all-features --locked --test native executable
cargo test --all-features --locked --test native artifacts
./scripts/audit-native-artifact.sh target/native-acceptance
```

Use the PowerShell audit on Windows.

Acceptance conditions:

- all numeric spellings, newlines, runtime messages, and statuses are byte
  exact; finite f64 output round-trips and `-0.0` is preserved;
- stdout failure produces `CKR0005` behavior without heap or formatting runtime;
- void/i32 and checked main wrappers agree with the contract;
- executables run under an empty external-tool PATH and need no CK, LLVM, LLD,
  Clang, libc formatting, or external compiler runtime;
- objects, archives, libraries, executables, import metadata, runtime objects,
  and compiler helpers pass provenance and dependency audits;
- Darwin output is ad-hoc signed and loadable/runnable with the declared
  platform version; Windows computation DLLs have no runtime entry point.

## Stage 6 exit — run, process, cache, and JIT protection

Required commands on every release host:

```bash
cargo test --all-features --locked --test native jit
cargo test --all-features --locked --test native run
cargo test --all-features --locked --test native cache
cargo test --all-features --locked --test cli run
./scripts/audit-jit-memory.sh target/release/ckc
```

Use the PowerShell audit on Windows.

Acceptance conditions:

- ORC eagerly executes the same O3 object used for AOT and resolves all symbols
  before entry; the reported object layer is platform-correct;
- public parent/private child behavior, stdio ownership, interrupts, normal
  statuses, checked failures, and exact `CKR0006` mapping pass;
- a successful run writes no compiler status text;
- cold miss, warm hit, bypass, corruption, permission, symlink, concurrent
  writer, atomic store, eviction, and clean cases preserve program semantics;
- cache key vectors cover every object-affecting input and are stable across
  process runs;
- Linux/Windows prove RW-to-RX code and NX data, including Windows AArch64;
  Darwin proves thread-level JIT write protection under signed release policy.

## Stage 7 exit — performance and distribution

Required controlled-host commands:

```bash
cargo bench --features native-toolchain --bench ckc_perf
python3 scripts/check-native-performance.py target/ckc-perf/results.json
cargo test --locked --test contracts ci
cargo test --locked --test contracts release
cargo test --locked --test contracts native_toolchain
```

Acceptance conditions:

- strict native O3 geometric-mean throughput is at least 95% of equivalent
  strict Clang C O3 on controlled x86-64 and AArch64 hosts;
- no individual kernel is more than 10% slower without an approved,
  reproducible target limitation in release evidence;
- checked and unchecked results are gated separately and every reference uses
  equivalent CPU and floating-point semantics;
- required native integration covers all six hosts, while the fast job remains
  usable without LLVM bootstrap;
- six archives retain their existing names and checksum sidecars, contain one
  functional native-enabled `ckc`, expose notices, pass dependency audits, and
  are published only as a complete immutable set.

## Stage 8 exit — contract freeze

Required commands:

```bash
cargo test --locked --test contracts
cargo test --locked
cargo test --all-features --locked
cargo doc --all-features --no-deps
./target/release/ckc --version --verbose
./target/release/ckc licenses
```

Acceptance conditions:

- Cargo, lockfile, CLI, READMEs, changelogs, release tag rule, ABI revisions,
  and current bilingual documentation consistently state 0.10.0;
- every intentional compatibility change has a fixture and unaffected V0.9
  source remains compatible;
- English and Simplified Chinese trees mirror, all local links resolve, and
  no superseded promise remains normative;
- no placeholder, ignored acceptance test, native external-tool invocation,
  untracked generated build product, or unrelated worktree change remains;
- [final acceptance](final-acceptance.md) is ready to run from a clean commit.
