# Implementation blocker 49: full-width v4 integer maps

V0.14 run `34295522872`, job `102291370458`, rebuilt exact V0.13
`f5dd9989245fd6d9e70babc95dcdf7af17ecb42f` on an Intel Xeon Platinum 8573C.
Its complete historical schema-8 replay failed `trip-unroll-simd`'s unchanged
1.03 dispatch/ordinary limit: 21,102 / 20,151 ns = 1.0471936877. The selected
direct channel was 21,100 ns, so this failure does not identify the dispatch thunk
as the bottleneck. The same SHA's independent V0.13 performance job passed on
an AMD EPYC 7763, whose selected implementation is the v3 member.

The retained ELF objects show that both v3 and v4 map implementations used
eight-lane AVX2 operations. LLVM's generic v4 cost preference did not exploit
the already-authorized full AVX-512 width. The compiler now supplies a
sixteen-lane vector-width request only for scalar wrapping i32 memory-map loops
in the exact v4 target with explicit AVX512F. Calls, checked overflow,
volatile/atomic accesses, non-i32 memory, existing vectors and existing loop
schedules are excluded. LLVM's independent legality analysis and all target
feature/ABI audits remain mandatory. The multiversion codegen cache identity
includes `x86-v4-i32-map-width-16-v1`.

The local contract regression was observed failing before the implementation.
An x86 Native regression lowers and emits the same source under baseline, v3,
and v4 targets, requiring four, eight, and sixteen lanes respectively. This
Native x86 test and actual performance acceptance require the remote x86 host;
local AArch64 results cannot substitute for them.

No threshold, stability rule, timed work, sample count, corpus, platform,
required job, dispatcher protocol, profile schema, safety fact, or floating-point
semantics changed. The complete retained report also contains a compute-bound
dispatch/direct ratio above its limit; a fresh full exact-SHA run remains
necessary to establish acceptance rather than selecting passing samples.
