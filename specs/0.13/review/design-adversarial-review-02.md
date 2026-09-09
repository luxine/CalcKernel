# CK 0.13 Design Adversarial Review 02

## Scope and method

This read-only review examined revised commit `2fead05d3b59` against both
language mirrors and the current CLI, KIR optimizer, LLVM 22.1.8 bridge,
artifact/cache, ABI, and CI implementation. It first retested review 01's three
closures, then searched for new implementation-blocking contradictions.

## Verdict

`BLOCKED` with two blocking findings.

## Blocking findings

### B1: exact artifact-kind identity leaves object use without a profile source

The revised CLI rejects object generation but promises baseline/native
profile-use objects. At the same time, `CkProfileIdentity` included exact
artifact kind and use compared every identity field. A dynamic/static training
profile therefore could not be used for object output, and Native `emit-kir`
has a consumer but no physical artifact-kind input with which to satisfy that
identity.

Repository evidence confirms executable maps to `NativeExecutable`, while
dynamic, static, and object all map to `NativeLibrary`. The semantic profile
topology follows that consumer split rather than the final container kind.

Minimum closure: identify profiles by executable/library topology class and
keep physical kind only in final artifact/cache identity, or withdraw object
use/add a new intended-kind training surface.

### B2: the proposed O2 codegen tail still permits profile-driven copying

Attaching weights after the default O2 IR pipeline did not close the boundary.
The current object path uses `TargetMachine::addPassesToEmitFile`, whose ordinary
LLVM 22.1.8 codegen pipeline still performs IR preparation before instruction
selection. Later, `MachineBlockPlacement` can use profile data to enable partial
tail duplication and change the machine CFG. Both violate O2's frozen
non-duplicating promise.

Evidence includes `ckc_llvm_module_optimize`, the bridge object-emission path,
LLVM 22.1.8 `TargetPassConfig::addISelPasses`, and
`MachineBlockPlacement`'s profile-sensitive tail-duplication path.

Minimum closure: keep all IR and structural machine codegen profile-blind, then
apply profile at a closed late-machine boundary whose independently verified
delta permits ordering only, plus unavoidable terminator/fixup/alignment repair.

## Important non-blocking findings

- The runtime should repeat no-follow/file-identity validation when opening the
  generation directory, not rely only on build-time validation.
- Library flush needs a concurrent-flush test and static-library wording based
  on a host shutdown boundary rather than unload.
- Histogram weighting should prove a bound on selected-versus-baseline cost
  difference instead of assuming an upper-endpoint representative is
  conservative.

## Confirmed closed areas

Raw-shard-only merge and rejection of multiversion/generate object combinations
close review 01's first two blockers narrowly. The explicit library flush,
saturation propagation, checked integer model, cache/ABI schema advances,
ten-job CI mapping, and English/Chinese mirror remain coherent apart from the
findings above.

