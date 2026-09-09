# CK 0.12 Implementation Blocker 03: Canonical Repository Rename and Fixed Oracle

## Decision

This blocker is accepted as a repository-identity and remote-quality-gate defect exposed after
GitHub renamed the Rust repository from `luxine/Rust_CalcKernel` to `luxine/CalcKernel`. It is not
a CK 0.12 language, ABI, optimizer, benchmark, threshold, or artifact counterexample.

Candidate `1c2596da11242704cc6d875e969fc45cf58ea21d` has no exact-SHA workflow run. Dispatching its
current `.github/workflows/ci.yml` would not be a valid acceptance attempt: the quality job checks
out `repository: luxine/CalcKernel` at retired TypeScript commit
`5e989939d89d75056e5f3bea25f3bf7204d5529a`. That repository coordinate now names the Rust
repository, whose Git object database does not contain the requested commit.

The same defect was reproduced and closed for CK 0.13 by
`specs/0.13/review/implementation-blocker-01.md` and candidate
`94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`. CK 0.12 adopts the same test-only oracle contract;
it does not invent a second provenance or relax the older schema-7 acceptance gate.

## Rediagnosis

- Replacing the checkout coordinate with `luxine/CalcKernel_retire` is not sufficient. That
  repository is private and the Rust repository has no narrowly scoped cross-repository Actions
  credential. The default workflow token cannot read it.
- The public `calckernel@0.8.0` package is not the retired compiler used by the differential gates.
  It exposes `npm/index.js` and lacks `dist/src/cli.js`, the source tree, and the registered fixture
  roots.
- Removing `CALCKERNEL_TS_ROOT`, permitting oracle readiness tests to return early, deleting the
  C/WebAssembly/CLI differential tests, or making the quality job optional would lower a frozen
  acceptance gate and is forbidden.
- Making the retired repository public or installing a broad personal token would add external
  authority, secret rotation, and availability requirements that are unnecessary for a fixed test
  oracle.
- Reusing generated `dist/` or `node_modules/` would make source provenance and reproducibility
  unauditable. CI must rebuild from source and the frozen lockfile.

## Closed design

The minimum retired TypeScript compiler source and every CK fixture already consumed by the v0.12
differential suites are stored under `tests/oracles/typescript`. The snapshot is byte-identical to
the already reviewed CK 0.13 snapshot and fixes:

- origin repository: `https://github.com/luxine/CalcKernel_retire`;
- origin commit: `5e989939d89d75056e5f3bea25f3bf7204d5529a`;
- origin tree: `445743ef4d270ba7a26a5402243ce0bb606fb44b`;
- original `package.json`, `pnpm-lock.yaml`, `tsconfig.json`, source tree, and exactly the fixtures
  reached by the Rust differential suites;
- `PROVENANCE.md` and an 85-entry `SOURCE_MANIFEST.sha256` covering every included source,
  configuration, lock, and fixture byte sequence.

`node_modules/` and `dist/` remain ignored and are never committed. The native compiler, released
archives, CK ABI, cache identities, benchmark inputs, and performance reports do not include or
consume this test-only snapshot.

The quality job:

1. checks out only the candidate repository;
2. sets `CALCKERNEL_TS_ROOT` to `${{ github.workspace }}/tests/oracles/typescript` only in quality;
3. runs `sha256sum --check SOURCE_MANIFEST.sha256` in that directory;
4. activates the pinned pnpm version;
5. runs `pnpm install --frozen-lockfile --ignore-scripts`;
6. builds the oracle from the verified source;
7. runs the unchanged default suite, including live C, WebAssembly, CLI-readiness, fixture-coverage,
   and runtime differential gates.

Release verification remains self-contained and does not build or ship the optional test oracle.

## Alternatives rejected

1. **Private retired-repository checkout with PAT:** rejected because it introduces secret and
   cross-repository authority, availability, and rotation requirements for immutable test data.
2. **Registry artifact:** rejected because the published artifact is not the frozen compiler or
   fixture corpus required by the existing tests.
3. **Remove or skip the oracle:** rejected because it lowers a required quality gate.

## TDD and implementation sequence

1. Add a focused repository-oracle contract test that requires `PROVENANCE.md`, exactly 85 manifest
   entries, canonical origin commit/tree, and required source/fixture roots. Run it and record RED
   because `tests/oracles/typescript` does not exist in the v0.12 candidate.
2. Change the CI contract test to require the repository-owned path, manifest verification,
   frozen script-disabled install, and unchanged live Rust gates, while forbidding the second
   repository checkout. Run it and record RED against the current workflow.
3. Copy the already reviewed snapshot byte-for-byte from CK 0.13 candidate `94aad2d...`; do not
   edit oracle source or fixtures. Add only its generated/dependency directories to `.gitignore`.
4. Update only the quality-job bootstrap and its scoped `CALCKERNEL_TS_ROOT`. Do not change any
   schema-7 job, performance threshold, host matrix, required-job topology, or diagnostic policy.
5. Run manifest verification, frozen install/build, focused repository/CI contracts, and the live
   C/WebAssembly/CLI/fixture differential tests. Then run all Stage 10 local commands.
6. Update current repository identity in README/Cargo metadata and release checklists where stale
   canonical GitHub links remain; historical path examples and tests that deliberately reject the
   old URL remain as history or negative fixtures.
7. Commit and push one new v0.12 candidate SHA, dispatch exactly one ten-job candidate workflow,
   update the low-frequency heartbeat to that run/SHA, and preserve all failures as evidence.

## Acceptance boundary

- No normative CK 0.12 specification, corpus, threshold, statistic, sampling order, replay SHA,
  required host/job, fail-closed behavior, or exact-SHA rule changes.
- The source manifest must verify 85/85 entries before dependency installation.
- The quality job must execute the same live oracle-dependent tests rather than merely compile the
  snapshot.
- Default and all-feature local tests, release build, sanitizer contract, release/native/JIT audits,
  format, Clippy, and diff gate must remain green.
- Stage 10 and total CK 0.12 acceptance remain pending until the new exact-SHA ten-job workflow is
  fully successful. This design document does not authorize merging v0.12 before that result.

## Self-review

- Placeholder scan: no unfinished marker, unknown path, unresolved SHA, or optional required gate
  remains.
- Consistency: the snapshot identity and workflow sequence match the reviewed CK 0.13 closure;
  v0.12 retains its own schema-7 performance and exact replay contracts.
- Scope: only repository identity, test-oracle provenance, quality bootstrap, contracts, and
  current repository documentation change.
- Ambiguity: generated outputs are explicitly excluded; the manifest is checked before install;
  one new candidate receives one complete workflow; no old result may be spliced into acceptance.

Verdict: the design is internally closed and does not lower the v0.12 acceptance contract.
