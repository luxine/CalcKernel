# Implementation blocker 43: Windows verbatim profile-directory root

## Evidence

Exact-SHA V0.13 workflow-dispatch run `34182330156`, commit
`77e5e0a95b83d0faa8f63ddc8f2451a9b1322a40`, completed the Windows x64
toolchain bootstrap, capability audit, and profile-runtime link checks. Its
required CLI suite then failed four real PGO execution paths. Every generated
training executable returned status `43`, the fixed
`CKC_PROFILE_RUNTIME_STATUS_DIRECTORY` value, while the equivalent Unix jobs
passed.

## Root cause and rejected shortcuts

`PgoTemporaryRoot` canonicalizes the system temporary directory before a
generation artifact is built. On Windows, Rust represents that absolute path
with the verbatim namespace prefix `\\?\C:\...`. The collector's independent
component walk skipped only separators at offsets zero through two. It
therefore attempted to open the namespace separator at offset three, and later
the drive-root separator, as directory components. The first impossible open
made every shard publication fail closed with status 43.

Ignoring the training exit status, accepting missing shards, removing the
component-wise reparse check, or weakening the Windows required job is
rejected. Those changes would conceal a real runtime failure or remove the
directory replacement defense.

## Repair

A regression contract was added first and observed failing. The Windows
collector now parses the root of canonical drive, verbatim drive, UNC, and
verbatim UNC paths before walking components. Namespace and root separators
are never opened as ordinary components; every component below the root and
the final directory are still reopened with `FILE_FLAG_OPEN_REPARSE_POINT` and
checked against the compiler-captured volume/file identity. An unrecognized
root continues to fail closed.

The profile-runtime provenance digest is updated for the exact source bytes.
No profile schema, language or public ABI, safety rule, optimization policy,
performance or stability threshold, timed work, sample count, corpus,
platform, or required job is changed.

## Verification

The focused source contract is rerun RED/GREEN, followed by formatting, lint,
the complete locked test suites, release native build/audits, and the
replacement exact-SHA ten-job workflow. The Windows x64 and ARM64 PGO CLI
execution paths remain the authoritative platform proof.
