# Implementation blocker 47: Windows profile path and atomic runtime boundaries

## Evidence and root causes

Exact commit `8286b32174e33a4b874d71e8c17a5a605a382f0d`, workflow-dispatch run
`34257508363`, failed four Windows x64 PGO CLI tests in job `102167665158`.
Each reported `profile I/O failed for \\?\C:` with Windows error 1. Profile merge,
read and output validation shared a component walker that queried a bare
verbatim drive prefix before appending its root separator. Generation's
directory anchor already skipped that incomplete prefix and is unchanged.

The same run's Windows ARM64 job `102167665095` passed bootstrap and fact audit,
then failed three native profile-generation tests at link time with unresolved
`_InterlockedCompareExchange*` and `_InterlockedExchangeAdd*` symbols. The log
identifies MSVC `19.44.35228.0`. MSVC 17.14 enables outlined interlocked functions
by default on Armv8.0, including operations used by `std::atomic_ref`; `/Oi`
alone does not disable this policy. See the
[Microsoft compiler explanation](https://devblogs.microsoft.com/cppblog/introducing-the-forceinterlockedfunctions-switch-for-arm64/).

## Repair and verification

The profile path walker now appends but does not query `Component::Prefix`;
the complete root and every normal component retain their no-follow checks.
A canonical-path merge/write/read regression covers the verbatim form on
Windows and validates exact terminal profile bytes on every host. Existing
native CLI regressions remain required.

Only ARM64 profile-runtime compilation adds `/forceInterlockedFunctions-`.
The real PowerShell flag expression was executed for both supported Windows
targets: the ARM64 expectation failed before the repair and both cases passed
after it. Bootstrap additionally rejects `_Interlocked*` undefined symbols in
the compiled runtime object before recording its hash. The C++20 atomic-ref
implementation, lock-free assertions and C ABI are retained.

Local canonical-path/profile tests pass. Windows native execution is not
available on the local Darwin host and must be confirmed by replacement
exact-SHA CI. No test/job is skipped, no target baseline or public ABI changes,
and no CRT import is added to satisfy an unresolved atomic symbol.
