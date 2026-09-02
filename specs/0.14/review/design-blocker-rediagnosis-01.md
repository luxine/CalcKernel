# CK 0.14 Design Blocker Rediagnosis 01

Review source: design-adversarial-review-01.md

Verdict: all seven findings are confirmed blockers. None requires changing the
chosen product architecture or lowering a performance, safety, determinism, or
release threshold.

## R1. Manifest and runner identity

Confirmed.

Repository evidence:

- physical output kind is already a CLI/build property;
- v0.13 profile inputs use closed identity and no-follow path validation;
- executing a later path lookup after hashing creates a real time-of-check/time-of-
  use gap.

Resolution:

- artifact kind comes only from ckc tune build;
- canonical manifest encoding contains logical fields, relative staged-input names,
  effective allowlisted environment values, and content digests, never the runner's
  operational absolute path;
- the host-native runner is copied from a no-follow handle into a private executable
  snapshot once and that snapshot is used throughout the session.

## R2. Process containment

Confirmed with a scope correction.

The compiler cannot truthfully promise sandbox containment for hostile same-user
code on Darwin or Linux. It can provide reliable lifecycle management for a
cooperative harness.

Resolution:

- Windows uses a non-breakaway kill-on-close Job Object;
- Linux and Darwin use a dedicated process group;
- the harness contract forbids daemonization, setsid, double fork, breakaway, and
  inherited background work;
- timeout escalates from graceful termination to forced termination with fixed
  bounds, reaps the direct runner, and verifies the cooperative group has exited;
- an intentionally escaped process is outside the explicitly stated no-sandbox
  boundary.

## R3. Search algorithm

Confirmed.

Actual linked bytes cannot rank the precompile frontier. The current KIR transaction
layer can instead supply predicted dynamic/static cost and deterministic KIR growth
before the CLI selects a bounded materialization set.

Resolution:

- a tuning unit exposes at most four coherent unit variants, not a Cartesian product
  invented by the CLI;
- baseline carry is free; every attempted nonbaseline extension consumes one plan
  expansion;
- beam formation and diversity use predicted metrics only;
- selected compile slots count even on a tuning-cache hit;
- actual bytes enter only postcompile rejection and finalist ranking;
- all stop, duplicate, cap, and ordering rules receive pseudocode.

## R4. Measurement state machine

Confirmed.

Resolution:

- calibration starts at one iteration, permits at most 32 baseline attempts, doubles
  with checked u64 arithmetic, accepts the first at least 50 ms result, and records
  overshoot;
- one confirmation invocation follows calibration;
- every candidate runs one smoke invocation for every search case;
- warmups use one invocation per evaluation, scored rows use three and store the
  minimum;
- validation uses the already fixed per-case iteration count and independent
  phase/round permutation domains;
- an evaluation is never started unless the full configured timeout remains;
- session deadline is incomplete evidence, while only the full configured candidate
  timeout is a performance rejection.

## R5. Wire schema

Confirmed.

Resolution:

- add complete tag tables for identity, contract, workload, environment, frontier,
  candidate/measurement, selection/certificate, and replay/output records;
- freeze enum values, limits, ordering, optional encoding, samples, and trace bounds;
- require an outer-framing vector plus complete baseline and tuned golden fixtures
  before the format phase can pass.

The golden fixture bytes are implementation deliverables, but their logical records,
file locations, and required digest assertions are fixed by the design and may not
be chosen differently by an implementation plan.

## R6. Output set and journal

Confirmed.

Repository evidence:

- NativeArtifactPaths emits a primary plus a header for dynamic libraries and a
  Windows import library;
- OutputTransaction is same-process best-effort rollback, not durable recovery.

Resolution:

- the output set is decision, primary, dynamic header, and Windows import library
  as applicable;
- the default decision suffix applies to the resolved primary path;
- schema 1 requires one canonical parent directory;
- an output-set hash names one lock and journal;
- staged files, backups, digest validation, phases, fsync order, primary-last commit,
  rollback-before-primary, and roll-forward-after-primary are all fixed.

## R7. Performance schema 9

Confirmed.

Resolution:

- schema 9 embeds and revalidates cumulative schema-8 evidence;
- it reuses the five frozen PGO sources and adds fixed tuning split/manifests and
  explicit SIMD oracle assets;
- search, validation, and release-held-out records are distinct;
- exact channels, rows, calls, fields, retained artifacts, compilation/resource
  measurements, and independent checker behavior are frozen;
- the final accepted v0.13 commit is an explicit prerequisite recorded by a
  repository baseline manifest, not an implementation-time discretionary choice.

## Non-blocking corrections adopted

- environment names and absence behavior are closed;
- only a same-v0.14-identity profile is accepted;
- original output-pair digests are distinguished from replay packaging identity;
- origin-session cache facts remain immutable on a warm decision reuse;
- diagnostic-only continue-on-error does not violate the required-gate rule.

## Revision acceptance rule

The revised English and Chinese designs must describe identical constants and
behavior. After revision, a fresh ultra-reasoning Sol review is required. Planning
cannot begin until that reviewer returns PASS with zero blockers.

## Applied revision

The revision is now represented by:

- `specs/0.14/offline-autotuning.md`;
- `specs/0.14/zh-CN/offline-autotuning.md`;
- the shared normative `specs/0.14/decision-schema-1.md` wire contract;
- the shared normative `specs/0.14/performance-schema-9.md` evidence contract.

It closes the runner snapshot and manifest authority, cooperative containment,
fully accounted deterministic beam algorithm, fixed invocation state machine,
tag-by-tag decision format, durable complete-output-set transaction, and frozen
schema-9 evidence contract described above. This diagnosis does not self-approve
the revision; review 02 remains the required independent gate.
