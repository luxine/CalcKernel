# Local Performance Suite Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a local-only hyperfine-based performance suite for IntKernel pricing benchmarks.

**Architecture:** Keep all new files under `bench/`. Use small tested library modules for argument parsing, statistics, summary generation, and command construction. Use `bench/perf/run.mjs` as the only executable entrypoint and keep generated outputs under ignored `build/perf/`.

**Tech Stack:** Node.js ESM, Node built-in `node:test`, `hyperfine`, `clang`, existing `pnpm ikc` CLI output.

---

### Task 1: Test the Runner Core

**Files:**
- Create: `bench/perf/lib/args.mjs`
- Create: `bench/perf/lib/stats.mjs`
- Create: `bench/perf/lib/summary.mjs`
- Create: `bench/perf/lib/hyperfine.mjs`
- Create: `bench/perf/tests/perf-core.test.mjs`

- [ ] **Step 1: Write failing tests**

Create `bench/perf/tests/perf-core.test.mjs` with tests for:

- `parseArgs(["--quick", "--items", "1000", "--iterations", "50"])`
- `median([3, 1, 2])`
- `percentile([1, 2, 100], 0.95)`
- `compareSummaries(current, baseline)`
- `buildHyperfineArgs(config, commands, outputPaths)`

- [ ] **Step 2: Verify tests fail**

Run:

```sh
node --test bench/perf/tests/perf-core.test.mjs
```

Expected result: fail because the library modules do not exist yet.

- [ ] **Step 3: Implement minimal library modules**

Implement:

- `parseArgs(argv)` with defaults: full mode, `items=100000`, `iterations=1000`, `runs=20`, `warmup=3`.
- `median(values)`, `percentile(values, p)`, and `relativeToFastest(results)`.
- `summarizeHyperfine(data, metadata)` that accepts hyperfine JSON and returns a normalized summary.
- `compareSummaries(current, baseline)` with 5% warning and 10% regression thresholds.
- `buildHyperfineArgs(config, commands, outputPaths)` that returns the exact `hyperfine` argument array.

- [ ] **Step 4: Verify tests pass**

Run:

```sh
node --test bench/perf/tests/perf-core.test.mjs
```

Expected result: all tests pass.

### Task 2: Add Benchmark Cases

**Files:**
- Create: `bench/perf/cases/pricing-js-bigint.mjs`
- Create: `bench/perf/cases/pricing-wasm.mjs`
- Create: `bench/perf/cases/pricing-c-unchecked.c`
- Create: `bench/perf/cases/pricing-c-checked.c`

- [ ] **Step 1: Write executable cases**

Each case accepts:

```text
--items <n>
--iterations <n>
```

The WASM case also accepts:

```text
--wasm <path>
```

Each case must:

- Generate the same pricing input.
- Run `calc_items` for the requested iteration count.
- Validate the final checksum.
- Print one concise line with case name, items, iterations, and checksum.
- Exit non-zero on incorrect status or checksum.

- [ ] **Step 2: Verify JS case directly**

Run:

```sh
node bench/perf/cases/pricing-js-bigint.mjs --items 1000 --iterations 2
```

Expected result: exits 0 and prints checksum.

### Task 3: Add the Hyperfine Runner

**Files:**
- Create: `bench/perf/run.mjs`
- Modify: `bench/perf/lib/summary.mjs`

- [ ] **Step 1: Implement tool checks and build steps**

`bench/perf/run.mjs` must:

- Check `hyperfine`, `pnpm`, and `clang`.
- Run `pnpm build`.
- Generate unchecked C, checked C, and WASM into `build/perf/generated/`.
- Compile C case binaries into `build/perf/bin/`.

- [ ] **Step 2: Implement smoke execution**

Run each command once before hyperfine. If a case exits non-zero, stop with the case name and output.

- [ ] **Step 3: Implement hyperfine execution**

Run hyperfine with named commands and export JSON/Markdown to `build/perf/`.

- [ ] **Step 4: Implement summary and baseline operations**

Read `latest.hyperfine.json`, write `latest.summary.json` and `latest.summary.md`, and support:

- `--save-baseline`
- `--compare`
- `--fail-on-regression`

### Task 4: Document Local Usage

**Files:**
- Modify: `bench/README.md`
- Modify: `bench/README.zh-CN.md`

- [ ] **Step 1: Add local performance suite docs**

Document:

- Installing hyperfine.
- Running quick and full local performance suites.
- Saving and comparing a local baseline.
- Output files under `build/perf/`.
- Why results are local-only and not CI-stable.

### Task 5: Final Verification

**Files:** No new files.

- [ ] **Step 1: Run unit tests**

```sh
node --test bench/perf/tests/perf-core.test.mjs
```

- [ ] **Step 2: Run project build**

```sh
pnpm build
```

- [ ] **Step 3: Run project tests**

```sh
pnpm test
```

- [ ] **Step 4: Run local perf smoke**

```sh
node bench/perf/run.mjs --quick
```

If `hyperfine` is not installed, verify the runner exits with install guidance and do not claim full perf execution passed.
