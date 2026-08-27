# CalcKernel 0.10 Native Toolchain Final Acceptance

[简体中文](../../zh-CN/compiler/native-toolchain-implementation/final-acceptance.md)

This is the total release-candidate gate for the implementation governed by
[control](control.md). Every item is mandatory. A platform item is complete
only when its named host produced the evidence from the candidate commit.

## Candidate identity

- [ ] Worktree is clean on `feat/native-toolchain-0.10` and the candidate commit
  is recorded in the CI run.
- [ ] `main` has not moved as a result of this work; no merge, tag, GitHub
  Release, or publication has occurred.
- [ ] LLVM bootstrap manifest digest, LLVM 22.1.8 source checksum, runtime input
  hashes, bridge ABI, Native ABI revision, and runtime ABI revision are present
  in the build evidence.
- [ ] All eight stage gates have fresh passing evidence and no waiver changes a
  design requirement.

## Source quality and repository contract

Run in both the feature-disabled developer profile and the native-enabled
profile where applicable:

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked
cargo test --all-features --locked
cargo doc --all-features --no-deps
cargo build --release --locked
cargo build --release --features native-toolchain --locked
git diff --check
git status --short
```

- [ ] No production source invokes or probes Clang, an LLVM/LLD executable,
  platform linker, `ar`, or network downloader.
- [ ] No placeholder, `todo!`, `unimplemented!`, unexplained lint exception,
  ignored acceptance test, generated bootstrap output, or untracked required
  source remains.
- [ ] Every unsafe block documents its local invariant and is covered at its
  nearest safe boundary.
- [ ] English and Simplified Chinese Markdown trees mirror and all links resolve.

## Semantic differential matrix

For O0-O3 where meaningful and for all four checked-mode combinations:

- [ ] scalar integer, unsigned integer, f64, and bool operations;
- [ ] branches, loops, `break`, `continue`, void calls, and nested calls;
- [ ] struct fields and all target layout boundaries;
- [ ] raw pointers, slices, index, `.data`, `.len`, and sub-slices;
- [ ] overflow, division/modulo zero and minimum/-1 ordering;
- [ ] slice bounds and `start <= end <= len` ordering;
- [ ] entry void/i32 return and checked propagation;
- [ ] seven numeric print functions, exact ordering, formatting, failures, and
  forbidden-backend reachability.

Each row must agree between the structural LLVM path and the strict C/Clang
development oracle wherever the C contract defines the behavior. LLVM verifier
must pass before and after optimization for every fixture.

## Native ABI and artifact matrix

On each target below, test object, static, dynamic, and executable output under
an empty external-tool PATH. Compile generated headers and load dynamic
libraries through a system FFI harness.

| Target | Host runner | ABI family | ORC layer | Required |
| --- | --- | --- | --- | --- |
| `darwin-arm64` | macOS 15 AArch64 | Darwin AAPCS64 | JITLink | [ ] |
| `darwin-x64` | macOS 15 Intel | Darwin x86-64 | JITLink | [ ] |
| `linux-arm64` | Ubuntu 24.04 AArch64 | SysV AAPCS64 | JITLink | [ ] |
| `linux-x64` | Ubuntu 24.04 x86-64 | SysV AMD64 | JITLink | [ ] |
| `win32-arm64` | Windows 11 AArch64 | Windows ARM64 | RuntimeDyld | [ ] |
| `win32-x64` | Windows Server 2025 x86-64 | Windows x64 | JITLink | [ ] |

For every target:

- [ ] ABI classifier and generated thunk match pinned Clang fixtures.
- [ ] Baseline/native CPU policy and host-only target rejection pass.
- [ ] Dynamic library exports only requested CK symbols and required metadata.
- [ ] Dependency audit finds no CK, LLVM, ORC, LLD, Clang, formatting runtime,
  or non-system C++ runtime dependency.
- [ ] Executable and `ckc run` agree on stdout, stderr, normal/checked statuses,
  and numeric formatting.
- [ ] Cache miss/hit/bypass/corruption/permission/concurrency/eviction/clean pass.
- [ ] JIT permission behavior and instruction-cache finalization pass.

Darwin additionally requires runnable/loadable LLD ad-hoc-signed output and a
signed hardened `ckc run` test with only the approved JIT entitlement. Windows
requires computation DLL `/noentry`, import-library validation, and correct
exception mapping. Linux requires syscall-only runtime import evidence.

## Process, cache, and transactional failure injection

- [ ] Public `run` self-spawns only the same candidate binary's private child
  protocol; no persistent compiler process executes generated code.
- [ ] Signals or Windows exceptions map to exact `CKR0006`, while normal child
  and checked statuses remain unchanged.
- [ ] Successful run reserves stdout entirely for the CK program and emits no
  status message.
- [ ] Object cache canonical vectors cover every documented input; unsafe or
  corrupt entries become misses and never change execution semantics.
- [ ] Cache and output writes resist tested symlink, permission, and concurrent
  replacement cases.
- [ ] Injected pre-commit failure leaves every destination unchanged; injected
  multi-file commit failure restores backups or reports every unrecovered path.

## Runtime and security boundary

- [ ] Runtime performs no heap allocation and imports only approved stable OS
  process APIs where imports are necessary.
- [ ] `CKR0001` through `CKR0006` messages and statuses are byte exact.
- [ ] All numeric edge vectors pass, including shortest finite f64 round-trip,
  subnormal, infinity, NaN spelling, and negative zero.
- [ ] Linux and Windows demonstrate writable/non-executable relocation followed
  by read/execute code and non-executable data; Windows AArch64 covers the
  reserve-enabled RuntimeDyld path.
- [ ] Darwin demonstrates per-thread JIT write protection and does not falsely
  reject `MAP_JIT` maximum permissions.
- [ ] Raw-pointer and unchecked-code failures remain contained by the child
  process but are not described as memory safety or sandboxing.

## Performance gate

On controlled x86-64 and AArch64 workers, run strict equivalent native and
Clang C O3 suites for checked and unchecked modes separately.

- [ ] Reference sources, inputs, CPU features, floating-point rules, iteration
  counts, and output validation are identical.
- [ ] Native geometric-mean throughput is at least 95% of strict Clang C O3.
- [ ] No individual kernel is more than 10% slower without an approved and
  reproducible target limitation attached to the candidate evidence.
- [ ] Compilation latency, cold run, warm cache hit, peak memory, artifact size,
  and steady-state runtime are reported separately.

## Distribution and legal gate

- [ ] `ckc --version --verbose` reports compiler 0.10.0, LLVM 22.1.8, ABI
  revisions, target, backend, CPU policy, and active ORC layer on every target.
- [ ] `ckc licenses` contains notices for every embedded or statically linked
  third-party component and agrees with repository provenance hashes.
- [ ] Exactly the six existing archive names and six checksum sidecars are
  produced; each archive contains one complete native-enabled `ckc`.
- [ ] Archive extraction followed by tests succeeds without LLVM, Clang, LLD,
  linker, SDK lookup, runtime download, or first-run setup.
- [ ] Release workflow refuses partial artifact sets, checksum mismatch,
  version/tag mismatch, an existing GitHub Release, or dependency-audit failure.

## Contract freeze and handoff

- [ ] Cargo metadata, lockfile, READMEs, changelogs, CLI output, normative docs,
  compatibility fixtures, workflow tag logic, and archives all state 0.10.0.
- [ ] The standalone V0.9 LLVM exported-shape promise is retired and the single
  Native C ABI is authoritative without changing unaffected C/WASM behavior.
- [ ] The final branch commit contains all implementation and evidence changes,
  `git status --short` is empty, and the complete diff has been reviewed.
- [ ] The branch and worktree remain unmerged and available for owner review.

Passing this document authorizes reporting the review candidate as complete;
it does not authorize merging, tagging, pushing a release, or deleting the
worktree.
