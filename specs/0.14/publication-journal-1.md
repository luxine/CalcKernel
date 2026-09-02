# CK 0.14 Publication Journal Schema 1

Status: normative crash-consistency protocol shared by both language designs

This internal schema is not a promise that future CK versions will read a v0.14
journal. It is a complete implementation contract for v0.14 recovery. A command
must establish the stable overlap closure and recover or fail closed before it
reads, stages, or replaces any member of the intended destination set.

## 1. Names, ownership, and bounds

Let `set_id` be the full output-set digest from `decision-schema-1.md`, and `s` its
first 32 lowercase hexadecimal characters. For every decision/artifact/sidecar
destination, let `destination_id = H("CK-TUNE-DESTINATION\0", canonical path
bytes)` and `d` be its first 32 hex characters. All files share the already-
validated canonical output parent:

| Purpose | Basename |
| --- | --- |
| persistent lock for each destination | `.ckc-tune-dest-<d>.lock` |
| active set journal | `.ckc-tune-set-<s>.journal` |
| set-journal update stage | `.ckc-tune-set-<s>.journal.new` |
| destination stage | `.ckc-tune-set-<s>.<tx>.<role>.stage` |
| destination backup | `.ckc-tune-set-<s>.<tx>.<role>.backup` |

`tx` is 32 lowercase hex characters encoding a fresh 128-bit operating-system
CSPRNG value; failure to acquire it aborts before staging. Role names are
`decision`, `header`, `import`, and `primary`. Only roles present in the output set
exist. CK reserves the exact basename grammar above. Each lock file contains exactly
`CKTLCK01` followed by its full 32-byte destination id; an existing lock with a
different id is a hard prefix-collision error. A colliding non-regular file,
symlink/reparse point, wrong owner/permissions, unexpected transaction, or prefix
whose journal carries another full `set_id` is a hard error.

Destination locks are persistent regular files with owner-only write permissions.
CK opens them no-follow, acquires all required exclusive OS advisory locks in
canonical full path-byte order, and holds them through discovery, recovery,
publication, and cleanup. It releases them in reverse order and never removes them,
preventing inode replacement and overlapping-set races.
Journal and transaction files are owner-only regular files opened no-follow. A
journal is at most 128 KiB, contains one through four destinations, and each path
byte string is at most 4,096 bytes.

### 1.1 Overlap-closure acquisition

Before mutation, CK computes the intended destination set and snapshots every valid
`.ckc-tune-set-*.journal`/`.journal.new` in the common parent. Starting with the
intended paths, it repeatedly adds all destinations from any journal that intersects
the current set, until no path is added. It then acquires the resulting destination
locks in canonical path-byte order and rescans. If a new or changed valid journal
expands the closure, CK releases in reverse order and retries. A malformed reserved
journal is a hard error.

Once the rescan is stable, CK recovers every intersecting transaction in full
set-id order while holding the complete closure. A concurrently active overlapping
transaction must hold one of the same destination locks and therefore completes or
becomes recoverable before this acquisition succeeds. Nonoverlapping transactions
do not block one another. Only after closure recovery may CK stage the intended set.

## 2. Exact journal bytes

The journal is exactly:

1. eight bytes `CKTJNL01`;
2. `U32(1)` schema;
3. `U64 generation`;
4. `U8 phase`;
5. 16 transaction-id bytes;
6. 32 full output-set-id bytes;
7. `U8 destination_count`;
8. that many `Destination` records in publication order;
9. `SHA-256("CK-TUNE-JOURNAL\0" || every preceding byte)`.

Integers are big-endian. A byte string is `U32 length || bytes`. Unix destination
bytes are exact non-NUL path bytes; Windows bytes are normalized absolute UTF-8.
A `Destination` is exactly:

| Field | Encoding |
| --- | --- |
| role | `U8`: decision=0, primary=1, header=2, import-library=3 |
| destination | canonical absolute path byte string |
| stage basename | ASCII byte string |
| backup basename | ASCII byte string |
| old present | `U8` exactly 0 or 1 |
| old digest | 32 bytes; all zero iff old is absent |
| old size | `U64`; zero iff old is absent |
| new digest | 32 bytes |
| new size | `U64` |

Publication order is decision, header when present, import library when present,
then primary. Records use that order regardless of numeric role. Paths must resolve
to the same parent and match the names derived before locking. Duplicate paths,
unknown phases/roles, noncanonical strings, bad zero fields, trailing bytes, wrong
digest, wrong transaction filename, or a generation inconsistent with phase are
invalid.

Phases and generations are exactly `Prepared=(1,1)`, `BackedUp=(2,2)`,
`DecisionPublished=(3,3)`, `SidecarsPublished=(4,4)`,
`PrimaryPublished=(5,5)`, and `Committed=(6,6)`.

## 3. Atomic journal update

To install generation `g`, CK creates the exact `.journal.new` with create-new
semantics, writes all bytes, flushes the file to stable storage, atomically replaces
`.journal`, then flushes the parent directory. Windows uses the documented replace/
write-through equivalent. It never overwrites the active journal in place.

If both files exist after a crash, recovery fully validates each before mutation.
It selects the higher valid generation only when transaction id, set id, and all
destination records are identical and the phase transition is the next legal one;
otherwise it fails closed. It installs the selected generation through the same
atomic-replace and directory-flush rule before continuing.

## 4. Publication barriers

With the complete destination-lock closure held and no unrecovered transaction:

1. create every destination stage with create-new, write, hash, and flush it; flush
   the parent directory so all stage names are durable;
2. install `Prepared` through Section 3;
3. rename each existing destination to its exact backup, then flush the parent
   directory; only afterward install `BackedUp`;
4. rename the decision stage to its destination, reopen/flush it, and flush the
   parent directory; only afterward install `DecisionPublished`;
5. in publication order rename each present sidecar stage, reopen/flush it, then
   flush the parent directory after the complete sidecar set; only afterward
   install `SidecarsPublished`;
6. rename the primary stage, reopen/flush it, and flush the parent directory; only
   afterward install `PrimaryPublished`;
7. rehash every new destination, install `Committed`, remove all backup/stage files,
   flush the parent directory, remove journal update/active files, and flush the
   parent directory again.

No journal phase may become durable before the preceding destination-directory
barrier. Every rename is within one parent filesystem. A normal reported error
enters rollback and returns only after rollback and its final directory flush.

## 5. Recovery decision

Recovery hashes every present destination, stage, and backup before choosing a
direction. Every file must match its journaled old or new digest/size according to
its role; any third value or missing sole copy is a hard error that preserves all
evidence.

- Phase Prepared through SidecarsPublished rolls back, except that a primary which
  already equals a distinct new digest proves a crash after the primary rename and
  forces roll-forward.
- Phase PrimaryPublished or Committed rolls forward.
- If old and new primary bytes are identical, phases 1..4 roll back and phases 5..6
  roll forward; either result preserves the primary bytes but uniquely resolves the
  decision and sidecars.

Rollback processes roles in reverse publication order. An old-present destination
is restored from a matching backup, or retained when the destination already equals
the old identity; an old-absent destination is removed only when it equals the new
identity. Rollback then removes matching stages and backups, verifies the complete
old set, removes both journal files, and flushes the directory.

Roll-forward processes publication order. A destination already matching new is
retained; otherwise its matching stage is renamed into place. It flushes each file
and the parent directory at the same boundaries as Section 4, installs any missing
later phase, verifies the complete new set, then performs committed cleanup.

Both paths are idempotent. A crash during recovery re-enters the same direction
from the surviving phase and digests.

## 6. Journal-free orphan rule

This rule is evaluated only for the intended set-id prefix, or for an intersecting
set id already discovered from a valid journal; unrelated reserved prefixes in the
same directory are neither opened nor changed. The command holds every destination
lock for the set while applying it, so it cannot mistake another live pre-Prepared
transaction for an orphan.

When neither valid journal file for that exact set id exists, backups are impossible
in a legitimate transaction and therefore cause a hard error. Well-formed stages
from a crash before `Prepared` may be removed only after the filename prefix and
transaction grammar match that full intended/discovered set id and no-follow
regular-file validation succeeds; CK then flushes the directory. A prefix collision
or malformed inspected entry is preserved and fails closed. No user destination is
changed under the journal-free rule.
