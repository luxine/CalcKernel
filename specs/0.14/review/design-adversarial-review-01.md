# CK 0.14 Design Adversarial Review 01

Review target: commit 2f58e2a74749fef3fa113d115bdf8e310516d159

Reviewer: read-only gpt-5.6-sol, ultra reasoning

Verdict: BLOCKED

The review deliberately ignored stylistic preferences and reported only issues
that prevent one correct implementation, break an existing contract, leave a
cross-platform guarantee impossible, or make release acceptance non-decidable.

## Blocking findings

### B1. Workload schema and launched runner identity are not closed

The schema table does not contain an artifact-kind field, while the harness
section says the manifest declares the kind. The CLI already owns the physical
artifact kind, so an implementation would have to invent a conflict rule.

The runner is digested once but is then described as executing from its original
operational path. Replacing that file after baseline calibration could mix
different harness implementations under one recorded identity.

Required correction:

- make the tune CLI the sole artifact-kind authority;
- define canonical manifest identity independently of operational absolute paths;
- execute one no-follow, digest-verified immutable runner snapshot for the complete
  session.

### B2. Complete-process-tree termination is impossible under the stated threat model

The design allows arbitrary user-authorized runner code, explicitly provides no
sandbox, and nevertheless promises to terminate and reap a complete process tree
on all six platforms. A POSIX child can detach with setsid or a double fork, and
Darwin has no Windows Job Object equivalent that makes an arbitrary process tree
inescapable.

Required correction:

- define a cooperative containment contract;
- freeze Windows Job Object and POSIX process-group behavior;
- require the runner and descendants not to detach;
- treat malicious containment escape as outside the no-sandbox contract.

### B3. Beam search does not define one implementable algorithm

Actual artifact size appears in candidate rank before the bounded compile set has
been chosen. Unit expansion, baseline carry, beam truncation, diversity when class
count exceeds beam width, cache-hit accounting, illegal candidates, and cap
exhaustion are unspecified.

Required correction:

- provide complete deterministic pseudocode;
- use predicted KIR/code-size units before compilation;
- use actual artifact bytes only after the bounded compile set is materialized;
- freeze all accounting, truncation, diversity, duplicate, cache, and over-limit
  rules.

### B4. Measurement state machine and wall-clock behavior are incomplete

Calibration refers to an absent preset limit and does not freeze the starting
iteration count, doubling limit, overshoot behavior, or confirmation. Correctness
smoke has no invocation matrix. Remaining session time is allowed to shorten an
invocation timeout, which is indistinguishable from a true candidate timeout.
Validation calibration and permutation domains are also undefined.

Required correction:

- define the complete state machine and constants;
- define every calibration, smoke, warmup, search, and validation invocation;
- derive every order from explicit phase and round domains;
- make insufficient session time an incomplete-evidence error;
- classify a candidate timeout only after the complete configured invocation
  timeout elapses.

### B5. The .cktune description is not a complete wire schema

Only the eight outer payload topics are named. Nested tags, field types, enums,
required and optional fields, bounds, collection ordering, matrix dimensions, and
object-graph/link-recipe encoding remain unspecified. Two incompatible encoders
could both claim schema 1 conformance.

Required correction:

- freeze every nested record and enum;
- freeze every allocation and collection bound;
- define canonical ordering and matrix dimensions;
- require framing and complete-decision golden vectors.

### B6. Publication omits dynamic-library sidecars and crash recovery states

Existing dynamic output includes a header on all platforms and an import library
on Windows. Publishing only one primary artifact and one decision can pair a new
library with stale ABI sidecars.

The journal has no location, discovery, lock, filesystem, fsync, phase, or recovery
contract. The existing OutputTransaction provides only in-process best-effort
rollback.

Required correction:

- define the complete physical output set;
- bind every output digest and size in the decision;
- constrain all outputs to one parent for schema 1;
- define lock/journal naming, durable phases, publish order, and deterministic
  recovery for every crash point.

### B7. Schema 9 performance acceptance cannot be independently checked

The design names schema 9 and thresholds but does not freeze report fields, corpus,
split identities, tune manifests, eligibility, baselines, audited SIMD oracles,
sampling channels, retained evidence, or checker inputs. Existing schema 8 is much
more specific.

Required correction:

- make schema 9 a cumulative extension of schema 8;
- freeze repository paths and logical corpus records;
- freeze channel/sample/statistic and exact report contracts;
- define the independent fail-closed checker.

## Important non-blocking findings

- Freeze inherited-environment syntax, Windows case-insensitive uniqueness, and
  absent-variable behavior.
- Clarify that an accepted .ckprof must match the v0.14 compiler and schema identity;
  “already collected” must not imply accepting a v0.13 profile.
- Distinguish original output-pair byte digests from tune-use replay identity when
  packaging or signing bytes are deliberately excluded.
- Clarify that cache facts stored in a reused decision describe its origin session;
  current warm-hit diagnostics do not rewrite the decision.
- Limit the no-continue-on-error rule to required gate steps; diagnostic capture may
  remain best effort.

## Confirmed closed areas

- The tune command can be integrated as a pgo-style nested command without changing
  the source-language parser.
- Ordinary-build isolation and the O3/host/cpu-native/sanitizer/multiversion matrix
  are closed.
- Legality/profitability separation and non-publishable trial typestate fit the
  existing optimizer transaction/checker direction.
- Q32 ratio, upper median, stability interval, validation threshold, and tie-break
  arithmetic are internally coherent.
- Optional-profile topology and executable/dynamic Native consumers are reachable.
- CKCOBJ04/schema 5 and schema-4 clean-miss behavior are explicit.
- The ten required job count matches the current workflow.
- English and Simplified Chinese contain no material policy divergence.

Blockers: 7

The reviewer left the worktree clean at the reviewed commit.
