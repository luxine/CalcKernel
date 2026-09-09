# Implementation blocker 37: exact-run portability, checked-map schedule, and size closure

## Evidence

Exact-SHA workflow-dispatch run `34133617442`, commit
`7b883bf36a2edfb6720caa69aa7f10c94ebb9e43`, exposed four actionable defects after
all job and step states, failed logs, and available artifacts were downloaded:

- native integration job `101779791500` failed Clippy because the checked
  multiversion emitter retained an unused request binding;
- x86-64 performance job `101779791592` measured checked `map_u32` at
  `6,651,367 ns` versus the faster `5,680,485 ns` C oracle, below the unchanged
  90% throughput floor; checked `zip_u32` showed the same two-way loop shape;
- AArch64 performance job `101779791298` measured aggregate multiversion
  artifacts at `16,408 / 8,160 = 2.01078`, just above the unchanged `2x` gate;
- Windows x64 job `101779791518` treated an empty private-symbol listing from a
  deliberately stripped PE DLL as proof that `cold_step` had been re-inlined,
  while Windows ARM64 job `101779791620` also failed to link
  `profile-runtime.obj` because MSVC 19.44 left the `_Interlocked*` spellings as
  unresolved externals despite `/Oi`.

The AArch64 artifacts isolate the size delta: ordinary products retained only
LLD's 19-byte `.comment`, while every multiversion product also retained GCC's
compiler-ident string from the private dispatch runtime. Removing that repeated
46-byte payload from five products brings the observed aggregate to at most
`16,178 / 8,160 = 1.9826` without changing program code or the size gate.

## Root cause and rejected shortcuts

The bridge attached `llvm.loop.unroll.count=2` to every scalar x86 checked
overflow loop. For streaming maps this duplicates the overflow-test dependency
chain and branch footprint; the equivalent checked C and Rust oracles retain a
single scalar chain. This is a scheduling defect, not evidence for relaxing the
oracle threshold.

Unix runtime compilation omitted `-fno-ident`, even though the identity string
does not participate in execution, provenance, or target auditing. The PE test
asked a stripped final DLL for private COFF symbols that no longer exist by
contract. The optimizer invariant must instead be observed in optimized LLVM IR
before link-time stripping. Finally, the ARM64 underscore intrinsics are not a
portable link contract when the pinned compiler demonstrably emits references;
the already embedded kernel32 import library is the closed Windows ABI authority.

Lowering thresholds, reducing samples or timed work, retaining a Windows-only
false positive, enabling a system linker, disabling ARM64 profiling, or accepting
unresolved compiler helpers are rejected.

## Repair

Regression tests first required a memory-aware checked-loop schedule, complete
Windows ARM64 import closure, compiler-ident-free Unix runtime recipe, and a
pre-strip helper-policy assertion. The implementation then:

- removes the stale checked-request binding so the all-feature Clippy gate can
  compile the exact emitter;
- detects scalar non-local load/store maps and attaches
  `llvm.loop.unroll.disable` only to their checked overflow loops, while retaining
  the bounded two-way schedule for other checked scalar loops;
- advances both ordinary and multiversion Native object cache identities with
  `x86-checked-memory-map-schedule-v1`;
- adds `-fno-ident` to the shared Unix runtime flags used by the private dispatch
  runtime;
- checks the retained `cold_step`/inlined `hot_step` invariant in optimized LLVM
  IR, and checks only public exports in the stripped Windows DLL;
- binds ARM64 profile atomics and the run-id increment to explicit
  `Interlocked*` kernel32 imports, while retaining MSVC intrinsics on x64, and
  updates the hashed profile-runtime provenance.

The repair changes no language or public ABI rule, checked-overflow behavior,
strict-FP or bounds semantics, target ISA, performance or stability threshold,
artifact-size threshold, timed work, sample count, corpus, platform, or required
job matrix.

## Verification

The focused structural RED tests are green after the repair. Local formatting,
no-native lint/tests, provenance validation, and source checks are rerun for the
candidate commit. The replacement exact-SHA workflow-dispatch run remains the
authority for LLVM Native code generation, Windows x64/ARM64 imports and PE
inspection, and the unchanged x86/AArch64 performance gates.
