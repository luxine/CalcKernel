# CK 0.14 Publication Journal Schema 1

Status: normative crash-consistency protocol shared by both language designs

This internal schema is not a promise that future CK versions will read a v0.14
journal. It is a complete implementation contract for v0.14 recovery. A command
must establish the stable overlap closure and recover or fail closed before it
reads, stages, or replaces any member of the intended destination set.

## 1. Names, ownership, and bounds

Let `set_id` be the full output-set digest from `decision-schema-1.md`, and `s` its
complete 64 lowercase hexadecimal characters. For every decision/artifact/sidecar
destination, let `destination_id = H("CK-TUNE-DESTINATION\0",
DestinationKeyMaterial)` and `d` be its complete 64 lowercase hexadecimal
characters. All files share the already-validated canonical output parent:

| Purpose | Basename |
| --- | --- |
| persistent lock for each destination | `.ckc-tune-dest-<d>.lock` |
| private lock initializer | `.ckc-tune-lock-init-<d>.<tx>.write` |
| active set journal | `.ckc-tune-set-<s>.journal` |
| set-journal update stage | `.ckc-tune-set-<s>.journal.new` |
| private journal write | `.ckc-tune-set-<s>.<tx>.<g>.write` |
| destination stage | `.ckc-tune-set-<s>.<tx>.<role>.stage` |
| destination backup | `.ckc-tune-set-<s>.<tx>.<role>.backup` |

`tx` is 32 lowercase hex characters encoding a fresh 128-bit operating-system
CSPRNG value; failure to acquire it aborts before staging. Role names are
`decision`, `header`, `import`, and `primary`. Only roles present in the output set
exist. CK reserves the exact basename grammar above. Each lock file contains exactly
`CKTLCK01` followed by its full 32-byte destination id; an existing lock with a
different id is a hard identity error. A colliding non-regular file,
symlink/reparse point, wrong owner/permissions, or unexpected transaction is a hard
error.

Destination locks are persistent regular files with owner-only write permissions.
To initialize one, CK create-news its private initializer, writes and flushes the
complete magic and id, then exposes the final lock name atomically only if absent
(POSIX hard-link/no-replace or the Windows fail-if-exists rename equivalent) and
flushes the directory. A loser removes its initializer and opens the winning final
file. A platform without a documented atomic no-replace primitive fails closed
before it creates a reserved file. Thus a final lock name is never partial. CK opens final locks no-follow,
validates on the same handle, acquires all required exclusive OS advisory locks in
canonical destination-id order, and holds them through discovery, recovery,
publication, and cleanup. It releases them in reverse order and never removes them,
preventing inode replacement and overlapping-set races. Stranded private lock
initializers have no authority and may be removed after their final lock is held.
Journal and transaction files are owner-only regular files opened no-follow. A
journal is at most 128 KiB and contains exactly one of these role layouts:
`decision,primary`; `decision,header,primary`; or
`decision,header,import-library,primary`. Thus destination count is exactly two,
three, or four. Each path byte string is at most 4,096 bytes.

### 1.1 Overlap-closure acquisition

Before mutation, CK computes the intended destination set and snapshots every valid
`.ckc-tune-set-*.journal`/`.journal.new` in the common parent. Starting with the
intended destination ids, it repeatedly adds all destinations from any journal that intersects
the current set, until no id is added. It then acquires the resulting destination
locks in canonical destination-id order and rescans. If a new or changed valid journal
expands the closure, CK releases in reverse order and retries. A malformed reserved
journal is a hard error.

Journal path bytes are operational rename evidence only. Intersection, equality,
ordering, and lock selection use the recomputed destination ids, so two spellings
that the parent filesystem resolves as one ASCII leaf can never form separate sets.

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
5. `U8 direction` where forward=1 and rollback=2;
6. 16 transaction-id bytes;
7. 32 full output-set-id bytes;
8. `U8 destination_count`;
9. that many `Destination` records in publication order;
10. `SHA-256("CK-TUNE-JOURNAL\0" || every preceding byte)`.

Integers are big-endian. A byte string is `U32 length || bytes`. Unix destination
bytes are exact non-NUL path bytes; Windows bytes are normalized absolute UTF-8.
A `Destination` is exactly:

| Field | Encoding |
| --- | --- |
| role | `U8`: decision=0, primary=1, header=2, import-library=3 |
| destination | canonical absolute path byte string |
| destination id | 32 bytes recomputed from the opened parent identity and lookup leaf |
| stage basename | ASCII byte string |
| backup basename | ASCII byte string |
| old present | `U8` exactly 0 or 1 |
| old digest | 32 bytes; all zero iff old is absent |
| old size | `U64`; zero when old is absent and permitted to be zero when present |
| new digest | 32 bytes |
| new size | `U64` |

Publication order is decision, header when present, import library when present,
then primary. Records use that order regardless of numeric role. Paths must resolve
to the same opened parent and reproduce the ids derived before locking. The role
layout, destination count, full output-set id, and recomputed `OutputSetMaterial`
must all agree. Duplicate destination ids,
unknown phases/roles, noncanonical strings, bad zero fields, trailing bytes, wrong
digest, wrong transaction filename, or a generation inconsistent with phase are
invalid.

Phases are `Prepared=1`, `BackedUp=2`, `DecisionPublished=3`,
`SidecarsPublished=4`, `PrimaryPublished=5`, and `Committed=6`. Generation starts
at 1 for forward Prepared and increments by exactly one for every installed
journal state. A forward transition advances to the next phase. Before any rollback
mutation, a forward journal at phase 1..4 transitions at the same phase to direction
rollback and the next generation. Rollback direction never returns to forward and
is invalid at phases 5..6. Consequently a valid forward state has
`generation == phase`, and a valid rollback state has `generation == phase + 1`.

## 3. Atomic journal update

To install generation `g`, CK create-news the unique private `.write`, writes and
hashes all bytes, flushes it to stable storage, reopens and validates it, then
atomically renames it without replacement to `.journal.new` and flushes the parent.
Only a complete valid file can therefore acquire the `.journal.new` name. CK then
atomically replaces `.journal` with `.journal.new` and flushes the parent again;
Windows uses documented fail-if-exists and replace/write-through equivalents. It
never overwrites active journal bytes in place. A platform lacking either required
atomic primitive fails before staging.

With the destination-lock closure held, pre-mutation recovery uses this exhaustive
metadata table. “Valid successor” means same transaction id, set id, and destination
records, generation exactly one higher, and either the next forward phase or the
single permitted forward-to-rollback transition.

| Active | Update | Private write(s) | Required action |
| --- | --- | --- | --- |
| valid | absent | any | use active; remove same-set private writes |
| absent | valid forward Prepared generation 1 | any | promote update to active; remove private writes |
| valid | valid successor | any | promote update to active; remove private writes |
| absent | absent | present | treat as pre-Prepared orphan for the exact intended set; remove writes/stages and flush |
| malformed | any | any | preserve evidence and fail closed |
| any | malformed | any | preserve evidence and fail closed |
| absent | valid but not initial Prepared | any | preserve evidence and fail closed |
| valid | valid non-successor | any | preserve evidence and fail closed |

An update final name cannot represent an interrupted write. A private write has no
authority and is never parsed as a journal. Set discovery reads only final active
and update names; if a private write exists beside an active journal, that active
journal supplies the destination closure.

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
before primary publication first installs rollback direction and returns only after
rollback and its final directory flush. At or after primary publication, CK instead
finishes idempotent roll-forward and then reports the already-published result or a
cleanup diagnostic; it never attempts an unsafe late rollback.

## 5. Recovery decision

Recovery first resolves the metadata table, then hashes every present destination,
stage, and backup. Every file must match its journaled old or new digest/size
according to its role; any third value or missing sole copy is a hard error that
preserves all evidence. Direction is then exhaustive:

- rollback direction always continues rollback and is valid only at a recorded
  phase 1..4;
- forward phase 5..6 rolls forward;
- forward phase 1..4 with a primary matching a distinct new identity proves the
  primary rename crossed its barrier and rolls forward;
- every other forward phase 1..4 state first durably installs rollback direction,
  then rolls back;
- when old and new primary identities are equal, forward phases 1..4 select
  rollback and phases 5..6 select roll-forward.

Rollback processes roles in reverse publication order. An old-present destination
is restored from a matching backup, or retained when the destination already equals
the old identity; an old-absent destination is removed only when it equals the new
identity. Rollback then removes matching stages and backups, verifies the complete
old set, removes both journal files, and flushes the directory.

Roll-forward processes publication order. A destination already matching new is
retained; otherwise its matching stage is renamed into place. It flushes each file
and the parent directory at the same boundaries as Section 4, installs any missing
later phase, verifies the complete new set, then performs committed cleanup.

Both paths are idempotent. A crash during rollback re-enters with durable rollback
direction regardless of which old files have already been restored; a crash during
roll-forward re-enters forward. File digests determine the next missing rename but
never reverse the journaled recovery direction.

## 6. Journal-free orphan rule

This rule is evaluated only for the intended full set id, or for an intersecting
set id already discovered from a valid journal; unrelated reserved prefixes in the
same directory are neither opened nor changed. The command holds every destination
lock for the set while applying it, so it cannot mistake another live pre-Prepared
transaction for an orphan. Every reserved stage/backup basename contains the
complete 64-hex set id, so no other set can share this namespace.

When neither valid journal file for that exact full set id exists, backups are impossible
in a legitimate transaction and therefore cause a hard error. Well-formed stages
from a crash before `Prepared` may be removed only after the filename's full set id
and transaction grammar match that intended/discovered set id and no-follow
regular-file validation succeeds; CK then flushes the directory. A malformed
inspected entry is preserved and fails closed. No user destination is changed under
the journal-free rule.
