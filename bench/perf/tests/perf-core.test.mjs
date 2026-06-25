import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { describe, it } from "node:test";
import { parseArgs } from "../lib/args.mjs";
import { benchmarkCommands, nativeCompileJobs } from "../lib/cases.mjs";
import { withinTolerance } from "../lib/f64-correctness.mjs";
import {
  byteOffsetToF64Index,
  checksumOutputFloat64Array,
  createLowCopyF64Inputs,
  requiredBytesFor,
  writeInputsFloat64Array
} from "../lib/f64-wasm-memory.mjs";
import { buildHyperfineArgs } from "../lib/hyperfine.mjs";
import { compareSummaries, filterBenchmarkCommands, formatSummaryMarkdown, hasRegression, parseBaselineSummary, summarizeHyperfine } from "../lib/summary.mjs";
import { median, percentile, relativeToFastest } from "../lib/stats.mjs";

describe("perf runner core", () => {
  it("parses quick mode and numeric overrides", () => {
    const config = parseArgs(["--quick", "--items", "1000", "--iterations", "50"]);

    assert.equal(config.mode, "quick");
    assert.equal(config.items, 1000);
    assert.equal(config.iterations, 50);
    assert.equal(config.runs, 5);
    assert.equal(config.warmup, 1);
    assert.equal(config.saveBaseline, false);
    assert.equal(config.compare, false);
    assert.equal(config.failOnRegression, false);
  });

  it("parses full mode flags", () => {
    const config = parseArgs([
      "--full",
      "--save-baseline",
      "--compare",
      "--fail-on-regression",
      "--threshold",
      "12",
      "--case",
      "pricing-c-unchecked",
      "--case",
      "pricing-wasm-unchecked"
    ]);

    assert.equal(config.mode, "full");
    assert.equal(config.items, 100000);
    assert.equal(config.iterations, 1000);
    assert.equal(config.runs, 20);
    assert.equal(config.warmup, 3);
    assert.equal(config.saveBaseline, true);
    assert.equal(config.compare, true);
    assert.equal(config.failOnRegression, true);
    assert.equal(config.thresholdPercent, 12);
    assert.deepEqual(config.cases, ["pricing-c-unchecked", "pricing-wasm-unchecked"]);
  });

  it("rejects invalid numeric flags", () => {
    assert.throws(() => parseArgs(["--items", "0"]), /--items must be a positive integer/);
    assert.throws(() => parseArgs(["--iterations", "nan"]), /--iterations must be a positive integer/);
    assert.throws(() => parseArgs(["--threshold", "0"]), /--threshold must be a positive number/);
    assert.throws(() => parseArgs(["--threshold", "fast"]), /--threshold must be a positive number/);
    assert.throws(() => parseArgs(["--case"]), /--case requires a non-empty value/);
  });

  it("calculates median and percentile from unsorted samples", () => {
    assert.equal(median([3, 1, 2]), 2);
    assert.equal(median([4, 1, 2, 3]), 2.5);
    assert.equal(percentile([1, 2, 100], 0.95), 100);
  });

  it("adds relative speed against fastest result", () => {
    const results = relativeToFastest([
      { name: "slow", medianSeconds: 0.2 },
      { name: "fast", medianSeconds: 0.1 }
    ]);

    assert.equal(results[0].relativeToFastest, 2);
    assert.equal(results[1].relativeToFastest, 1);
  });

  it("summarizes hyperfine JSON", () => {
    const commands = benchmarkCommands(
      { mode: "quick", items: 100000, iterations: 100 },
      {
        binDir: "build/perf/bin",
        generatedDir: "build/perf/generated"
      }
    );
    const summary = summarizeHyperfine(
      {
        results: [
          {
            command: "pricing-c-unchecked-O3",
            mean: 0.12,
            stddev: 0.01,
            min: 0.1,
            max: 0.15,
            median: 0.11,
            times: [0.1, 0.11, 0.15]
          }
        ]
      },
      {
        mode: "full",
        items: 100000,
        iterations: 1000,
        runs: 20,
        warmup: 3,
        gitSha: "abc123",
        dirty: false,
        generatedAt: "2026-06-24T00:00:00.000Z"
      },
      commands
    );

    assert.equal(summary.metadata.mode, "full");
    assert.equal(summary.results[0].name, "pricing-c-unchecked-O3");
    assert.equal(summary.results[0].category, "native");
    assert.equal(summary.results[0].phase, "total");
    assert.equal(summary.results[0].optLevel, "O3");
    assert.equal(summary.results[0].overflowMode, "unchecked");
    assert.equal(summary.results[0].medianSeconds, 0.11);
    assert.equal(summary.results[0].p95Seconds, 0.15);
    assert.equal(summary.results[0].relativeToFastest, 1);
    assert.equal(summary.results[0].ratioToCUncheckedO3, 1);
  });

  it("defines decomposed benchmark cases with metadata", () => {
    const commands = benchmarkCommands(
      { mode: "full", items: 100000, iterations: 1000 },
      {
        binDir: "build/perf/bin",
        generatedDir: "build/perf/generated"
      }
    );

    assert.deepEqual(commands.map((command) => command.name).slice(0, 19), [
      "pricing-c-unchecked-O0",
      "pricing-c-unchecked-O2",
      "pricing-c-unchecked-O3",
      "pricing-c-unchecked-ik-O3",
      "pricing-c-checked-O3",
      "pricing-helpers-c-unchecked-ik-O0",
      "pricing-helpers-c-unchecked-ik-O2",
      "pricing-llvm-unchecked-O0",
      "pricing-llvm-unchecked-O2",
      "pricing-llvm-unchecked-O3",
      "pricing-wasm-unchecked-total",
      "pricing-wasm-unchecked-total-O3",
      "pricing-wasm-unchecked-compute-only",
      "pricing-wasm-unchecked-compute-only-O3",
      "pricing-wasm-unchecked-memory-only",
      "pricing-wasm-unchecked-call-overhead",
      "pricing-js-number",
      "pricing-js-typedarray-number",
      "pricing-js-bigint"
    ]);

    const wasmCompute = commands.find((command) => command.name === "pricing-wasm-unchecked-compute-only");
    assert.equal(wasmCompute.category, "wasm");
    assert.equal(wasmCompute.optLevel, "IK-O0");
    assert.equal(wasmCompute.overflowMode, "unchecked");
    assert.match(wasmCompute.command, /--mode compute-only/);

    const wasmComputeO3 = commands.find((command) => command.name === "pricing-wasm-unchecked-compute-only-O3");
    assert.equal(wasmComputeO3.category, "wasm");
    assert.equal(wasmComputeO3.optLevel, "IK-O3");
    assert.equal(wasmComputeO3.overflowMode, "unchecked");
    assert.match(wasmComputeO3.command, /pricing_o3\.wasm/);

    const memoryOnly = commands.find((command) => command.name === "pricing-wasm-unchecked-memory-only");
    assert.equal(memoryOnly.category, "memory");
    assert.match(memoryOnly.command, /--mode memory-only/);

    const callOverhead = commands.find((command) => command.name === "pricing-wasm-unchecked-call-overhead");
    assert.equal(callOverhead.category, "call-overhead");
    assert.match(callOverhead.command, /--calls 10000/);

    const helperO2 = commands.find((command) => command.name === "pricing-helpers-c-unchecked-ik-O2");
    assert.equal(helperO2.category, "native");
    assert.equal(helperO2.optLevel, "IK-O2/clang-O3");
    assert.equal(helperO2.overflowMode, "unchecked");

    const pricingIkO3 = commands.find((command) => command.name === "pricing-c-unchecked-ik-O3");
    assert.equal(pricingIkO3.category, "native");
    assert.equal(pricingIkO3.optLevel, "IK-O3/clang-O3");
    assert.equal(pricingIkO3.overflowMode, "unchecked");

    const llvmO2 = commands.find((command) => command.name === "pricing-llvm-unchecked-O2");
    assert.equal(llvmO2.category, "native");
    assert.equal(llvmO2.optLevel, "O2");
    assert.equal(llvmO2.overflowMode, "unchecked");

    const checkedIkO3 = commands.find((command) => command.name === "pricing-c-checked-O3");
    assert.equal(checkedIkO3.category, "native");
    assert.equal(checkedIkO3.optLevel, "IK-O3/clang-O3");
    assert.equal(checkedIkO3.overflowMode, "checked");
  });

  it("defines f64 benchmark cases for each kernel and target", () => {
    const commands = benchmarkCommands(
      { mode: "quick", items: 1000, iterations: 10 },
      {
        binDir: "build/perf/bin",
        generatedDir: "build/perf/generated"
      }
    );
    const f64Names = commands.map((command) => command.name).filter((name) => name.startsWith("f64-"));

    assert.deepEqual(f64Names, [
      "f64-axpy-js-array-number",
      "f64-axpy-js-float64array",
      "f64-axpy-ik-c-o3",
      "f64-axpy-ik-llvm-o3",
      "f64-axpy-ik-wasm-o3-setup",
      "f64-axpy-ik-wasm-o3-input-marshal",
      "f64-axpy-ik-wasm-o3-compute-only",
      "f64-axpy-ik-wasm-o3-output-readback",
      "f64-axpy-ik-wasm-o3-total",
      "f64-axpy-wasm-memory-only",
      "f64-axpy-ik-wasm-o3-low-copy-setup",
      "f64-axpy-ik-wasm-o3-low-copy-input-marshal",
      "f64-axpy-ik-wasm-o3-low-copy-compute-only",
      "f64-axpy-ik-wasm-o3-low-copy-output-readback",
      "f64-axpy-ik-wasm-o3-low-copy-total",
      "f64-dot-js-array-number",
      "f64-dot-js-float64array",
      "f64-dot-ik-c-o3",
      "f64-dot-ik-llvm-o3",
      "f64-dot-ik-wasm-o3-setup",
      "f64-dot-ik-wasm-o3-input-marshal",
      "f64-dot-ik-wasm-o3-compute-only",
      "f64-dot-ik-wasm-o3-output-readback",
      "f64-dot-ik-wasm-o3-total",
      "f64-dot-wasm-memory-only",
      "f64-dot-ik-wasm-o3-low-copy-setup",
      "f64-dot-ik-wasm-o3-low-copy-input-marshal",
      "f64-dot-ik-wasm-o3-low-copy-compute-only",
      "f64-dot-ik-wasm-o3-low-copy-output-readback",
      "f64-dot-ik-wasm-o3-low-copy-total",
      "f64-sum-js-array-number",
      "f64-sum-js-float64array",
      "f64-sum-ik-c-o3",
      "f64-sum-ik-llvm-o3",
      "f64-sum-ik-wasm-o3-setup",
      "f64-sum-ik-wasm-o3-input-marshal",
      "f64-sum-ik-wasm-o3-compute-only",
      "f64-sum-ik-wasm-o3-output-readback",
      "f64-sum-ik-wasm-o3-total",
      "f64-sum-wasm-memory-only",
      "f64-sum-ik-wasm-o3-low-copy-setup",
      "f64-sum-ik-wasm-o3-low-copy-input-marshal",
      "f64-sum-ik-wasm-o3-low-copy-compute-only",
      "f64-sum-ik-wasm-o3-low-copy-output-readback",
      "f64-sum-ik-wasm-o3-low-copy-total",
      "f64-scale-js-array-number",
      "f64-scale-js-float64array",
      "f64-scale-ik-c-o3",
      "f64-scale-ik-llvm-o3",
      "f64-scale-ik-wasm-o3-setup",
      "f64-scale-ik-wasm-o3-input-marshal",
      "f64-scale-ik-wasm-o3-compute-only",
      "f64-scale-ik-wasm-o3-output-readback",
      "f64-scale-ik-wasm-o3-total",
      "f64-scale-wasm-memory-only",
      "f64-scale-ik-wasm-o3-low-copy-setup",
      "f64-scale-ik-wasm-o3-low-copy-input-marshal",
      "f64-scale-ik-wasm-o3-low-copy-compute-only",
      "f64-scale-ik-wasm-o3-low-copy-output-readback",
      "f64-scale-ik-wasm-o3-low-copy-total"
    ]);

    const cCase = commands.find((command) => command.name === "f64-axpy-ik-c-o3");
    assert.equal(cCase.category, "native");
    assert.equal(cCase.optLevel, "IK-O3/clang-O3");
    assert.equal(cCase.overflowMode, "unchecked");
    assert.match(cCase.command, /--kernel axpy/);

    const llvmCase = commands.find((command) => command.name === "f64-dot-ik-llvm-o3");
    assert.equal(llvmCase.category, "native");
    assert.equal(llvmCase.optLevel, "IK-O3\/LLVM-O3");

    const wasmCompute = commands.find((command) => command.name === "f64-sum-ik-wasm-o3-compute-only");
    assert.equal(wasmCompute.category, "wasm");
    assert.equal(wasmCompute.phase, "compute");
    assert.equal(wasmCompute.optLevel, "IK-O3");
    assert.match(wasmCompute.command, /--mode compute-only/);

    const wasmSetup = commands.find((command) => command.name === "f64-axpy-ik-wasm-o3-setup");
    assert.equal(wasmSetup.category, "wasm");
    assert.equal(wasmSetup.phase, "setup");
    assert.match(wasmSetup.command, /--mode setup/);

    const wasmInputMarshal = commands.find((command) => command.name === "f64-dot-ik-wasm-o3-input-marshal");
    assert.equal(wasmInputMarshal.category, "memory");
    assert.equal(wasmInputMarshal.phase, "input-marshal");
    assert.match(wasmInputMarshal.command, /--mode input-marshal/);

    const wasmOutputReadback = commands.find((command) => command.name === "f64-scale-ik-wasm-o3-output-readback");
    assert.equal(wasmOutputReadback.category, "memory");
    assert.equal(wasmOutputReadback.phase, "output-readback");
    assert.match(wasmOutputReadback.command, /--mode output-readback/);

    const wasmMemory = commands.find((command) => command.name === "f64-scale-wasm-memory-only");
    assert.equal(wasmMemory.category, "memory");
    assert.equal(wasmMemory.phase, "memory-only");
    assert.match(wasmMemory.command, /--mode memory-only/);

    const wasmLowCopyTotal = commands.find((command) => command.name === "f64-axpy-ik-wasm-o3-low-copy-total");
    assert.equal(wasmLowCopyTotal.category, "wasm-low-copy");
    assert.equal(wasmLowCopyTotal.phase, "total");
    assert.match(wasmLowCopyTotal.command, /--mode total/);
    assert.match(wasmLowCopyTotal.command, /--copy-mode float64array/);

    const wasmLowCopyInput = commands.find((command) => command.name === "f64-dot-ik-wasm-o3-low-copy-input-marshal");
    assert.equal(wasmLowCopyInput.category, "memory-low-copy");
    assert.equal(wasmLowCopyInput.phase, "input-marshal");
    assert.match(wasmLowCopyInput.command, /--copy-mode float64array/);
  });

  it("defines native f64 compile jobs without changing pricing jobs", () => {
    const jobs = nativeCompileJobs({
      binDir: "build/perf/bin",
      generatedDir: "build/perf/generated"
    });

    assert.deepEqual(
      jobs.map((job) => job.name).slice(0, 10),
      [
        "pricing-c-unchecked-O0",
        "pricing-c-unchecked-O2",
        "pricing-c-unchecked-O3",
        "pricing-c-unchecked-ik-O3",
        "pricing-c-checked-O3",
        "pricing-helpers-c-unchecked-ik-O0",
        "pricing-helpers-c-unchecked-ik-O2",
        "pricing-llvm-unchecked-O0",
        "pricing-llvm-unchecked-O2",
        "pricing-llvm-unchecked-O3"
      ]
    );
    assert.deepEqual(jobs.map((job) => job.name).slice(10), ["f64-ik-c-o3", "f64-ik-llvm-o3"]);
  });

  it("compares f64 correctness with absolute and relative tolerance", () => {
    assert.equal(withinTolerance(1.0, 1.0, { absTol: 1e-9, relTol: 1e-9 }), true);
    assert.equal(withinTolerance(1.0 + 5e-10, 1.0, { absTol: 1e-9, relTol: 1e-12 }), true);
    assert.equal(withinTolerance(1000000.001, 1000000.0, { absTol: 1e-12, relTol: 1e-8 }), true);
    assert.equal(withinTolerance(1.1, 1.0, { absTol: 1e-9, relTol: 1e-9 }), false);
    assert.equal(withinTolerance(Number.NaN, 1.0, { absTol: 1e-9, relTol: 1e-9 }), false);
  });

  it("smoke-runs f64 JavaScript benchmark kernels", () => {
    for (const kernel of ["axpy", "dot", "sum", "scale"]) {
      const result = spawnSync(
        "node",
        ["bench/perf/cases/f64-js-array-number.mjs", "--kernel", kernel, "--items", "16", "--iterations", "2"],
        { encoding: "utf8" }
      );

      assert.equal(result.status, 0, result.stderr || result.stdout);
      assert.match(result.stdout, new RegExp(`f64-${kernel}-js-array-number`));
    }
  });

  it("smoke-runs the f64 WASM memory-only benchmark path without a module", () => {
    const result = spawnSync(
      "node",
      [
        "bench/perf/cases/f64-wasm.mjs",
        "--kernel",
        "axpy",
        "--mode",
        "memory-only",
        "--label",
        "f64-axpy-wasm-memory-only",
        "--items",
        "16",
        "--iterations",
        "2"
      ],
      { encoding: "utf8" }
    );

    assert.equal(result.status, 0, result.stderr || result.stdout);
    assert.match(result.stdout, /f64-axpy-wasm-memory-only/);
  });

  it("compares summaries with configurable median regression thresholds", () => {
    const baseline = {
      results: [
        { name: "ok-case", medianSeconds: 1 },
        { name: "warning-case", medianSeconds: 1 },
        { name: "regression-case", medianSeconds: 1 }
      ]
    };
    const current = {
      results: [
        { name: "ok-case", medianSeconds: 1.04 },
        { name: "warning-case", medianSeconds: 1.08 },
        { name: "regression-case", medianSeconds: 1.2 }
      ]
    };

    const comparison = compareSummaries(current, baseline, { thresholdPercent: 10 });

    assert.deepEqual(
      comparison.map((entry) => [entry.name, entry.status, Number(entry.deltaPercent.toFixed(2)), Number(entry.ratio.toFixed(2))]),
      [
        ["ok-case", "ok", 4, 1.04],
        ["warning-case", "warning", 8, 1.08],
        ["regression-case", "regression", 20, 1.2]
      ]
    );
  });

  it("parses baseline summary shape", () => {
    const baseline = parseBaselineSummary(
      JSON.stringify({
        metadata: { mode: "full" },
        results: [{ name: "pricing-c-unchecked-O3", medianSeconds: 0.1 }]
      }),
      "example.summary.json"
    );

    assert.equal(baseline.results[0].name, "pricing-c-unchecked-O3");
    assert.throws(() => parseBaselineSummary("{}", "bad.summary.json"), /Invalid baseline summary/);
  });

  it("filters benchmark commands by repeated case prefixes", () => {
    const commands = benchmarkCommands(
      { mode: "full", items: 100000, iterations: 1000 },
      {
        binDir: "build/perf/bin",
        generatedDir: "build/perf/generated"
      }
    );

    const filtered = filterBenchmarkCommands(commands, ["pricing-c-unchecked", "pricing-wasm-unchecked-compute-only"]);

    assert.deepEqual(
      filtered.map((command) => command.name),
      [
        "pricing-c-unchecked-O0",
        "pricing-c-unchecked-O2",
        "pricing-c-unchecked-O3",
        "pricing-c-unchecked-ik-O3",
        "pricing-wasm-unchecked-compute-only",
        "pricing-wasm-unchecked-compute-only-O3"
      ]
    );
    assert.throws(() => filterBenchmarkCommands(commands, ["does-not-exist"]), /No benchmark cases matched/);
  });

  it("filters f64 benchmark commands by kernel prefix", () => {
    const commands = benchmarkCommands(
      { mode: "full", items: 100000, iterations: 1000 },
      {
        binDir: "build/perf/bin",
        generatedDir: "build/perf/generated"
      }
    );

    assert.deepEqual(
      filterBenchmarkCommands(commands, ["f64-axpy"]).map((command) => command.name),
      [
        "f64-axpy-js-array-number",
        "f64-axpy-js-float64array",
        "f64-axpy-ik-c-o3",
        "f64-axpy-ik-llvm-o3",
        "f64-axpy-ik-wasm-o3-setup",
        "f64-axpy-ik-wasm-o3-input-marshal",
        "f64-axpy-ik-wasm-o3-compute-only",
        "f64-axpy-ik-wasm-o3-output-readback",
        "f64-axpy-ik-wasm-o3-total",
        "f64-axpy-wasm-memory-only",
        "f64-axpy-ik-wasm-o3-low-copy-setup",
        "f64-axpy-ik-wasm-o3-low-copy-input-marshal",
        "f64-axpy-ik-wasm-o3-low-copy-compute-only",
        "f64-axpy-ik-wasm-o3-low-copy-output-readback",
        "f64-axpy-ik-wasm-o3-low-copy-total"
      ]
    );
  });

  it("writes and reads low-copy f64 WASM memory through Float64Array views", () => {
    const layout = requiredBytesFor(4);
    const values = new Float64Array(layout.totalBytes / 8);
    const inputs = createLowCopyF64Inputs(4, "axpy");

    assert.equal(byteOffsetToF64Index(layout.yOffset), 4);
    assert.throws(() => byteOffsetToF64Index(6), /8-byte aligned/);

    const checksum = writeInputsFloat64Array(values, layout, inputs, "axpy");
    assert.equal(values[byteOffsetToF64Index(layout.xOffset)], 1.0);
    assert.equal(values[byteOffsetToF64Index(layout.yOffset)], -2.0);
    assert.equal(checksum, inputs.inputChecksum);

    values[byteOffsetToF64Index(layout.yOffset)] = 10.0;
    values[byteOffsetToF64Index(layout.yOffset) + 1] = 11.0;
    values[byteOffsetToF64Index(layout.yOffset) + 2] = 12.0;
    values[byteOffsetToF64Index(layout.yOffset) + 3] = 13.0;
    assert.equal(checksumOutputFloat64Array(values, layout, 4, "axpy"), 46.0);
    assert.equal(checksumOutputFloat64Array(values, layout, 4, "dot", 17.5), 17.5);
  });

  it("formats comparison markdown with ratio and slower percentage", () => {
    const summary = {
      metadata: {
        generatedAt: "2026-06-24T00:00:00.000Z",
        mode: "full",
        items: 100000,
        iterations: 1000,
        runs: 20,
        warmup: 3,
        gitSha: "abc123",
        dirty: false,
        nodeVersion: "v24.0.0",
        clangVersion: "clang 17",
        hyperfineVersion: "hyperfine 1.0",
        platform: "darwin",
        arch: "arm64",
        cpuModel: "Apple"
      },
      results: [
        {
          name: "pricing-c-unchecked-O3",
          category: "native",
          phase: "total",
          optLevel: "O3",
          overflowMode: "unchecked",
          medianSeconds: 0.11,
          p95Seconds: 0.12,
          minSeconds: 0.1,
          meanSeconds: 0.11,
          stddevSeconds: 0.01,
          relativeToFastest: 1,
          ratioToCUncheckedO3: 1
        }
      ]
    };
    const markdown = formatSummaryMarkdown(summary, [
      {
        name: "pricing-c-unchecked-O3",
        status: "regression",
        currentMedianSeconds: 0.11,
        baselineMedianSeconds: 0.1,
        ratio: 1.1,
        deltaRatio: 0.1,
        deltaPercent: 10
      }
    ]);

    assert.match(markdown, /Baseline Comparison/);
    assert.match(markdown, /\| Case \| Category \| Phase \| Opt \| Mode \|/);
    assert.match(markdown, /1\.10x/);
    assert.match(markdown, /10\.00%/);
  });

  it("detects whether fail-on-regression should trip", () => {
    assert.equal(hasRegression([{ status: "ok" }, { status: "warning" }]), false);
    assert.equal(hasRegression([{ status: "ok" }, { status: "regression" }]), true);
  });

  it("builds hyperfine arguments with named commands and exports", () => {
    const args = buildHyperfineArgs(
      { runs: 20, warmup: 3 },
      [
        { name: "pricing-c-unchecked", command: "build/perf/bin/pricing-c-unchecked --items 100000 --iterations 1000" },
        { name: "pricing-js-bigint", command: "node bench/perf/cases/pricing-js-bigint.mjs --items 100000 --iterations 1000" }
      ],
      {
        json: "build/perf/latest.hyperfine.json",
        markdown: "build/perf/latest.hyperfine.md"
      }
    );

    assert.deepEqual(args.slice(0, 8), [
      "--warmup",
      "3",
      "--runs",
      "20",
      "--export-json",
      "build/perf/latest.hyperfine.json",
      "--export-markdown",
      "build/perf/latest.hyperfine.md"
    ]);
    assert.ok(args.includes("--command-name"));
    assert.ok(args.includes("pricing-c-unchecked"));
    assert.ok(args.includes("node bench/perf/cases/pricing-js-bigint.mjs --items 100000 --iterations 1000"));
  });
});
