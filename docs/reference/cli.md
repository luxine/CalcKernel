# `ckc` 0.10 CLI Reference

[简体中文](../zh-CN/reference/cli.md)

This document defines the native `ckc` command surface. Success exits 0;
usage, source, filesystem, unsupported-mode, toolchain, backend, and runtime
failures exit nonzero. Errors and diagnostics use stderr; successful status and
requested textual output use stdout unless stated otherwise.

## Commands

| Command | Result |
| --- | --- |
| `ckc check <file>` | Parse and type-check; no artifact. |
| `ckc emit-mir <file>` | Deterministic MIR on stdout or in `--out`. |
| `ckc emit-c <file> --out <file.c>` | C source and sibling or explicit header; source-only. |
| `ckc emit-wat <file>` / `emit-wasm` | Textual or binary WebAssembly. |
| `ckc emit-llvm <file>` | Verified textual LLVM IR for the host triple. |
| `ckc build <file> --out <path>` | In-process Native build. |
| `ckc build-llvm <file> --out <path>` | Deprecated alias for Native dynamic/object build. |
| `ckc run <file>` | Compile and execute `main` in an isolated child. |
| `ckc cache clean` | Remove only the resolved CK native cache. |
| `ckc licenses` | Print embedded third-party notices. |
| `ckc --version --verbose` | Print compiler, ABI, LLVM, target, codegen, and ORC identity. |

`build` accepts `--kind executable|dynamic|static|object`; omission means
`dynamic`. Object, static, and dynamic outputs receive a sibling Native C ABI
header. Windows dynamic output also receives an import library. Executables do
not receive a header. Output sets are staged before destination replacement.
Object suffixes are `.o`/`.obj`, static libraries `.a`/`.lib`, and dynamic
libraries `.so`/`.dylib`/`.dll`. A pre-commit failure leaves every destination
unchanged; commit-time multi-file failure restores same-filesystem backups or
reports each path it could not recover.

The compiler invokes LLVM 22.1.8 and LLD in process. Product commands do not
discover or spawn external Clang, linkers, or archivers, and native builds leave
no `.c` or `.ll` intermediate. `emit-c` never compiles or links its output.

## Options and defaults

- `--out <file>` and `-o <file>` select output; `--header` selects a C header.
- `--overflow unchecked|checked` and `--bounds unchecked|checked` default to
  unchecked.
- `--opt-level 0|1|2|3` and `-O0` through `-O3` select one MIR/LLVM level.
  `run`, `build`, and `build-llvm` default to O3; inspection commands to O0.
- `--cpu baseline|native` applies to `build`; baseline is the portable default.
  `run` uses the host CPU.
- `--target <host-triple>` is accepted by Native inspection/build commands only
  when it normalizes to the detected host triple; cross-compilation is rejected.
- `--no-cache` makes `run` bypass persistent cache reads and writes.
- MIR pass-printing flags write diagnostics to stderr.

`CKC_LLVM_PREFIX` selects the pinned developer LLVM installation when building
`ckc` from source. It is a compiler build-time input, not a runtime dependency
of a release binary.

## Backend and effect matrix

| Surface | Overflow checked | Bounds checked | Reachable Native print |
| --- | --- | --- | --- |
| Native `run` / executable | accepted | accepted | accepted |
| Native dynamic/static/object | accepted | accepted | rejected from exports |
| C `emit-c` | accepted | accepted | rejected from exports |
| WASM | rejected | rejected | rejected from exports |
| `check` / `emit-mir` | recorded, backend-independent | recorded | accepted source model |

Unsupported combinations are rejected before artifact creation. `emit-llvm`
uses Native lowering, accepts all four checked combinations, and is an
inspection artifact rather than a standalone public ABI.

## Cache and failures

The run cache is keyed by exact source, compiler/Native ABI/runtime ABI/LLVM
identities, target, complete CPU features, optimization, checked modes, and all
object-affecting options. Entries contain a versioned manifest, object, and
SHA-256 integrity digest. `--no-cache` bypasses reads and writes. Corruption,
unsafe ownership/permissions, a symlink replacement, or an unparseable object
is a miss, never executable input. The same-user cache remains inside the
user's trust boundary and is not a security sandbox.

Roots are `$XDG_CACHE_HOME/ckc` or `$HOME/.cache/ckc` on Linux,
`$HOME/Library/Caches/ckc` on macOS, and
`%LOCALAPPDATA%\CalcKernel\cache` on Windows. A missing required base disables
cache for the run. Writes use owner-only same-filesystem staging and atomic
rename; the default soft limit is 1 GiB with best-effort LRU eviction.

Native checked failures use statuses 240–243; stdout failure uses 244; abnormal
child termination maps to 245. See [Checked modes](../abi/modes.md).
