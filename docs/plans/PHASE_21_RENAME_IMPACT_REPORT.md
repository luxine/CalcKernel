# Phase 21.1 Rename Impact Report

Date: 2026-06-26

Scope: impact-only scan for the destructive rename from IK / IntKernel to CK /
CalcKernel. This phase does not modify compiler code, docs, tests, snapshots,
examples, generated artifacts, package metadata, or benchmark baselines.

This report is intentionally the only Phase 21.1 project file change.

## Rename Contract

Required mapping:

| Legacy name | New name |
| --- | --- |
| `IK` | `CK` |
| `IntKernel` | `CalcKernel` |
| `ikc` | `ckc` |
| `.ik` | `.ck` |
| `intkernel` | `calckernel` |
| `IK_` | `CK_` |
| `IK_API` | `CK_API` |
| `IK_BUILD_DLL` | `CK_BUILD_DLL` |
| `IK_Status` | `CK_Status` |
| `IK_OK` | `CK_OK` |
| `IK_ERR_OVERFLOW` | `CK_ERR_OVERFLOW` |
| `IK_ERR_DIV_BY_ZERO` | `CK_ERR_DIV_BY_ZERO` |
| `IK_ERR_NULL_POINTER` | `CK_ERR_NULL_POINTER` |

Names that remain forbidden:

- `tk`
- `tkc`
- `.tk`
- `LK`
- `lkc`
- `.lk`

Default compatibility position:

- Do not retain an `ikc` alias.
- Do not retain `.ik` compatibility.
- Do not retain `IK_` C ABI aliases.

## Scan Method

The scan excluded generated/dependency directories:

```sh
node scan script over repository files, excluding:
.git/
node_modules/
build/
dist/
coverage/
```

Patterns counted:

- `\bIK\b`
- `IntKernel`
- `\bikc\b`
- `\.ik\b`
- `\bintkernel\b`
- `IK_`
- `IK_Status`
- `IK_OK`
- `IK_ERR_[A-Z_]+`
- forbidden `tk` / `tkc` / `.tk`
- forbidden `LK` / `lkc` / `.lk`

Counts below are a scan snapshot before adding this report. They include the
existing ignored `Ai_repository/` historical reports because the user explicitly
asked to include that directory if present. They do not include generated
`dist/`, ignored benchmark output under `build/`, or dependency copies under
`node_modules/`.

The counts are non-exclusive. For example, `IK_Status` also contributes to
`IK_`.

## Legacy Hit Statistics

| Pattern | Count |
| --- | ---: |
| `IK` | 196 |
| `IntKernel` | 772 |
| `ikc` | 350 |
| `.ik` | 887 |
| `intkernel` | 229 |
| `IK_` | 743 |
| `IK_Status` | 238 |
| `IK_OK` | 118 |
| `IK_ERR_*` | 222 |
| forbidden `tk` / `tkc` / `.tk` | 12 |
| forbidden `LK` / `lkc` / `.lk` | 7 |

The forbidden `tk` / `LK` hits are expected allowlist candidates in repo policy,
naming tests, and historical/report text. They should not appear in current
user-facing project surfaces after Phase 21.

## Hits By Area

| Area | Files | Matches | Meaning |
| --- | ---: | ---: | --- |
| `README.md`, `README.zh-CN.md` | 2 | 192 | User-visible project identity, CLI examples, ABI docs. |
| `package.json` | 1 | 6 | Package name, bin, scripts, keywords, description. |
| `src/` | 14 | 168 | Internal symbols, CLI help, C ABI emitters, LLVM module id. |
| `tests/` | 47 | 625 | File names, CLI help, C harnesses, ABI expectations. |
| `tests/snapshots/` generated C/header | 13 | 349 | ABI macro/status snapshots. |
| other snapshots | 8 | 16 | LLVM source filenames and module id. |
| `tests/package-bin-smoke.test.ts` | 1 | 21 | Fresh install package/bin smoke. |
| `tests/naming-consistency.test.ts` | 1 | 4 | Naming rule definitions. |
| `tests/fixtures/` and `bench/perf/fixtures/` | 3 | filename hits | Source extension rename required. |
| `examples/` | 18 | 287 | Example docs/scripts and checked ABI constants. |
| `docs/` excluding release checklist | 22 | 810 | Shipped user documentation. |
| release checklist docs | 2 | 124 | Release gate still points at old names. |
| `bench/` | 15 | 144 | Benchmark runner paths, docs, C harnesses. |
| VS Code plugin current surface | 27 | 279 | Extension id, grammar scope, snippets, imports, docs/tests. |
| historical docs/plans under VS Code plugin | 5 | 448 | Historical allowlist candidates. |
| `Ai_repository/` | 11 | 282 | Historical/AI report allowlist candidates. |
| repo policy `AGENTS.md` | 1 | 19 | Must be updated or explicitly treated as policy during rename. |

## Required Hit Locations By Token

### `IK`

Primary current surfaces:

- `README.md`, `README.zh-CN.md`
- `package.json`
- `docs/ABI.md`, `docs/LANGUAGE_SPEC.md`, `docs/LLVM_BACKEND.md`,
  `docs/NAMING_CONVENTIONS.md`, `docs/OPTIMIZATION.md`,
  `docs/PERFORMANCE.md`, `docs/RELEASE_CHECKLIST.md`, `docs/ROADMAP.md`,
  `docs/WASM_ABI.md`
- matching `docs/zh-CN/*` files
- `bench/README.md`, `bench/README.zh-CN.md`,
  `bench/perf/baselines/example.summary.json`,
  `bench/perf/lib/cases.mjs`, `bench/perf/tests/perf-core.test.mjs`
- `examples/node-wasm-f64-array/README.md`,
  `examples/node-wasm-f64-array/README.zh-CN.md`
- `AGENTS.md`
- historical `Ai_repository/**`

Migration: user-facing `IK` becomes `CK`. Historical references can remain only
inside explicit migration/historical allowlists.

### `IntKernel`

Primary current surfaces:

- `README.md`, `README.zh-CN.md`
- `package.json`
- shipped docs under `docs/` and `docs/zh-CN/`
- `examples/browser-wasm-call/index.html`
- example READMEs under `examples/node-ffi-call`,
  `examples/node-wasm-f64-array`, and `examples/python-ctypes-call`
- VS Code extension files: `ik-vscode-plugin/package.json`, README,
  PUBLISHING, CHANGELOG, snippets, grammar, tests, and `src/**`
- internal public TypeScript type names, especially `IntKernelType`
- historical `Ai_repository/**`
- historical VS Code design/implementation docs under
  `ik-vscode-plugin/docs/superpowers/**`

Migration: current user-visible and public API identity becomes `CalcKernel`.
Internal TypeScript names such as `IntKernelType` should become
`CalcKernelType` unless a smaller internal-only name is deliberately chosen.

### `ikc`

Primary current surfaces:

- root `package.json` bin and script
- `src/cli.ts` help text
- `README.md`, `README.zh-CN.md`
- all release checklist command examples
- shipped docs under `docs/` and `docs/zh-CN/`
- `bench/perf/run.mjs`
- `bench/wasm_pricing_benchmark.mjs`
- example README and runtime error messages
- `tests/e2e.test.ts`
- `tests/package-bin-smoke.test.ts`
- `tests/naming-consistency.test.ts`
- historical `Ai_repository/**`

Migration: current command becomes `ckc`. Do not retain `ikc` as a bin, script,
or documented alias.

### `.ik`

Primary current surfaces:

- 23 tracked source/fixture files:
  - `bench/perf/fixtures/f64_kernels.ik`
  - `bench/perf/fixtures/pricing_helpers.ik`
  - `examples/dijkstra.ik`
  - `examples/explicit_casts.ik`
  - `examples/llvm_bool.ik`
  - `examples/llvm_calls.ik`
  - `examples/llvm_control_flow.ik`
  - `examples/llvm_memory.ik`
  - `examples/llvm_scalar.ik`
  - `examples/llvm_short_circuit.ik`
  - `examples/node-wasm-f64-array/f64_array.ik`
  - `examples/pricing.ik`
  - `examples/scalar.ik`
  - `examples/scalar_calls_checked.ik`
  - `examples/scalar_checked.ik`
  - `examples/scalar_control_checked.ik`
  - `examples/scalar_logical_checked.ik`
  - `examples/wasm_calls.ik`
  - `examples/wasm_control_flow.ik`
  - `examples/wasm_memory.ik`
  - `examples/wasm_scalar.ik`
  - `examples/wasm_short_circuit.ik`
  - `tests/fixtures/f64_edges.ik`
- `README.md`, `README.zh-CN.md`
- `docs/**`, `docs/zh-CN/**`, release checklist
- `bench/perf/run.mjs`
- benchmark docs and harness messages
- example READMEs and runtime messages
- VS Code extension package metadata, diagnostics, language service, tests,
  README/PUBLISHING/CHANGELOG
- nearly every e2e/emitter/backend test that uses source file names
- LLVM snapshots with `source_filename = "*.ik"`
- historical reports/plans

Parser/CLI finding: no hard parser or CLI extension gate was found. The CLI
reads the path supplied by the caller. The extension gate is in VS Code metadata
and `document.fileName.endsWith(".ik")`.

Migration: project-owned source and fixture files should be moved with
`git mv *.ik *.ck`. Do not document or test `.ik` compatibility in Phase 21.

### `intkernel`

Primary current surfaces:

- root `package.json` package name and keyword
- `examples/node-ffi-call/package.json`
- VS Code plugin dependency key and lockfile
- VS Code language id / grammar scope / diagnostic source / syntax file names
- LLVM module id in `src/backend/llvm/mir-llvm-emitter.ts`
- test temp directories and package tarball expectations
- LLVM snapshots with module/source metadata
- historical reports/plans

Migration: package identity and lower-case project identity become
`calckernel`. VS Code language id should likely become `calckernel`, and grammar
scope should become `source.calckernel`.

### `IK_`, `IK_Status`, `IK_OK`, `IK_ERR_*`

Primary current surfaces:

- `src/backend/c/c-header-emitter.ts`
- `src/backend/c/mir-c-emitter.ts`
- `src/backend/c/c-emitter.ts`
- `src/backend/c/c-build.ts`
- `README.md`, `README.zh-CN.md`
- `docs/ABI.md`, `docs/CHECKED_ARITHMETIC.md`,
  `docs/COMPILER_ARCHITECTURE.md`, `docs/LANGUAGE_SPEC.md`, `docs/MIR.md`,
  `docs/ROADMAP.md`, release checklist, and zh-CN equivalents
- `bench/perf/cases/f64-native.c`
- `bench/perf/cases/pricing-c-checked.c`
- `bench/pricing_checked_benchmark.c`
- `examples/node-ffi-call/checked.mjs`
- `examples/node-ffi-call/README*`
- `examples/python-ctypes-call/call_pricing_checked.py`
- `examples/python-ctypes-call/README*`
- `tests/c-emitter.test.ts`
- `tests/e2e.test.ts`
- `tests/mir-loop-optimization.test.ts`
- generated C/header snapshots under `tests/snapshots/`
- historical reports/plans

Migration: generated C ABI must move cleanly to `CK_` names with no
compatibility aliases.

## User-Visible Migration Range

Must migrate directly:

- Package name and binary command.
- README files.
- Shipped docs under `docs/`, including the release checklist.
- Example file names, examples docs, and example runtime error messages.
- Benchmark docs and benchmark runner command/file references.
- VS Code extension display name, language id, extension docs, snippets,
  syntax grammar, and diagnostics source.
- Generated package contents verified by `npm pack --dry-run`.

## Internal Symbol Migration Range

Must migrate directly:

- `src/cli.ts` help text.
- `src/index.ts` public type export.
- TypeScript type names such as `IntKernelType`.
- Checker/lowering/backend imports and type references using `IntKernelType`.
- LLVM module id string `intkernel`.
- VS Code service types and functions named `IntKernel*`.

These are internal/public API names, not language semantics. The rename should
not alter parser, type checker, optimizer, C/WASM/LLVM semantics, f64 behavior,
or explicit cast behavior.

## ABI Migration Range

Must migrate directly:

- `IK_API` -> `CK_API`
- `IK_BUILD_DLL` -> `CK_BUILD_DLL`
- `IK_Status` -> `CK_Status`
- `IK_OK` -> `CK_OK`
- `IK_ERR_OVERFLOW` -> `CK_ERR_OVERFLOW`
- `IK_ERR_DIV_BY_ZERO` -> `CK_ERR_DIV_BY_ZERO`
- `IK_ERR_NULL_POINTER` -> `CK_ERR_NULL_POINTER`

ABI implementation files:

- `src/backend/c/c-header-emitter.ts`
- `src/backend/c/mir-c-emitter.ts`
- `src/backend/c/c-emitter.ts`
- `src/backend/c/c-build.ts`

ABI consumers and tests:

- Python ctypes example and docs.
- Node FFI example and docs.
- C benchmark harnesses.
- C emitter tests and e2e C harnesses.
- Generated C/header snapshots.

Risk: this intentionally breaks existing host bindings. Do not add compatibility
aliases unless a later user request explicitly reopens that scope.

## Snapshots And Fixtures

Snapshot changes expected:

- Generated checked C `.c.snap`: `IK_Status`, `IK_OK`, `IK_ERR_*` become `CK_*`.
- Generated checked and unchecked header `.h.snap`: `IK_API`,
  `IK_BUILD_DLL`, and status definitions become `CK_*`.
- LLVM `.ll.snap`: source filename suffix changes to `.ck`; module id should
  become `calckernel`.
- WAT snapshots should only change if source-name comments or fixture names are
  embedded; WASM opcodes and arithmetic behavior must not change.

Fixture changes expected:

- `tests/fixtures/f64_edges.ik` -> `tests/fixtures/f64_edges.ck`
- `bench/perf/fixtures/f64_kernels.ik` -> `bench/perf/fixtures/f64_kernels.ck`
- `bench/perf/fixtures/pricing_helpers.ik` ->
  `bench/perf/fixtures/pricing_helpers.ck`

## Package Fresh Install Tests

`tests/package-bin-smoke.test.ts` must migrate:

- tarball name regex from `intkernel-*.tgz` to `calckernel-*.tgz`
- temp prefix from `intkernel-package-bin-` to a CalcKernel prefix
- installed bin from `node_modules/.bin/ikc` to `node_modules/.bin/ckc`
- smoke source from `smoke.ik` to `smoke.ck`
- help assertion from `ikc check <file>` to `ckc check <file>`
- expected success text from `OK: smoke.ik` to `OK: smoke.ck`

## Naming Consistency Tests

`tests/naming-consistency.test.ts` must be rewritten:

- allow current names: `CK`, `CalcKernel`, `ckc`, `.ck`, `calckernel`, `CK_`
- forbid legacy names in user-visible/current files:
  - `IK`
  - `IntKernel`
  - `ikc`
  - `.ik`
  - `intkernel`
  - `IK_`
  - `IK_Status`
  - `IK_OK`
  - `IK_ERR_*`
- continue forbidding:
  - `tk`
  - `tkc`
  - `.tk`
  - `LK`
  - `lkc`
  - `.lk`
- allow old names only in explicit historical/migration allowlists and inside
  the naming-test rule definitions themselves.

## Historical Allowlist Recommendation

Allowlist only with explicit historical labels:

- `Ai_repository/**`
- `ik-vscode-plugin/docs/superpowers/specs/**`
- `ik-vscode-plugin/docs/superpowers/plans/**`
- `bench/docs/2026-06-24-*.md`
- `bench/plans/2026-06-24-*.md`
- the Phase 21 migration guide
- release notes that explicitly say old names are legacy
- `tests/naming-consistency.test.ts` rule definitions and allowlist metadata
- possibly old git tag references if release docs later include them

Do not allow old names in ordinary current docs, README, examples, package
metadata, generated outputs, or CLI help.

## Package And Repository Recommendations

Package name:

- Recommend migrating `intkernel` -> `calckernel`.
- `npm view calckernel name version --json` should be rerun immediately before
  any publish. A prior scan returned 404, but that is not a reservation.
- Do not publish in this phase.

Repository name:

- Recommend migrating repository identity `IntKernel` -> `CalcKernel`.
- Do not rename local or remote repositories in Phase 21.1.
- Remote repo rename should be a separate operator-controlled step.

Version:

- Recommend `v0.7.0` for the rename release because this is a breaking
  user-visible package/CLI/source-extension/C-ABI rename.
- Recommended precondition: finish and tag Phase 20 as `v0.6.0` first, because
  Phase 20 added explicit casts and is a separate completed language milestone.

## Compatibility Recommendations

Should `ikc` alias be retained?

- No.

Should `.ik` compatibility be retained?

- No.

Should `IK_` ABI aliases be retained?

- No.

Rationale: the Phase 21 contract is a clean destructive rename. Aliases would
increase test surface and contradict the requested default behavior.

## Maximum Risks

1. Package/bin breakage: existing `node_modules/.bin/ikc` consumers break.
2. Source extension breakage: all `.ik` scripts, editor config, examples, and
   tests must move to `.ck`.
3. C ABI breakage: C/C++/Python/Node FFI users must migrate status and export
   macro names.
4. VS Code plugin split-brain risk: language id, grammar scope, snippets,
   dependency import, docs, and tests must be renamed together.
5. Snapshot churn risk: many snapshots change mechanically; semantic changes
   must be rejected.
6. Historical allowlist risk: if allowlist is too broad, old names can remain in
   current user-facing docs unnoticed.
7. Tooling risk: `pnpm` in this environment is v11.7.0 even though the package
   declares `pnpm@9.15.9`; dependency-state checks can create temporary
   `pnpm-workspace.yaml` if builds are approved.

## Recommended Implementation Stages After This Report

Phase 21.2: package/bin/source extension rename.

- Move tracked `.ik` files to `.ck` with `git mv`.
- Update root package name/bin/script and CLI help.
- Update package fresh install smoke and file-path-only tests.

Phase 21.3: C ABI rename.

- Update C emitters and `-DCK_BUILD_DLL`.
- Update examples, C harnesses, Python ctypes, Node FFI, and ABI tests.
- Regenerate and inspect C/header snapshots.

Phase 21.4: docs/examples/bench rename.

- Update README, shipped docs, release checklist, examples, benchmark runner,
  benchmark docs, and fixture paths.
- Keep historical docs on explicit allowlist.

Phase 21.5: VS Code plugin rename.

- Update extension package identity, language id, grammar scope, snippets,
  imports from package, tests, and lockfile.

Phase 21.6: migration guide and naming gate.

- Add migration guide with old -> new mapping.
- Tighten naming consistency test with explicit allowlists.
- Run final forbidden-name scan.

Phase 21.7: release readiness only.

- Verify package dry-run reports `calckernel@0.7.0` if versioning is approved.
- Do not tag or publish unless explicitly requested.

## Verification Commands

Required commands run for this report:

```sh
pnpm typecheck
pnpm build
pnpm test
npm pack --dry-run
```

Results:

- `pnpm typecheck`: passed.
- `pnpm build`: passed.
- `pnpm test`: failed in the existing naming consistency gate only.
  57 test files and 402 tests passed; 1 test file and 1 test failed:
  `tests/naming-consistency.test.ts` flags the Phase 21 report content because it
  intentionally names forbidden legacy tokens and the current gate has no
  historical/migration allowlist for this report path. It also scans the existing
  ignored `Ai_repository/plans/PHASE_21_RENAME_IMPACT_REPORT.md` file.
- `npm pack --dry-run`: passed, current package still reports
  `intkernel@0.5.0` with 263 files, including this report.

Benchmark was not run. Existing ignored local files under `build/perf/` were not
submitted or modified for this report.

## Can Phase 21.2 Start?

Recommendation: do not enter Phase 21.2 automatically.

Phase 21.2 can start after explicit approval, but it should include naming
consistency allowlist work early because the current gate fails on the required
legacy-name migration/report text. The cleaner release path is:

1. Finish/freeze/tag Phase 20 as `v0.6.0`.
2. Start the destructive CK / CalcKernel rename.
3. Use `v0.7.0` for the rename release.
