# Implementation blocker 62: request DACL-write access for Windows publication

## Evidence

Exact-SHA V0.14 run `34258812502`, commit
`cd5f463fd9a717d782caa6686269c9a942022616`, completed Windows x64 native job
`102171709543` with a required-suite failure. Six independent publication and
recovery tests failed while acquiring their first persistent destination lock:
`SetSecurityInfo` returned Windows error 5, `Access is denied`, under the
diagnostic `protect Windows publication file`. The same suite passed its
unrelated tests and failed before publication mutation, so the evidence does not
indicate a journal, recovery, or test-order race.

The same run's x86-64 performance job `102171709329` independently passed schema
8 and then correctly reported a runner capability/infrastructure failure. Its
AMD EPYC 7763 exposes x86-64-v3 but lacks the required
`avx512bw,avx512cd,avx512dq,avx512f,avx512vl` feature set. That failure remains
fail-closed and requires a replacement run on a real x86-64-v4 worker; it is not
classified or repaired as a compiler performance regression.

## Root cause and rejected shortcuts

Windows `SetSecurityInfo` with `DACL_SECURITY_INFORMATION` requires the target
handle to carry `WRITE_DAC`. `create_private` opened the new file with generic
read and generic write only, then attempted to replace its DACL through that same
handle. Generic write does not include `WRITE_DAC`, so Windows correctly denied
every owner-only ACL installation.

Skipping ACL protection, accepting inherited permissions, weakening validation,
retrying without owner-only security, dropping either Windows job, treating the
x86 capability error as a pass, or reducing any performance work is rejected.

## Repair

A platform-independent access-mask regression was added first and observed
failing with a zero `WRITE_DAC` bit. Windows private-file creation now explicitly
requests generic read, generic write, and `WRITE_DAC` in its desired-access mask
before calling the unchanged owner-only `SetSecurityInfo` path. If protection
still fails, the existing fail-closed cleanup drops the handle and removes the
unprotected initializer.

This changes no language or public ABI, publication format, owner-only policy,
journal protocol, tuning decision, schema 8 or 9 rule, target eligibility,
performance/stability/size threshold, timed work, warmup, sample count, corpus,
platform, or required job.

## Acceptance

The RED/GREEN access-mask regression, an x86_64-pc-windows-msvc cross-check,
formatting, lint, complete locked tests, Python performance/checker contracts,
documentation contracts, and a replacement exact-SHA V0.14 ten-job workflow
must pass. Windows x64 and ARM64 must execute the unchanged owner-only
publication tests successfully. The replacement x86 performance job must obtain
a real v4-capable worker; a v3-only allocation remains an actionable
infrastructure failure.
