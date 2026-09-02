# CK 0.13 Implementation Blocker 01: Repository Rename and TypeScript Oracle

## Decision

The blocker reproduces and is accepted as a stage-11 remote-environment and
repository-identity defect. It is not a CK language, ABI, optimizer, benchmark,
or threshold counterexample.

The first candidate run at
`https://github.com/luxine/CalcKernel/actions/runs/33576019981` failed in the
quality job while its second checkout tried to resolve
`5e989939d89d75056e5f3bea25f3bf7204d5529a` from `luxine/CalcKernel`. That
commit belongs to the retired TypeScript implementation. Renaming the Rust
repository to `CalcKernel` and the old repository to `CalcKernel_retire` made
the prior checkout coordinate point at the wrong Git object database.

## Rediagnosis

- Changing the checkout to `luxine/CalcKernel_retire` is insufficient: the
  retired repository is private and this repository has no narrowly scoped
  cross-repository Actions credential. The default job token cannot read it.
- The public `calckernel@0.8.0` registry artifact is not the historical
  TypeScript compiler used by these gates. Its manifest points to `npm/index.js`
  and does not contain `dist/src/cli.js`, the source tree, or the fixture roots
  required by the live differential suites.
- Removing `CALCKERNEL_TS_ROOT`, allowing the tests to return early, or deleting
  the differential tests would lower the already accepted quality gate and is
  forbidden.
- Making the retired repository public or copying a broad personal token into
  Actions would expand remote authority beyond this implementation task.

## Closed implementation contract

The minimum TypeScript compiler source and every CK fixture already registered
by the differential suites are copied byte-for-byte from the detached retired
commit into `tests/oracles/typescript`. `PROVENANCE.md` fixes the origin commit
and tree, `SOURCE_MANIFEST.sha256` fixes every included byte sequence, and the
original pnpm lockfile fixes all build dependencies. Generated output and
dependency directories remain ignored.

The quality job verifies the source manifest, performs a frozen install with
scripts disabled, builds the oracle, sets `CALCKERNEL_TS_ROOT` to that exact
repository-owned directory, and runs the same C, WebAssembly, CLI-readiness,
fixture-coverage, and runtime differential tests. Release verification remains
self-contained and never consumes this test-only oracle.

No normative CK specification changes. No corpus, threshold, statistic,
required job, fail-closed rule, or exact-candidate-SHA requirement changes.
