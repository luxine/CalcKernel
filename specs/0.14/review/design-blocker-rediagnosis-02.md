# CK 0.14 Design Blocker Rediagnosis 02

Review source: `design-adversarial-review-02.md`

Verdict: all six findings are confirmed blockers. The non-blocking ordering defect
is also accepted. None requires changing the product direction or lowering a
correctness, determinism, safety, performance, or release gate.

## R1. Session identity and search ties

Confirmed. A permutation seed must be reproducible from recorded, pre-measurement
identity only. Search entrants must also have a total ordering before truncation.

Resolution:

- define one domain-separated session digest over the decision identity, contract,
  workload, environment-before-calibration, frontier, and baseline object graph;
- exclude measurements, cache origin, temporary paths, and wall-clock timestamps;
- rank search entrants by score, actual bytes, choice count, and plan digest.

## R2. Smoke timeout phase

Confirmed.

Resolution:

- add candidate-smoke as ordering phase 1 and renumber the six warmup/measured
  phases to 2 through 7;
- record case and candidate smoke permutation/input identity;
- permit a timeout record at smoke or any later phase while storing samples only
  for measured phases 3, 5, and 7.

## R3. Effective environment count

Confirmed.

Resolution:

- the total effective environment bound remains 16;
- on Windows the compiler inserts `SystemRoot` and `WINDIR` first when required;
- the user allowlist may occupy only the remaining slots, after case-insensitive
  deduplication against base names;
- the manifest remains syntactically bounded at 16 but validation rejects an
  effective set above 16.

## R4. Canonical digests and collections

Confirmed.

Resolution:

- define a common digest function over domain bytes plus canonical typed value;
- freeze domains and canonical components for manifest, correctness aggregate,
  unit variant, plan/empty plan, candidate space, session, round, certificate,
  object graph, link recipe, output set, and replay result;
- sort site alternatives by site id then alternative id;
- aggregate per-case correctness as case-id/digest records;
- use the complete attachment encoding wherever a record digest is referenced.

## R5. Durable journal

Confirmed. An internal file format need not be a public compatibility promise, but
one implementation plan still requires exact bytes and barriers.

Resolution:

- add a closed journal schema with magic, version, complete destination records,
  fixed bounds, domain-separated digest, and exact sibling filenames;
- write journal generations through a sibling stage followed by file flush, atomic
  replacement, and parent-directory flush;
- place a directory durability barrier after each rename set and before the
  corresponding journal phase becomes durable;
- define cleanup and recovery for every phase and digest state.

## R6. Schema 9 closure

Confirmed.

Resolution:

- freeze external rotation domains and input serialization;
- freeze all seven workload-manifest logical records (five main and two domain),
  including runner args, inputs,
  timeout, search/validation cases, seeds, weights, and expected-digest provenance;
- pin x86-64-v4 and AArch64 SVE2 as the required stable performance tiers;
- add the two domain decisions, artifacts, build commands, and provenance to closed
  schema-9 evidence.

## Non-blocking correction

Both overview tables will say manifest order for declared inputs, matching the wire
authority.

## Acceptance rule

Revision 02 must be committed and reviewed by a third fresh ultra-reasoning Sol
agent. Planning remains prohibited until that review returns PASS with zero
blockers.

## Applied revision

The revision defines the session digest and total search ordering, adds the smoke
phase, closes the 16-entry effective environment, freezes all referenced digests
and collection orders, adds `publication-journal-1.md`, and closes schema-9
rotation/manifests/tiers/domain provenance. The English and Chinese designs share
the same three normative attachments. This diagnosis does not self-approve the
revision; review 03 remains the independent gate.
