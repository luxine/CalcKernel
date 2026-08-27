# CalcKernel 0.10 Native LLVM and C ABI

[简体中文](../zh-CN/abi/llvm.md)

CalcKernel 0.10 pins LLVM 22.1.8. Native MIR is lowered structurally through a
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

## Internal representation

CK integers map to equal-width LLVM integers, `f64` to `double`, bool to `i1`,
pointers to opaque `ptr`, structs to declaration-order LLVM structs, and void
returns to LLVM `void`. Signedness selects compare, divide, remainder, and
integer-to-float operations. No fast-math flags are enabled.

Stored `slice<T>` is `{ ptr, i32 }`. Internal calls preserve its data/length
pair and internal aggregate returns. Checked modes use explicit control flow and
the status codes in [modes.md](modes.md), never traps. `--overflow` and
`--bounds` select those modes.

A natural void function is `define void`; targetless calls use `call void`, and
explicit or natural completion uses `ret void`.

These forms are compiler internals, not the public library ABI. The independent
0.9 textual LLVM export-shape promise is retired.

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
Linux executable runtime uses its kernel boundary; Windows uses embedded stable
process imports and computation DLLs have no entry; Darwin uses embedded minimal
system stubs and LLD ad-hoc signing.

## ORC execution

`ckc run` executes the same optimized object semantics through ORC. ELF and
Mach-O AArch64/x86-64 plus COFF x86-64 use JITLink. COFF AArch64 uses the pinned
RuntimeDyld compatibility path because LLVM 22.1.8 lacks its JITLink backend.
Both resolve all symbols eagerly and enforce writable-to-executable page
transitions before calling `main`.

ORC is not a public embeddable API in 0.10. `emit-llvm` is host-only diagnostic
output and does not promise a stable external LLVM ABI.
