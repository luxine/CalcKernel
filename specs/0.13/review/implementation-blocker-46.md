# Implementation blocker 46: preserve Win32 null argument types under the ARM64 C++ frontend

## Evidence

Exact-SHA workflow-dispatch run `34241617859`, commit
`f62149d8bed686032a2631305cb3af0de30df54d`, failed required Windows ARM64
job `102113212994` while bootstrapping the pinned native toolchain. The profile
runtime C++20 compilation reported MSVC C2664 at all three `CreateFileW`
security-attributes arguments and the `WriteFile` overlapped argument. The job
stopped before fact-audit or capability artifacts could be produced; the full
job log identifies the four compiler diagnostics directly.

## Root cause and rejected shortcuts

The preceding Windows ARM64 atomic repair intentionally selects the C++20
frontend so `std::atomic_ref` can emit lock-free inline atomics. The surrounding
platform source still passed C-style `(void *)0` values to Win32 parameters
whose pointer types are more specific than `void *`. C permits those implicit
conversions; C++ does not.

Returning to the C frontend would restore the unavailable ARM64 atomic path.
Removing Windows ARM64, weakening `/WX`, skipping the profile runtime, or
allowing the failed required job is rejected.

## Repair

A focused contract was added first and observed failing. The three
`CreateFileW` calls now pass `(LPSECURITY_ATTRIBUTES)0`, and `WriteFile` passes
`(LPOVERLAPPED)0`. These are explicit null values of the SDK-declared parameter
types, remain valid when the same source is compiled as C on Windows x64, and
do not change runtime behavior. The profile-runtime provenance digest is
updated to bind the repaired source bytes.

This changes no language or public ABI, profile format, safety or strict-FP
semantics, optimizer policy, target eligibility, threshold, timed work, sample
count, corpus, platform, or required job.

## Acceptance

The RED/GREEN Win32 argument contract, profile-runtime provenance validation,
formatting, lint, complete locked tests, and replacement exact-SHA ten-job
workflow must pass. Windows ARM64 must compile the profile runtime through the
same C++20 frontend with `/WX`; no platform may be skipped.
