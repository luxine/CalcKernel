# CalcKernel 0.13 Native LLVM and C ABI

[简体中文](../zh-CN/abi/llvm.md)

CalcKernel 0.13 pins LLVM 22.1.8. Verified KIR is lowered structurally through a
checked C++ bridge, verified before and after optimization, emitted as object
bytes by the host TargetMachine, and linked in process with LLD. `emit-llvm`
prints this verified module for inspection.

## Target boundary

Native generation is host-only. An explicit target must normalize to the host
triple; cross-target output is rejected before artifact creation. Release
baselines are LLVM `x86-64` plus mandatory SSE2 and generic ARMv8-A plus its
ABI-mandated FP/Advanced SIMD facilities. `--cpu native` is opt-in for builds;
`run` uses detected host features.

The release binary contains the required host code generator, LLD driver, and
ORC layer. It has no runtime dependency on LLVM, LLD, Clang, or a non-system C++
runtime. `CKC_LLVM_PREFIX` is used only when building the compiler from source.
The bootstrap cache identity includes every native runtime source, header,
assembly file, and platform link input in addition to the pinned LLVM manifest
and bootstrap recipes, so a cached prefix cannot retain stale runtime objects.

On Windows, LLVM/LLD and the bridge use the release-static MSVC CRT (`/MT`),
and Rust uses `+crt-static` in every build profile. The bootstrap sets
`CMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded` and checks the actual C/C++ compile
commands before building. Both installation and cache validation inspect real
COFF archive directives with pinned `llvm-readobj`; dynamic, debug, or mixed
CRT inputs are rejected. The Windows manifest records this CRT identity and
includes the COFF driver's LibDriver, WindowsManifest, and DTLTO dependencies.
The verifier scripts are part of the cache key; `static_only = true` alone is
not evidence of static CRT contents.

## Internal representation

Native accepts only a verified KIR artifact. A pre-LLVM fact audit validates
the origin, dominance, contract-instance scope, alias completeness, alignment,
range, effect, and proof dependencies of every attribute or metadata candidate.
Audit failure stops before bridge invocation. Valid facts may become LLVM
`noalias`, `readonly`/`writeonly`, alignment, range, alias-scope, loop, or
vectorization information; the bridge never invents a stronger fact.

The module's canonical `KirTargetProfile` is queried from LLVM 22.1.8 for the
exact host target and CPU policy before optimization. It closes the fixed
operation universe, vector lane legality, alignment, and integer structural
costs used by CK's independent cost checker. Its digest is revalidated at the
Rust/C++ boundary and enters object/cache identity. A target, feature, query,
or digest mismatch stops before LLVM IR construction.

CK integers map to equal-width LLVM integers, `f64` to `double`, bool to `i1`,
pointers to opaque `ptr`, structs to declaration-order LLVM structs, and void
returns to LLVM `void`. Signedness selects compare, divide, remainder, and
integer-to-float operations. No fast-math flags are enabled.

KIR v3 fixed vectors lower structurally to equal-width LLVM vectors. Vector
loads/stores, strict arithmetic, casts, compares/selects, and modular integer
add/multiply reductions are emitted only after the KIR independent checker has
closed lane mapping, operation equivalence, fallback identity, target legality,
and cost/budget proofs. LLVM optimization may further improve that valid module,
but cannot be the source of CK safety or alias claims.

Stored `slice<T>` is `{ ptr, i32 }`. Internal calls preserve its data/length
pair and internal aggregate returns. Checked modes use explicit control flow and
the status codes in [modes.md](modes.md), never traps. `--overflow` and
`--bounds` select those modes.

A natural void function is `define void`; targetless calls use `call void`, and
explicit or natural completion uses `ret void`.

These forms are compiler internals, not the public library ABI. The independent
0.9 textual LLVM export-shape promise is retired.

The public Native C ABI remains version 1 in 0.13 and the Runtime ABI remains
version 2. The private LLVM bridge ABI 4 replaces 0.12 bridge ABI 3; native cache
and code-generation identity use KIR v3 plus `CKCOBJ03` key schema 4 and
manifest schema 4. These private identities intentionally invalidate 0.12 and older
objects without changing foreign-call signatures.

## Profile and multiversion objects

Profile-generation modules link the compiler-private schema-1 collection runtime
and expose the generated full-identity flush control only for library topology.
Final profile-use modules contain no counter, writer, profile path, or generation
runtime. Profile annotations are consumed by CK's independent optimizer; they do
not become LLVM safety metadata or proof.

`--cpu multiversion` lowers one verified baseline module, zero or more independently
verified target variants from the same KIR pre-state, and the compiler-private
dispatch runtime as separate named-object members. Every object is verified and
feature-audited before assembly. The baseline-safe detector recognizes only the
closed x86-64 v3/v4 and Linux AArch64 SVE/SVE2 tiers, fails closed on incomplete
state, and publishes one process-local selection through acquire-release atomics.
Public Native C ABI thunks keep their names, addresses, signatures, checked-status
behavior, and visibility; baseline, variant, detector, and runtime symbols stay hidden.

The named-object bundle links as an executable, dynamic library, or static archive.
A multiversion object output is rejected because 0.13 has no partial-link bundle
contract; baseline/native single-version objects remain supported. `CKCOBJ03`
admits a cached bundle only when ordered member names/roles, target set, profile,
dispatch runtime, physical artifact kind, every object digest, key schema 4, and
manifest schema 4 all match. Final artifacts retain the existing self-contained
system-runtime policy and add no CK/LLVM/compiler shared dependency.

## Native C ABI

Native object, static, and dynamic artifacts expose one Native C ABI described
by their generated header. Every public source function is implemented by an
export thunk around an internal natural function. The thunk applies target ABI
classification, bool normalization, slice flattening, checked return/status
rules, and platform symbol visibility. This is the same contract for all three
library artifact kinds.

The public mappings are fixed-width C integers, strict `double`, target C
`_Bool`, declaration-order C struct layout with target padding/alignment,
flattened slice parameters `(T* data, uint32_t len)`, direct unchecked returns
and C `void`, or module-wide checked status plus result out-pointer. Source
symbol names and default visibility are preserved; Windows dynamic exports use
the generated DLL decoration.

The compiler owns explicit classifiers for SysV AMD64, Darwin x86-64, Linux and
Darwin AAPCS64, Windows x64, and Windows ARM64. They determine register classes,
indirect/by-value aggregates, small aggregate returns, extension attributes,
alignment, and hidden results. Pinned Clang fixtures are development oracles;
the generated header is the consumer authority.

`main` is never exported by a library. Executables link a compiler-owned entry
and runtime object with the user object. See [C ABI](c.md) for the separate
source-only C emitter and [checked modes](modes.md) for shared status meaning.

Native user artifacts contain no CK, LLVM, ORC, LLD, Clang, libc formatting,
or external compiler-runtime dependency. Objects and static archives naturally
need a consumer link step, but add no CK runtime after linking. Dynamic
libraries export only requested CK symbols and required platform metadata.
Non-allocating producer provenance is not a dependency: ELF linked products
retain the exact `Linker: LLD 22.1.8` `.comment`, and the artifact audit requires
that section to remain non-`ALLOC` while independently rejecting loader-visible
dependencies, undefined executable symbols, and unexpected exports.
Linux executable runtime uses its kernel boundary; Windows uses embedded stable
process imports and computation DLLs have no entry; Darwin uses embedded minimal
system stubs and LLD ad-hoc signing. Darwin objects use PIC with an explicit
Small code model for both AOT and ORC. Internal calls must not require absolute
pointer fixups in executable `__text`; dyld must never need to write code pages.
`LC_MAIN` references the compiler-generated C-ABI entry `_main`, which dyld calls
as a normal function and uses its return value as the process exit status.
Runtime failures terminate through the embedded platform exit helper.

## ORC execution

`ckc run` executes the same optimized object semantics through ORC. ELF and
Mach-O AArch64/x86-64 plus COFF x86-64 use JITLink. COFF AArch64 uses the pinned
RuntimeDyld compatibility path because LLVM 22.1.8 lacks its JITLink backend.
Both resolve all symbols eagerly and enforce writable-to-executable page
transitions before calling `main`.

The COFF AArch64 compatibility layer retains CK's audited section memory
manager and restores LLVM 22.1.8 LLJIT's standard COFF responsibility contract:
RuntimeDyld object flags are reconciled with the materialization responsibility,
and additional object symbols such as weak/COMDAT entries are automatically
claimed. This is confined to the existing compatibility path; it does not
enable process-symbol search or turn RuntimeDyld into a general CK backend.

COFF x86-64 JITLink keeps arbitrary process-symbol lookup disabled. Its five
embedded CK runtime objects are joined only for JIT execution by a separately
hashed, data-only `__ImageBase` anchor. The anchor and the fixed object set are
allocated in the same 512 MiB JIT reservation so MSVC `.pdata` image-relative
relocations remain representable. This support object is internal to `run`: it
is not passed to LLD for object, static, dynamic, or executable artifacts and
does not add a public CK symbol or runtime dependency. A CK program object that
defines the PE/COFF-reserved `__ImageBase` name is rejected before execution.

On Darwin, ORC selects one of two mutually exclusive W^X mechanisms from the
runtime capability. Where per-thread JIT write protection is supported, code
uses `MAP_JIT` and toggles the thread between writable/non-executable and
readable/executable modes. Where that capability is unavailable, including
Darwin x86-64 and restricted virtual hosts, it reserves ordinary RW/NX pages
and finalizes each segment with page protection to RX or R/NX. The latter is
not an RWX fallback. The internal audit rejects mixed capability tuples and
proves relocation, final code/data permissions, and instruction-cache
finalization for both paths.

ORC is not a public embeddable API in 0.13. `emit-llvm` is host-only diagnostic
output and does not promise a stable external LLVM ABI.
