# Local Performance Suite Design

## Goal

Build a local-only performance suite for IntKernel pricing kernels using a professional benchmark tool. The suite must stay under `bench/`, avoid CI assumptions, and produce repeatable local reports that can be compared against a private local baseline.

## Constraints

- All documentation, plans, scripts, benchmark cases, and tests for this work live under `bench/`.
- The root `package.json` is not modified; users run the suite directly with `node bench/perf/run.mjs`.
- The suite is local-only. It does not add GitHub Actions or shared baseline files.
- Generated outputs live under `build/perf/`, which is already ignored by the repository.
- `hyperfine` is an explicit local prerequisite. The runner checks for it and prints install guidance instead of silently falling back to ad hoc timing.
- The runner builds the current TypeScript source before benchmarking so results match the current checkout.

## Tool Choice

Use `hyperfine` for process-level benchmarking. It provides warmups, multiple runs, JSON export, Markdown export, and comparable command names. Each benchmark command performs enough internal work to make process startup and timer noise less dominant.

The suite still validates correctness itself through checksums. Hyperfine measures commands; it does not know whether the generated kernel returned correct results.

## First Benchmark Cases

- `pricing-c-unchecked`: generated C backend, unchecked arithmetic.
- `pricing-c-checked`: generated C backend, checked arithmetic.
- `pricing-wasm-unchecked`: generated WASM backend, unchecked arithmetic.
- `pricing-js-bigint`: JavaScript BigInt baseline matching IntKernel `i64` semantics.

These cases use the same input generation pattern and the same expected checksum for each item count. The default item count is `100000`.

## Runner Behavior

`node bench/perf/run.mjs` performs these steps:

1. Resolve the repository root from the script location.
2. Parse flags:
   - `--quick`: fewer hyperfine runs and fewer internal iterations.
   - `--full`: full local benchmark mode. This is the default.
   - `--save-baseline`: save the latest summary to `build/perf/baseline.local.json`.
   - `--compare`: compare the latest summary against `build/perf/baseline.local.json`.
   - `--fail-on-regression`: exit non-zero when comparison finds a regression.
   - `--items <n>`: override item count.
   - `--iterations <n>`: override internal loop count per command.
3. Check required tools: `node`, `pnpm`, `clang`, and `hyperfine`.
4. Run `pnpm build`.
5. Emit C/header and WASM artifacts into `build/perf/generated/`.
6. Compile C benchmark executables into `build/perf/bin/`.
7. Run each command once as a correctness smoke check.
8. Run hyperfine and export:
   - `build/perf/latest.hyperfine.json`
   - `build/perf/latest.hyperfine.md`
9. Generate:
   - `build/perf/latest.summary.json`
   - `build/perf/latest.summary.md`
10. Optionally save or compare a local baseline.

## Comparison Rules

Comparison uses each case's median runtime from the hyperfine result:

- Slower by less than or equal to 5%: `ok`
- Slower by more than 5% and less than or equal to 10%: `warning`
- Slower by more than 10%: `regression`
- Faster or equal: `ok`

By default, regressions are reported but do not fail the process. `--fail-on-regression` makes regressions exit non-zero for local scripting.

## Reporting

The Markdown summary includes:

- Run timestamp.
- Git SHA and dirty flag.
- Node, clang, hyperfine, platform, arch, and CPU model.
- Mode, item count, iterations, warmup count, and hyperfine run count.
- Case table with median, p95, min, mean, standard deviation, and relative speed.
- Optional baseline comparison table.

## Testing Strategy

Use Node's built-in `node:test` under `bench/perf/tests/` so no root configuration changes are needed. Unit tests cover:

- CLI flag parsing.
- Median and p95 calculations.
- Hyperfine JSON summarization.
- Baseline comparison thresholds.
- Hyperfine command argument construction.

Integration validation is manual/local:

```sh
node --test bench/perf/tests/*.test.mjs
node bench/perf/run.mjs --quick
```

The full local suite is:

```sh
node bench/perf/run.mjs --full
```

## Non-Goals

- No CI workflow.
- No shared checked-in baseline.
- No root package script changes.
- No automatic installation of `hyperfine`.
- No cross-machine performance comparison.
- No strict fail-by-default performance gate.
