# CK 0.14 Design Adversarial Review 05

Review target: commit cd73cae01b87f6af6ab5119ff386144256dbf0c2

Reviewer: fresh read-only gpt-5.6-sol, ultra reasoning

Verdict: BLOCKED

Blockers: 11

## Blocking findings

### B1. Filesystem lookup equivalence remains incomplete

Destination identity does not discuss Windows 8.3 alternate names, and logical
input paths are not checked under the lookup semantics of the actual staging
filesystem. Byte-distinct input paths can overwrite one another in a
case-insensitive temporary tree.

Minimum correction: close alternate-name handling or fail closed, and validate
every staged input component under the destination filesystem's lookup semantics.

### B2. Smoke and terminal states conflict

The main design smokes every compiled candidate, while the schema forbids
correctness on `compiled-unmeasured` and `size-rejected`. A candidate rejected for
size after compilation cannot be represented if smoke already ran.

Minimum correction: smoke only size-valid measured finalists, or retain smoke
correctness in those terminal states.

### B3. Validation summaries are not derived from streams

Round medians, Q32 ratios, aggregate score, stability, paired wins, entrants, and
rank are not explicitly bound to the retained raw measurement streams.

Minimum correction: define the full deterministic derivation and foreign keys.

### B4. Allowlisted secrets are retained and inspected

The decision stores environment values and inspection renders the complete tree,
contradicting the promise that decisions contain no secrets.

Minimum correction: retain only name, length, and a domain-separated value digest;
keep execution values only in private session state.

### B5. Windows argv execution is ambiguous

The source model promises exact UTF-8 argv bytes, but Windows process creation
uses one UTF-16 command line whose quoting rules determine the child's argv.

Minimum correction: freeze Unicode conversion plus Windows quoting/parsing ABI or
use an unambiguous transport.

### B6. The cumulative schema-8 gate cannot use the old checker as written

The current schema-8 checker requires candidate version 0.13.0 and report SHA equal
to current HEAD, so a historical accepted report cannot pass in a v0.14 checkout.

Minimum correction: separate immutable v0.13 replay acceptance from a v0.14
cumulative compatibility run and freeze both checker modes.

### B7. External calls are not bound to equal work

The report retains 20x7 timings but no per-case iteration count or calibration
evidence. Channels could time different work amounts.

Minimum correction: retain and verify one calibrated `iterationsPerCall` per
case/split, shared by every channel, warmup, and measured call.

### B8. External correctness is not bound to the expected result

Per-channel equality accepts a shared wrong digest unless it also equals the
manifest or frozen release expected digest.

Minimum correction: bind validation to manifest expected digests and held-out/domain
rows to the independently derived frozen expected digest.

### B9. Build commands and decision leaves do not close channel semantics

Generic commands do not fix compiler, profile, flags, cache state, or all repeated
compile invocations. Decoded decision identity is not tied leaf-by-leaf to report
compiler, source, manifest, runner, target, and profile evidence.

Minimum correction: freeze channel recipes and invocation records and add the
complete decision-to-report identity graph.

### B10. Cold/warm determinism lacks run provenance

Result equalities alone do not prove two independent empty-cache runs and one warm
reuse; command, cache namespace/state, raw counts, and logs are absent.

Minimum correction: add an authoritative `TuneRun` record with exact command,
isolated cache identity and pre/post state, counters/log, decision, and outputs.

### B11. Archive-size endpoints are unconstrained

The v0.13 side is not required to equal the replay archive, and the candidate side
has no deterministic packaging provenance or member closure.

Minimum correction: bind both endpoints to their canonical archives and retain the
candidate packaging command and complete member manifest.

## Confirmed closures

The reviewer confirmed the rotated-timeout set, validation reason table, primary
digest materials, inspection grammar, durable journal direction and atomic update,
full-id naming, pre-tune phase order, 20x7 arrays, and file-root discriminator.
English and Chinese had no material divergence. Same-byte primary recovery and the
empty-plan case were not blockers.

Final verification showed the exact requested HEAD, a clean worktree and index,
and no reviewer edits.
