# Implementation blocker 59: inherit the Windows ARM64 profile-runtime type repair

## Evidence

Exact V0.13 workflow-dispatch run `34241617859`, commit
`f62149d8bed686032a2631305cb3af0de30df54d`, failed required Windows ARM64
job `102113212994`. Its C++20 profile-runtime compilation reported MSVC C2664
for all three `CreateFileW` security-attributes arguments and the `WriteFile`
overlapped argument. The job stopped before its fact-audit and capability
artifacts could be produced; the complete job log identifies the four compiler
diagnostics. V0.14 inherited the same source and C++20 bootstrap policy, so its
in-flight exact run and old V0.13 replay pin were both superseded by this defect.

## Root cause and rejected shortcuts

The lock-free Windows ARM64 atomics intentionally require the C++20 frontend,
but the Win32 platform source retained C-only implicit conversions from
`(void *)0` to more specific SDK pointer types. The equivalent x64 build uses
the C frontend and could not expose this mismatch.

Reverting the atomic repair, compiling without `/WX`, dropping Windows ARM64,
skipping profile publication, retaining the rejected replay SHA, or accepting
an older run is rejected.

## Repair and replay identity

V0.14 applies the same focused repair: `CreateFileW` receives
`(LPSECURITY_ATTRIBUTES)0`, `WriteFile` receives `(LPOVERLAPPED)0`, and the
profile-runtime provenance digest binds the new bytes. A contract was added
first and observed failing before the source change.

V0.13 independently carries the repair at exact commit
`8286b32174e33a4b874d71e8c17a5a605a382f0d`. V0.14's normative accepted base,
replay manifest, replay preparer, task and acceptance documents, and structural
contract are repinned to that exact commit. The resulting
`benches/baselines/v0_13_replay.toml` SHA-256 is
`26f30615b9e87f7c3d88d5dd14e00e635e891637792cc445a9ecd812d3a2fbae`.
Historical blocker records retain superseded identities as evidence.

This repair changes no language or public ABI, profile format, tuning decision,
schema 8 or 9 rule, safety or strict-FP semantics, target eligibility,
performance/stability/size threshold, timed work, sample count, corpus,
platform, or required job.

## Acceptance

The focused RED/GREEN contracts, provenance and replay-pin validation,
formatting, lint, complete locked tests, and separate replacement exact-SHA
V0.13 and V0.14 ten-job workflows must pass. Windows ARM64 must compile through
the same C++20 `/WX` path, and V0.14 must rebuild exact V0.13 rather than adapt
or substitute its historical evidence.
