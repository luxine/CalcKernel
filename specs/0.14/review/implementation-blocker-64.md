# Implementation blocker 64: writable Windows publication directory flush handles

## Evidence and diagnosis

Exact V0.14 run `34281039339`, commit
`abad55eff21d0b0fb8dfa3afef8b6e9ef6e80a86`, failed Windows x64 job
`102246357720` in publication lock acquisition and recovery. The owner-only
DACL installation error from blocker 62 is gone; the remaining failure is a
bare Windows error 5 during persistent-lock initialization.

Tracing initialization finds a read-only directory handle passed to
`File::sync_all`, which uses Windows `FlushFileBuffers`. That API requires
`GENERIC_WRITE`. The runtime correctly retains the mandatory directory barrier,
but the adapter did not request the access needed to invoke it. See the
[Windows API contract](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-flushfilebuffers).

## Repair and acceptance

The directory-handle access policy now requests generic read and generic write.
The access-mask regression was observed failing with a missing write bit, then
passing after the repair. Errors now identify directory open versus directory
flush. No error is suppressed, no volume-wide privileged flush is introduced,
and write-through rename is not treated as a replacement for the barrier.
Unsupported filesystem behavior remains fail-closed.

Both existing Windows host jobs execute the real publication test selector
before LLVM bootstrap, without removing the post-bootstrap native tests. This
gives an early real-host check of access, ACLs, persistent locks, journal
barriers and process-death recovery. All six hosts and both performance jobs
remain required. The local Darwin publication tests pass; actual Windows
directory flushing remains a replacement exact-SHA CI acceptance requirement.
