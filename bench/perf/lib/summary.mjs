import { median, percentile, relativeToFastest } from "./stats.mjs";

export function summarizeHyperfine(data, metadata) {
  const results = relativeToFastest(
    data.results.map((result) => {
      const times = Array.isArray(result.times) ? result.times : [];
      return {
        name: result.command,
        meanSeconds: result.mean,
        stddevSeconds: result.stddev,
        minSeconds: result.min,
        maxSeconds: result.max,
        medianSeconds: result.median ?? median(times),
        p95Seconds: times.length > 0 ? percentile(times, 0.95) : result.max,
        samples: times.length
      };
    })
  );

  return {
    metadata,
    results
  };
}

export function compareSummaries(current, baseline) {
  const baselineByName = new Map(baseline.results.map((result) => [result.name, result]));

  return current.results.map((result) => {
    const baselineResult = baselineByName.get(result.name);
    if (!baselineResult) {
      return {
        name: result.name,
        status: "missing-baseline",
        currentMedianSeconds: result.medianSeconds,
        baselineMedianSeconds: null,
        deltaRatio: null
      };
    }

    const deltaRatio = result.medianSeconds / baselineResult.medianSeconds - 1;
    const status = deltaRatio > 0.1 ? "regression" : deltaRatio > 0.05 ? "warning" : "ok";

    return {
      name: result.name,
      status,
      currentMedianSeconds: result.medianSeconds,
      baselineMedianSeconds: baselineResult.medianSeconds,
      deltaRatio
    };
  });
}

export function formatSummaryMarkdown(summary, comparison = []) {
  const lines = [];
  const metadata = summary.metadata;

  lines.push("# IntKernel Local Performance Summary");
  lines.push("");
  lines.push(`Generated at: ${metadata.generatedAt}`);
  lines.push("");
  lines.push("## Environment");
  lines.push("");
  lines.push(`- Mode: ${metadata.mode}`);
  lines.push(`- Items: ${metadata.items}`);
  lines.push(`- Iterations per command: ${metadata.iterations}`);
  lines.push(`- Hyperfine runs: ${metadata.runs}`);
  lines.push(`- Hyperfine warmup: ${metadata.warmup}`);
  lines.push(`- Git SHA: ${metadata.gitSha}`);
  lines.push(`- Dirty worktree: ${metadata.dirty ? "yes" : "no"}`);
  lines.push(`- Node: ${metadata.nodeVersion}`);
  lines.push(`- Clang: ${metadata.clangVersion}`);
  lines.push(`- Hyperfine: ${metadata.hyperfineVersion}`);
  lines.push(`- Platform: ${metadata.platform} ${metadata.arch}`);
  lines.push(`- CPU: ${metadata.cpuModel}`);
  lines.push("");
  lines.push("## Results");
  lines.push("");
  lines.push("| Case | Median ms | p95 ms | Min ms | Mean ms | Stddev ms | Relative |");
  lines.push("| --- | ---: | ---: | ---: | ---: | ---: | ---: |");

  for (const result of summary.results) {
    lines.push(
      `| ${result.name} | ${ms(result.medianSeconds)} | ${ms(result.p95Seconds)} | ${ms(result.minSeconds)} | ${ms(result.meanSeconds)} | ${ms(result.stddevSeconds)} | ${result.relativeToFastest.toFixed(2)}x |`
    );
  }

  if (comparison.length > 0) {
    lines.push("");
    lines.push("## Baseline Comparison");
    lines.push("");
    lines.push("| Case | Status | Current ms | Baseline ms | Delta |");
    lines.push("| --- | --- | ---: | ---: | ---: |");

    for (const entry of comparison) {
      const baseline = entry.baselineMedianSeconds === null ? "n/a" : ms(entry.baselineMedianSeconds);
      const delta = entry.deltaRatio === null ? "n/a" : `${(entry.deltaRatio * 100).toFixed(2)}%`;
      lines.push(`| ${entry.name} | ${entry.status} | ${ms(entry.currentMedianSeconds)} | ${baseline} | ${delta} |`);
    }
  }

  lines.push("");
  return `${lines.join("\n")}\n`;
}

function ms(seconds) {
  return (seconds * 1000).toFixed(3);
}
