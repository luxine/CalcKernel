# Implementation blocker 65: validate persistent identity under its owning lock

## Evidence

Exact V0.14 commit `23c0eb254a7da211dba4940e1439d774a77cf827`, run
`34291739279`, failed Windows ARM64 job `102279724406` and Windows x64 job
`102279724438` in the early publication selector. Both passed five parent
tests, including successful publication, the complete crash matrix and real
process-death recovery. The remaining test read the persistent lock file via
`fs::read` while `PublicationSet` still held its exclusive lock. Windows returned
error 33 at `tests/tune/publication.rs:112`.

Windows `LockFileEx` denies access through a second handle even in the owning
process. Its exclusion is stronger than Unix advisory-lock behavior; see the
[Win32 lock contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-lockfileex).
The directory-write-access repair is therefore retained, as are all flush,
owner-only ACL and recovery requirements.

## Root cause and repair

The observer exposed the same ordering flaw in production: `acquire_one` read
and validated the persistent identity before acquiring the OS lock. A competing
Windows session would fail with lock violation instead of waiting, while a Unix
reader could observe bytes before it owned exclusion.

Production now acquires an RAII lock first, then rewinds and validates through
that exact owning handle. Identity rejection drops the guard and releases the
lock. No unlock/reopen gap, weaker shared lock or test-only public API is added.

A real concurrent regression holds the first destination lock, temporarily
writes an intermediate identity through its owning handle, starts a waiter,
then restores the valid identity before releasing exclusion. Before the repair,
the local test failed with an early identity error; after it, the waiter blocks
and succeeds after release. Both Windows preflights execute this regression.

The original persistence test now additionally requires Windows second-handle
reads to fail with error 33 while exclusion is held. Its full-name, magic,
40-byte, persistence and reacquisition checks remain and inspect bytes after
unlock. No test, sample, corpus, performance/size/stability threshold, target or
required job is removed. Replacement exact-SHA CI must pass both Windows
preflights and the unchanged complete native and performance matrix.
