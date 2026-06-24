import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { parseArgs } from "../lib/args.mjs";
import { buildHyperfineArgs } from "../lib/hyperfine.mjs";
import { compareSummaries, summarizeHyperfine } from "../lib/summary.mjs";
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
    const config = parseArgs(["--full", "--save-baseline", "--compare", "--fail-on-regression"]);

    assert.equal(config.mode, "full");
    assert.equal(config.items, 100000);
    assert.equal(config.iterations, 1000);
    assert.equal(config.runs, 20);
    assert.equal(config.warmup, 3);
    assert.equal(config.saveBaseline, true);
    assert.equal(config.compare, true);
    assert.equal(config.failOnRegression, true);
  });

  it("rejects invalid numeric flags", () => {
    assert.throws(() => parseArgs(["--items", "0"]), /--items must be a positive integer/);
    assert.throws(() => parseArgs(["--iterations", "nan"]), /--iterations must be a positive integer/);
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
    const summary = summarizeHyperfine(
      {
        results: [
          {
            command: "pricing-c-unchecked",
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
      }
    );

    assert.equal(summary.metadata.mode, "full");
    assert.equal(summary.results[0].name, "pricing-c-unchecked");
    assert.equal(summary.results[0].medianSeconds, 0.11);
    assert.equal(summary.results[0].p95Seconds, 0.15);
    assert.equal(summary.results[0].relativeToFastest, 1);
  });

  it("compares summaries with ok, warning, and regression thresholds", () => {
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

    const comparison = compareSummaries(current, baseline);

    assert.deepEqual(
      comparison.map((entry) => [entry.name, entry.status]),
      [
        ["ok-case", "ok"],
        ["warning-case", "warning"],
        ["regression-case", "regression"]
      ]
    );
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
