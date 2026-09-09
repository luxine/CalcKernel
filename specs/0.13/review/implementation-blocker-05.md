# Implementation blocker 05: AArch64 Linux runtime portability

Date: 2026-09-04

## Finding

Exact candidate CI run `33782668586` exposed two independent AArch64 Linux
failures that assertion-disabled local builds could not certify:

- the multiversion dispatcher supplied an LLVM SSA name to a `void` indirect
  call, which assertion-enabled LLVM correctly rejected;
- the freestanding profile runtime reused x86 `O_DIRECTORY` and `O_NOFOLLOW`
  flag values on AArch64, so its first directory `openat` failed with `EINVAL`
  and every profile flush returned status 43.

## Resolution

- The multiversion dispatcher selects the unnamed `CreateCall` overload for
  `void` functions and retains the stable name only for value-producing calls.
- Linux profile publication now freezes the architecture-specific open flags:
  x86-64 retains `00200000`/`00400000`, while AArch64 uses
  `00040000`/`00100000`.
- Profile runtime provenance advances with the corrected Linux source digest.

No language, ABI, PGO policy, performance threshold, sample count, or release
gate changes. Regression contracts cover both bridge call construction and the
AArch64 flag values. The raw publication transaction additionally succeeds in a
Linux/AArch64 container and publishes exactly one completed shard.
