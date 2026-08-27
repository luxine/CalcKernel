# CalcKernel 0.10 Native Toolchain Design

[简体中文](../zh-CN/compiler/native-toolchain-design.md)

This document is the approved implementation design for the CalcKernel 0.10
native toolchain. It is forward-looking: the V0.9 reference and ABI documents
remain authoritative until the implementation, tests, migration notes, and
0.10 release contract land together.

## Goals and fixed decisions

CalcKernel 0.10 adds a self-contained optimizing native toolchain while keeping
the C and WebAssembly source emitters. Each supported release contains one
functional `ckc` executable with a pinned LLVM, ORC, and LLD linked into it.
Users do not install Clang, LLVM, LLD, a platform linker, or a CK runtime.

The first release supports host-native compilation and execution on all six
existing release targets:

- macOS AArch64 and x86-64;
- Linux AArch64 and x86-64;
- Windows AArch64 and x86-64.

Cross-OS and cross-architecture linking are not part of 0.10. `ckc run` always
targets the current CPU. `ckc build` targets the current OS and architecture,
uses a portable CPU baseline by default, and accepts `--cpu native` for an
artifact tied to the build machine's available CPU features.

The native runtime does not add a heap, allocator, ownership system, strings,
command-line arguments, standard input, modules, or a sandbox. Generated FFI
libraries remain computation-only and caller-owned memory remains the language
contract.

## Compiler architecture

```text
CK source
  -> frontend and type checking
  -> MIR lowering and validation
  -> CK MIR O0-O3
  |  -> C emitter
  |  -> WAT/WASM emitter
  `  -> structural LLVM module builder
       -> Native ABI export thunks
       -> LLVM verifier
       -> LLVM PassBuilder O0-O3
       |  -> ORC/JITLink
       `  -> TargetMachine -> object -> archive writer or LLD
```

The native backend constructs an in-memory LLVM module. It does not print LLVM
text and parse it back. `emit-llvm` prints that same verified module, so textual
IR and native code cannot diverge through separate lowerings.

The Rust implementation owns a narrow internal C-compatible boundary to LLVM.
It uses the LLVM C interfaces where they cover Core IR, PassBuilder, targets,
and ORC, with a small version-pinned C++ shim only where LLD or ORC require it.
C++ objects and exceptions never cross into Rust. LLVM verification failure is
an internal compiler error and no artifact is committed.

LLVM is pinned to 22.1.8 for the 0.10 line. The source tag and archive checksum
are repository-controlled inputs. A patch-line upgrade requires the full
native semantic, ABI, performance, and release suites; a major LLVM upgrade is
a versioned compiler change, never an incidental dependency update.

Release builds include only the host code generator, object format, ORC/JITLink
support, and LLD driver needed by that release target. They do not include the
Clang frontend or unrelated LLVM targets. LLVM, LLD, and their non-system C++
runtime are statically linked; optional LLVM dependencies that would add a
target-machine shared-library requirement are disabled.

## Source entry point

`main` is a reserved program entry name with exactly these accepted forms:

```ck
fn main() -> void
fn main() -> i32
```

It takes no parameters and may not be declared `export`. `ckc run` and
`build --kind executable` require one valid `main`. A void `main` produces
process status zero; an i32 result is passed to the platform exit facility.
Portable applications use exit values 0 through 239. Values outside the
platform's observable exit range retain platform exit semantics.

Library and object builds do not expose `main`; it remains internal and is
removed when unreachable. C and WebAssembly may lower a valid `main` as an
ordinary internal function, but those backends do not create a native process
entry.

## CLI contract

The native build surface is unified:

```text
ckc run <file.ck> [-O0|-O1|-O2|-O3]
    [--overflow unchecked|checked]
    [--bounds unchecked|checked]
    [--no-cache]

ckc build <file.ck> --kind executable|dynamic|static|object --out <path>
    [-O0|-O1|-O2|-O3]
    [--overflow unchecked|checked]
    [--bounds unchecked|checked]
    [--cpu baseline|native]
```

Omitting `--kind` continues to mean `dynamic`, preserving the V0.9 command
default. `build-llvm` remains in 0.10 as a deprecated alias for the dynamic and
object native build forms and writes one migration warning to stderr. It is not
an independent backend.

`run` and `build` default to O3. One optimization selection controls both the
CK MIR pipeline and the LLVM default pipeline. `check` and `emit-*` retain an
O0 default. O3 preserves CK strict floating-point semantics and never implies
LLVM fast-math flags. A fast floating-point mode is outside this design.

`run` always uses the detected host CPU and features. `build` defaults to the
documented baseline for its release target; `--cpu native` opts into host
features. Native `build` and `emit-llvm` reject a non-host target triple before
creating any artifact. This keeps the printed module, target DataLayout, and C
ABI thunks inside the same verified host-native contract.

The CPU baselines are LLVM `x86-64` with the architecture-mandated SSE2 feature
for all x86-64 releases, and generic ARMv8-A with the ABI-mandated FP/Advanced
SIMD feature set for all AArch64 releases. Baseline artifacts may not acquire a
new optional ISA feature merely because the compiler ran on a newer host.

`emit-c` continues to emit C and a header without invoking a compiler. The
product CLI contains no external Clang discovery, subprocess, or automatic
fallback. A developer-installed Clang may be used only by repository oracle
and benchmark tests.

## Build artifacts

`object` writes `.o` on ELF/Mach-O hosts and `.obj` on Windows. `static` writes
`.a` or `.lib`. `dynamic` writes `.so`, `.dylib`, or `.dll`; a Windows dynamic
build also writes its import `.lib`. `executable` writes the platform-native
executable form.

Object, static, and dynamic builds emit a sibling C ABI header. Executable
builds do not. A Windows dynamic header marks exports as `dllimport` for the
consumer; object and static headers define `CK_API` without DLL storage class.
All outputs are staged and validated before destination replacement begins.
Each destination is replaced atomically as an individual file, so a partially
written object, library, executable, header, or import library is never
exposed. A pre-commit failure leaves every existing destination untouched. For
multi-file outputs, commit-time failure triggers rollback from same-filesystem
backups and reports the affected paths; an unclean process or OS failure may
leave complete old and new files side by side, because ordinary filesystems do
not provide a portable multi-file transaction. Release packaging and other
crash-consistent consumers build into a fresh directory and publish that
directory only after `ckc` succeeds.

TargetMachine emits object bytes directly. The archive writer packages native
objects without a platform `ar` command. LLD is invoked as an in-process library
behind the internal FFI boundary. It links only compiler-produced, verified
objects; user-supplied arbitrary objects are not accepted in 0.10.

## Native FFI ABI

The documented C ABI becomes the only public Native ABI. LLVM IR types are an
internal representation and may not leak into an exported signature. Every
`export fn` receives an external C ABI thunk; the optimized CK implementation
behind the thunk may use a different internal signature.

The thunks preserve all existing commitments:

- fixed-width integer and strict `double` mappings;
- target C `_Bool` parameter, result, and stored-field representation;
- declaration-order C struct layout and target padding/alignment;
- flattened slice parameters `(T* data, uint32_t len)`;
- direct unchecked returns and C `void`;
- module-wide checked status returns and result out-pointers;
- source symbol names, default visibility, and Windows DLL exports.

Source-level aggregate ABI lowering is a frontend responsibility, not something
LLVM infers from a named IR struct. The native backend therefore owns explicit
C ABI classifiers for these target families:

- SysV AMD64 for Linux x86-64;
- Darwin x86-64;
- AAPCS64 variants for Linux and Darwin AArch64;
- Windows x64 and Windows ARM64.

Each classifier determines register classes, indirect/by-value parameters,
small aggregate returns, alignment, extension attributes, and hidden result
pointers. Its fixtures are compared against the same pinned Clang major during
development. Those tests are compiler-development oracles; Clang is not a
release or user dependency.

An export thunk is added before LLVM O3. LLVM may inline the CK implementation
into the thunk, so the ABI boundary need not add an internal call. FFI callers
still pay one ordinary native C call, the same boundary cost as a comparable C
library. The generated header remains the authority for consumers.

## Checked modes

The native backend supports all four combinations of overflow and slice-bounds
modes. Both defaults remain unchecked. Selecting either checked mode enables
the existing module-wide `CK_Status` ABI and preserves its error ordering.

LLVM lowering uses overflow intrinsics, explicit division guards, slice guards,
and status propagation. It does not implement checked behavior with traps.
Unchecked lowering retains current language semantics and does not add guards.

For a checked program entry, the generated entry wrapper supplies a valid
result pointer when needed. `CK_OK` uses the source `main` result. A propagated
checked failure ignores the unwritten result, writes one fixed English runtime
diagnostic to stderr, and exits with the reserved status below:

| Runtime ID | Condition | Process status |
| --- | --- | ---: |
| `CKR0001` | integer overflow | 240 |
| `CKR0002` | integer division or modulo by zero | 241 |
| `CKR0003` | null checked result pointer | 242 |
| `CKR0004` | slice index or sub-slice out of bounds | 243 |
| `CKR0005` | standard-output write failure | 244 |
| `CKR0006` | abnormal native child termination | 245 |

The IDs, English messages, and process statuses are stable 0.10 runtime
contract. They are distinct from source diagnostics. Application code should
not use statuses 240 through 245 for portable process-level signaling.

The exact diagnostic byte strings are UTF-8/ASCII and end in one LF byte:

```text
CKR0001: integer overflow
CKR0002: integer division or modulo by zero
CKR0003: null checked result pointer
CKR0004: slice index or sub-slice out of bounds
CKR0005: standard output write failed
CKR0006: native child terminated abnormally
```

`CKR0005` is attempted on stderr after stdout fails; failure of that diagnostic
write does not change status 244. `CKR0006` is emitted only by the `ckc run`
parent and never replaces a more specific normal child status.

## Minimal native runtime

The runtime is a small, no-heap, host-specific object embedded as bytes in the
matching `ckc` release. `run` JIT-links it; executable builds link the same
object with LLD. It provides entry/exit glue, runtime diagnostics, and these
reserved, predeclared LLVM-native-only functions:

```ck
print_i32(value: i32) -> void
print_i64(value: i64) -> void
print_u32(value: u32) -> void
print_u64(value: u64) -> void
print_f64(value: f64) -> void
print_bool(value: bool) -> void
print_newline() -> void
```

Value functions do not append a newline. `print_newline` emits one LF byte on
every platform. Integer output is base-10 with no locale, grouping, leading
zeros, or positive sign. Boolean output is `true` or `false`. Finite f64 output
uses a no-allocation shortest-round-trip decimal algorithm under round-to-
nearest, ties-to-even. It preserves negative zero as `-0.0`; special spellings
are `nan`, `inf`, and `-inf`. NaN payload and sign are not represented.

Each call formats into a bounded stack buffer and completes its output or exits
with `CKR0005`. Linux uses the supported kernel write/exit boundary; Darwin and
Windows use their stable OS-provided process APIs through minimal embedded
import metadata. No libc formatting, locale, heap, Rust runtime, or CK dynamic
runtime is linked into the generated executable.

Print calls are observable side effects. MIR and LLVM optimization may not
remove, duplicate, combine, hoist, sink, or reorder them relative to source
evaluation order. They remain void runtime intrinsics even when a checked mode
enables the module-wide status ABI; output failure terminates the process rather
than returning `CK_Status`.

The print functions may appear in `emit-mir` and `emit-llvm` inspection output,
but are linkable only by `run` and executable builds. Non-executable native
builds reject a print call reachable from an artifact root. C and WebAssembly
emission reject any print builtin before writing an artifact. This prevents an
FFI library from terminating its host on output failure and keeps generated
libraries computation-only.

## Zero-dependency library guarantee

Native object, static, and dynamic outputs contain no dependency on CK, LLVM,
ORC, LLD, Clang, libc formatting, or an external compiler runtime. LLVM is a
compiler implementation detail and is not linked into user artifacts.

If target lowering introduces a compiler helper, its matching permissively
licensed implementation is linked into the artifact statically. A dynamic
library may rely on the platform loader but has no CK/LLVM runtime import. The
release suite inspects ELF `DT_NEEDED`, Mach-O load commands, and PE imports and
rejects an unexpected dependency. Windows computation-only DLLs use no runtime
entry point.

Libraries expose only requested CK exports and required ABI metadata. Memory
for pointers and slices remains allocated, aligned, and owned by the caller.
Static archives and objects naturally require the consuming language's link
step, but add no runtime library requirement after that link.

## `ckc run` process and cache model

The public `ckc run` process starts the same executable in a private child mode.
The child performs compilation, cache lookup, JIT linking, and `main` execution.
The parent forwards stdout and stderr, returns the child status, forwards user
interrupts, and translates identifiable signals or Windows exceptions into
`CKR0006`. This is process isolation, not a security sandbox.

The child uses eager ORC compilation at O3. Once execution starts there is no
interpreter loop and no lazy-compile stub in a hot CK function. Steady-state
code is ordinary native machine code.

The persistent object cache is enabled by default. A cache entry name is the
lowercase hexadecimal SHA-256 digest of a canonical, versioned serialization
of a key covering:

- exact source bytes and compiler version;
- runtime and Native ABI revisions;
- LLVM version and target triple;
- MIR/LLVM optimization level;
- overflow and bounds modes;
- host CPU name and complete feature set;
- every code-generation option that affects object bytes.

Cache entries contain a manifest, object bytes, and a SHA-256 integrity digest
over both. Writes use an owner-only same-filesystem temporary file and atomic
rename. The cache root and entries must be owned by the current OS identity and
must not be writable by other identities. Invalid ownership, permissions,
manifest, digest, or object parsing turns the entry into a miss and never makes
a valid source build fail. The digest detects corruption, not malicious changes
made with the same OS credentials: cache content is inside the user's trust
boundary, just like source and compiler configuration. `--no-cache` is required
when that boundary is not trusted. The default soft limit is 1 GiB with
best-effort least-recently-used eviction. `--no-cache` bypasses reads and
writes, and `ckc cache clean` removes only the resolved CK cache directory.

The resolved directories are `$XDG_CACHE_HOME/ckc` or `$HOME/.cache/ckc` on
Linux, `$HOME/Library/Caches/ckc` on macOS, and
`%LOCALAPPDATA%\CalcKernel\cache` on Windows. A missing required base directory
disables caching for that run instead of inventing a process-wide writable
location.

Cache content and eviction order are not compatibility commitments. A cache hit
must produce the same output and runtime behavior as a clean compilation.

## Performance contract

The reference comparison is the same CK source emitted through the C backend
and compiled by pinned Clang at strict `-O3`, using the same CPU baseline or
native feature set and the same checked mode. Fast-math and semantically weaker
C references are invalid comparisons.

For the designated core runtime suite:

- the geometric-mean native LLVM throughput must be at least 95% of the C/Clang
  O3 reference;
- any individual kernel more than 10% slower is a release blocker unless a
  reviewed, reproducible target limitation is documented;
- scalar and simple loop kernels should produce equivalent instruction quality;
- unchecked and checked suites are reported and gated separately;
- FFI benchmarks batch work so host-language call overhead is not mislabeled as
  generated-code performance.

Compilation latency, cold `run`, warm cache-hit `run`, peak memory, artifact
size, and steady-state runtime are measured separately. O3 may be slower to
compile and is still the `run`/`build` default. Controlled x86-64 and AArch64
benchmark hosts enforce the performance gate; all six release targets run the
functional and ABI suites.

## Diagnostics, failures, and security boundary

Source errors remain ordinary stable `CKxxxx` diagnostics and occur before
native output. Unsupported target, CPU, artifact-kind, or backend/runtime
combinations are CLI errors. LLVM, ORC, ABI classification, embedded runtime,
and LLD failures retain their stage in the message and never masquerade as
source diagnostics.

The JIT child contains failures from executing CK raw pointers or unchecked
operations, but it does not make such code memory-safe. No persistent compiler
process loads user machine code. LLD receives only objects generated by this
compiler. Temporary output and cache paths reject unsafe ownership or symlink
replacement where the platform provides the relevant checks.

JIT code memory is never left writable and executable at the same time. The
ORC/JITLink memory manager allocates writable, non-executable pages while
relocations are applied, then finalizes code read/execute and keeps data
non-executable before transferring control. Linux uses `mprotect` or an
equivalent dual mapping; Windows uses the corresponding allocation/protection
transition and instruction-cache flush; Darwin uses `MAP_JIT` and Apple's
required per-thread JIT write-protection API where applicable. Signed macOS
release binaries carry the narrowly scoped `allow-jit` entitlement only when
the hardened runtime requires it; they do not disable library validation or
other code-signing protections to make JIT execution work.

The parent returns nonzero for compilation failure, runtime failure, checked
failure, output failure, or abnormal child termination. It does not print a
success line before the child has exited successfully. A successful `run`
prints no compiler status text at all: stdout belongs to the CK program, while
compiler and runtime diagnostics belong to stderr.

`CKR0006` is a `ckc run` parent diagnostic. A standalone executable that is
terminated by an unhandled machine fault retains the host OS signal or
exception behavior; the minimal runtime does not install process-wide crash
handlers that could interfere with raw-pointer semantics.

## Verification and release gates

Implementation is incomplete until all of these gates pass:

1. Native IR is verified after construction and after optimization for every
   representative language fixture and checked-mode combination.
2. C-versus-Native differential tests cover scalar, control flow, void, calls,
   structs, pointers, slices, checked ordering, and f64 edge behavior.
3. ABI classifiers are compared with pinned Clang fixtures on every one of the
   six release targets; generated headers compile in development C harnesses.
4. Dynamic libraries are loaded without a compiler by Python `ctypes` or an
   equivalent system FFI test and every exported shape is exercised.
5. Object, static, dynamic, and executable artifacts are produced with an empty
   external-tool PATH. Dependency inspection proves the zero-runtime guarantee.
6. `run` and AOT executable output, normal exit status, checked diagnostics,
   and print formatting agree on all six targets. Separate tests prove that the
   `run` parent maps abnormal child termination to `CKR0006`.
7. Cache miss, hit, bypass, corruption, permission, concurrent write, eviction,
   and clean operations are tested; a corrupt cache never changes semantics.
8. The performance contract passes on controlled x86-64 and AArch64 workers.
9. Release archives remain the existing six target names with checksum
   sidecars. Each `ckc --version --verbose` reports the compiler, LLVM, Native
   ABI, runtime ABI, target, and enabled CPU backend.
10. Release binaries have no dynamic LLVM/LLD/Clang or non-system C++ runtime
    dependency. Required LLVM notices are embedded and available through
    `ckc licenses` so the functional distribution remains a single executable.
11. JIT page-permission tests prove the writable-to-executable transition and
    absence of persistent RWX mappings on every target. Signed macOS AArch64
    and x86-64 archives execute `ckc run` under their release signing and
    hardened-runtime configuration.

Release CI builds the pinned LLVM source in a controlled, cached stage and
links the target-specific components statically. General compiler tests remain
fast enough to run without rebuilding LLVM, while a required native-toolchain
job owns the full integration matrix. Building `ckc` from source documents the
matching pinned LLVM bootstrap; end users of release archives do not perform it.

## 0.10 compatibility boundary

0.10 changes implementation and command behavior only with explicit release
documentation:

- `build` still defaults to a dynamic C-ABI library, but now uses LLVM and LLD
  internally and needs no Clang;
- `build --kind` adds executable, static, and object output;
- `run`, `main`, and LLVM-native numeric output are new;
- `main` and the seven native print names become reserved; a conflicting V0.9
  user declaration must be renamed;
- `build-llvm` is a deprecated compatibility alias;
- native checked modes now match the existing C status contract;
- the standalone LLVM exported-shape ABI is retired in favor of the single
  Native C ABI, while `emit-llvm` remains an inspection artifact;
- Native `emit-llvm --target` is limited to the normalized host triple so its
  DataLayout and public thunks cannot claim an unsupported cross-target ABI;
- native builds no longer leave generated `.c` or `.ll` intermediates; use
  `emit-c` or `emit-llvm` when those inspection artifacts are required;
- `emit-c` remains available but never compiles or links its output.

The C and WebAssembly contracts do not silently gain native runtime I/O. V0.9
programs that do not depend on the documented standalone LLVM exported shape
retain their source semantics. Compatibility fixtures and release notes must
cover every intentional 0.10 change before tagging.

## Explicit non-goals

The 0.10 native toolchain does not include cross-compilation, fat multi-version
libraries, program arguments, strings, stdin, general byte I/O, dynamic memory,
an allocator, ownership, exceptions, threads, a REPL, a security sandbox,
fast-math, C/WASM runtime printing, or a public embeddable JIT API. These require
separate versioned designs and cannot be added merely to make this plan pass.
