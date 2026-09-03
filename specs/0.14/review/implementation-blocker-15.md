# Implementation blocker 15: runtime cache and native code-model closure

Date: 2026-09-04

## Finding

Fresh exact-SHA CI exposed two defects below the V0.14 tuning layer:

- V0.13 jobs `100751260765` and `100751260827` restored a cached LLVM prefix
  whose key omitted `native/profile_runtime/**` and
  `native/dispatch_runtime/**`. The corrected AArch64 source therefore still
  executed an older profile-runtime object and failed flush status 43.
- V0.12 x86-64 job `100738211965` emitted the domain-fact kernel with LLVM's
  Large code model on ELF. Its extra address materialization erased the
  required no-alias gain even though the durable native contract specifies
  PIC plus Small for emitted products.

Both mechanisms are inherited by V0.14. Reusing V0.14 commit `3e1cc49` or its
old V0.13 replay would repeat stale-runtime or Large-model behavior and cannot
sign final acceptance.

## Resolution

The bootstrap cache identity now covers the complete native, profile, and
dispatch runtime source/provenance closure. The shared target-machine helper
selects PIC plus Small on every supported host object format, with structural
tests rejecting a Mach-O-only policy. V0.14 advances its exact V0.13 replay to
`1a0e593efa4c5c962e06fd4d0c239750bf2e1c5a`; the corresponding schema-9
manifest digest is
`de3c525bfc3c9f5080259cedd910b8924783ec42534db2d5c2df6bbf61421bd4`.
The nested V0.12 replay is likewise pinned to repaired commit
`d83805075b0ac8986c895b7a287c84eac509b7f9`.

No language, ABI, tuning policy, performance threshold, sample count, corpus,
or platform gate changes. A fresh exact-SHA V0.14 CI run must rebuild the
runtime prefix and pass the existing schema-7, schema-8, and schema-9 gates.
