# Implementation blocker 33: v0.13 aggregate multiversion artifact size

## Verdict

Confirmed implementation blocker. Exact v0.13 run `34051103711`, AArch64
performance job `101534772807`, passed the cumulative runtime gates but rejected the
schema-8 aggregate multiversion artifact size: ordinary shared products totalled
`9632` bytes and multiversion products totalled `20584` bytes, so the measured ratio
was `2.13704 > 2.0`. Exact v0.14 run `34051523126`, job `101535896793`, reproduced the
same rejection while preparing its immutable v0.13 replay.

This is not measurement noise and not a reason to change the gate. The prior
dead-section fix removed unreachable runtime code, but each ELF shared product still
retained a full static symbol table and a redundant process-wide capability cache in
addition to the generated per-root dispatch pointer slot.

## Correction

V0.14 inherits the exact v0.13 correction at
`ad44b16a81762610b002a38806673102d3d55ff9`:

- ELF shared links use LLD `--strip-all`; the public export remains loader-visible in
  `.dynsym`.
- Every single-root performance product carries one private pointer-width, aligned,
  writable `.ck_dispatch_slot` `NOBITS` section. Selected-direct evidence reads that
  section from the same artifact and fails closed on a missing or ambiguous slot.
- The generated per-root acquire/release pointer slot is the sole cache and
  publication layer. Concurrent first calls may repeat baseline-safe capability
  detection, but exactly one compatible function pointer is published and later calls
  take the existing one-load fast path.
- The one-shot private detector is compiled with the repository's size-first recipe:
  `-Oz` on Clang-family builds and `/O1` on MSVC.

Reconstructing the exact failed AArch64 archive inputs with this production link shape
produced ordinary total `8544` bytes and multiversion total `14792` bytes, ratio
`1.73127`, below the unchanged `2.0` aggregate limit. Hosted exact-SHA CI remains the
authoritative performance acceptance.

## Frozen boundaries

No language rule, public Native ABI, Runtime ABI, target feature set, safety or strict
floating-point rule changed. No workload, sample count, timing method, corpus,
platform, required job or performance/artifact threshold was reduced. The v0.13 replay
manifest is repinned to the exact repaired revision; its SHA-256 is
`563df5c58baec8e0e23f95a8c23670823809c79f99c869ee8d1e543c33d52885`.
