# Phase 21 Rename Release Readiness Report

Date: 2026-06-26

Scope: final rename regression, snapshot review, package fresh-install smoke,
package contents review, naming scan, and release readiness assessment for the
legacy IK / IntKernel to CK / CalcKernel rename.

This phase did not change compiler semantics, parser behavior, type checking,
MIR lowering, optimizer behavior, C/WASM/LLVM backend semantics, package
publishing state, git tags, benchmark baselines, or benchmark thresholds.

## Files Changed In This Phase

- Added this report:
  `docs/plans/PHASE_21_RENAME_RELEASE_READINESS_REPORT.md`

No functional source files, snapshots, tests, package metadata, or docs were
changed during Phase 21.7.

## Full Verification

| Command | Result |
| --- | --- |
| `pnpm typecheck` | Passed. `tsc -p tsconfig.json --noEmit` exited 0. |
| `pnpm build` | Passed. `tsc -p tsconfig.json` exited 0. |
| `pnpm test` | Passed. 58 test files and 407 tests passed. |
| `npm pack --dry-run` | Passed. Reported `calckernel@0.5.0`, 268 files. |

The test run included naming consistency, package-bin smoke, C e2e, checked C
e2e, WASM e2e, LLVM e2e, backend regression comparison, f64 strict coverage,
and explicit cast regression coverage.

## Package Fresh Install Smoke

The smoke used a real package tarball and a temporary install directory outside
the repository:

- `npm pack` produced `calckernel-0.5.0.tgz`.
- Temporary install directory: `/tmp/ck-fresh-smoke.jNRl7p`.
- Both the tarball and temporary directory were removed by the cleanup trap.

Fresh install checks:

| Check | Result |
| --- | --- |
| `node_modules/.bin/ckc` exists | Passed. |
| `node_modules/.bin/ikc` does not exist | Passed. |
| `node_modules/.bin/ckc --help` contains `ckc` | Passed. |
| `node_modules/.bin/ckc check smoke.ck` | Passed. |
| `node_modules/.bin/ckc emit-mir smoke.ck -o smoke.mir` | Passed, non-empty output. |
| `node_modules/.bin/ckc emit-c smoke.ck -o smoke.c` | Passed, non-empty C and header output. |
| `node_modules/.bin/ckc emit-wat smoke.ck -o smoke.wat` | Passed, non-empty output. |
| `node_modules/.bin/ckc emit-wasm smoke.ck -o smoke.wasm` | Passed, non-empty output. |
| `node_modules/.bin/ckc emit-llvm smoke.ck -o smoke.ll` | Passed, non-empty output. |
| `node_modules/.bin/ckc build-llvm smoke.ck --kind object -o smoke.o` | Passed; clang was available. |

Additional ABI checks:

- Unchecked generated C header contains `CK_API`.
- Checked generated C header contains `CK_API` and `CK_Status`.
- Checked generated C source contains `CK_OK` / `CK_ERR_*`.
- Generated C/header smoke outputs do not contain `IK_`.
- Smoke MIR/help/C/header/WAT/LLVM outputs do not contain user-visible legacy
  project names, command names, or `.ik` source references.

Legacy extension check:

- `pnpm ckc check examples/pricing.ik` exits 1 as expected.
- Diagnostic text: `CalcKernel source files use .ck. Legacy .ik files are no longer accepted.`

## Package Contents Review

`npm pack --dry-run` after adding this report showed:

- package: `calckernel@0.5.0`
- files: 268
- includes `docs/MIGRATION_IK_TO_CK.md`
- includes this release readiness report
- includes `.ck` examples such as `examples/pricing.ck`
- includes no `.ik` example files

No package-content violations were found for:

- `node_modules`
- `coverage`
- cache directories
- debug logs
- tarballs
- temporary smoke directories
- `build/perf/latest.*`
- local benchmark baseline output under `build/perf`

## Snapshot Review

Snapshot diff scope:

- 21 snapshot files changed.
- 176 insertions and 176 deletions.

Expected snapshot changes:

- LLVM snapshots changed `source_filename` from `.ik` to `.ck`.
- LLVM snapshots include `ModuleID = 'calckernel'`.
- C/header snapshots changed public ABI names from `IK_*` to `CK_*`.
- Checked C snapshots changed status type and macro references from
  `IK_Status`, `IK_OK`, and `IK_ERR_*` to `CK_Status`, `CK_OK`, and `CK_ERR_*`.

No snapshot review evidence showed language semantic, optimizer semantic, or
backend behavior changes beyond rename text and public ABI prefixes.

## Naming Scan

The scan searched for:

- `IK`
- `IntKernel`
- `ikc`
- `.ik`
- `intkernel`
- `IK_`
- `tkc`
- `.tk`
- `LK`
- `lkc`
- `.lk`

Classification summary:

| Category | Result |
| --- | --- |
| User-visible current docs/source/tests/package surface | No hits. |
| Migration guides | Allowed legacy mapping references. |
| Historical docs/plans | Allowed historical references. |
| Naming consistency tests | Allowed forbidden-rule definitions. |
| Policy guardrail text | `AGENTS.md` mentions `tkc` and `.tk` only as prohibited aliases. |
| Errors requiring fix | None. |

Raw count summary:

| Category | Name | Count |
| --- | --- | ---: |
| migration guide | `.ik` | 10 |
| migration guide | `IK` | 8 |
| migration guide | `IK_` | 36 |
| migration guide | `IntKernel` | 10 |
| migration guide | `ikc` | 10 |
| migration guide | `intkernel` | 8 |
| historical docs | `.ik` | 146 |
| historical docs | `.lk` | 6 |
| historical docs | `.tk` | 6 |
| historical docs | `IK` | 67 |
| historical docs | `IK_` | 79 |
| historical docs | `IntKernel` | 231 |
| historical docs | `LK` | 7 |
| historical docs | `ikc` | 40 |
| historical docs | `intkernel` | 138 |
| historical docs | `lkc` | 6 |
| historical docs | `tkc` | 6 |
| naming test | `.ik` | 2 |
| naming test | `.lk` | 1 |
| naming test | `.tk` | 1 |
| naming test | `IK` | 2 |
| naming test | `IK_` | 8 |
| naming test | `IntKernel` | 1 |
| naming test | `ikc` | 2 |
| naming test | `intkernel` | 1 |
| policy guardrail | `.tk` | 2 |
| policy guardrail | `tkc` | 2 |

The current user-visible scan excludes migration guides, historical docs,
naming-test rule definitions, and package lockfiles. That scan returned no
hits.

## Version And Release Recommendation

The current package version is `0.5.0`.

No `v0.6.0` tag was found in the current checkout. No `v0.7.0` tag was found.

Because this rename is user-visible and breaking, the recommended rename release
version is `v0.7.0` if Phase 20 has already been released as `v0.6.0`.

If Phase 20 `v0.6.0` has not been tagged yet, recommended options are:

1. Tag/freeze Phase 20 as `v0.6.0` first, then release the rename as `v0.7.0`.
2. Explicitly decide to fold the rename into the next breaking release and bump
   directly according to that release plan.

Do not publish `calckernel` until npm package-name availability is checked by a
human.

## Readiness Assessment

Technical readiness: ready for final rename acceptance review.

Open release decision:

- package version and release tag strategy still need an explicit human
  decision because the current checkout has no `v0.6.0` tag and package version
  remains `0.5.0`.

Blockers:

- No technical blocker found in regression, fresh-install smoke, package
  contents, snapshots, or current user-visible naming scan.
- Release/version/tag decision remains a process blocker before publishing or
  tagging.
