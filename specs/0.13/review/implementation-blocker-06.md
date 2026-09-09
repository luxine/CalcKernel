# Implementation blocker 06: incomplete native-prefix cache identity

Date: 2026-09-04

## Finding

Exact candidate CI run `33785925048` still returned profile flush status 43 in
jobs `100751260765` and `100751260827` after the corrected AArch64 source had
passed local publication tests. The repeated result came from a stale binary,
not from the repaired source: the composite bootstrap cache key covered
`native/runtime/**` but omitted both `native/profile_runtime/**` and
`native/dispatch_runtime/**`, even though the bootstrap scripts compile all
three runtimes into the cached prefix.

The runner therefore restored a previously valid prefix containing the old
AArch64 profile runtime object. Prefix validation proved only that the cached
object matched its own cached manifest; it could not bind that object back to
the current repository runtime sources.

## Resolution

The manifest-addressed bootstrap recipe digest now includes every C/header and
provenance input for the profile and dispatch runtimes, in addition to the
already covered native runtime and bootstrap/validation scripts. Any runtime
source correction consequently creates a new immutable cache key and rebuilds
both release and oracle prefixes. A CI contract test freezes this complete
input closure.

No language, ABI, PGO policy, performance threshold, sample count, corpus, or
platform gate changes. The failed jobs must be replaced by a fresh exact-SHA
run that builds a new prefix; their stale cache artifacts are not acceptance
evidence.
