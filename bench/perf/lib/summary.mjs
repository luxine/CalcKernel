import { median, percentile, relativeToFastest } from "./stats.mjs";

export function summarizeHyperfine(data, metadata, commands = []) {
  const commandByName = new Map(commands.map((command) => [command.name, command]));
  const resultsWithFastest = relativeToFastest(
    data.results.map((result) => {
      const times = Array.isArray(result.times) ? result.times : [];
      const command = commandByName.get(result.command);
      return {
        name: result.command,
        category: command?.category ?? "unknown",
        optLevel: command?.optLevel ?? "n/a",
        overflowMode: command?.overflowMode ?? "unknown",
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
  const cUncheckedO3 = resultsWithFastest.find((result) => result.name === "pricing-c-unchecked-O3");
  const results = resultsWithFastest.map((result) => ({
    ...result,
    ratioToCUncheckedO3: cUncheckedO3 ? result.medianSeconds / cUncheckedO3.medianSeconds : null
  }));

  return {
    metadata,
    results
  };
}

export function parseBaselineSummary(text, fileName = "baseline") {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new Error(`Invalid baseline summary ${fileName}: ${error instanceof Error ? error.message : String(error)}`);
  }

  if (
    parsed === null ||
    typeof parsed !== "object" ||
    !Array.isArray(parsed.results) ||
    parsed.results.some((result) => typeof result?.name !== "string" || typeof result?.medianSeconds !== "number")
  ) {
    throw new Error(`Invalid baseline summary ${fileName}: expected a summary object with result names and medianSeconds.`);
  }

  return parsed;
}

export function filterBenchmarkCommands(commands, selectors = []) {
  if (!Array.isArray(selectors) || selectors.length === 0) {
    return commands;
  }

  const filtered = commands.filter((command) =>
    selectors.some((selector) => command.name === selector || command.name.startsWith(`${selector}-`))
  );

  if (filtered.length === 0) {
    throw new Error(`No benchmark cases matched: ${selectors.join(", ")}`);
  }

  return filtered;
}

export function compareSummaries(current, baseline, options = {}) {
  const threshold = (options.thresholdPercent ?? 10) / 100;
  const warningThreshold = threshold / 2;
  const baselineByName = new Map(baseline.results.map((result) => [result.name, result]));

  return current.results.map((result) => {
    const baselineResult = baselineByName.get(result.name);
    if (!baselineResult) {
      return {
        name: result.name,
        status: "missing-baseline",
        currentMedianSeconds: result.medianSeconds,
        baselineMedianSeconds: null,
        ratio: null,
        deltaRatio: null,
        deltaPercent: null
      };
    }

    const ratio = result.medianSeconds / baselineResult.medianSeconds;
    const deltaRatio = ratio - 1;
    const deltaPercent = deltaRatio * 100;
    const status = deltaRatio > threshold ? "regression" : deltaRatio > warningThreshold ? "warning" : "ok";

    return {
      name: result.name,
      status,
      currentMedianSeconds: result.medianSeconds,
      baselineMedianSeconds: baselineResult.medianSeconds,
      ratio,
      deltaRatio,
      deltaPercent
    };
  });
}

export function hasRegression(comparison) {
  return comparison.some((entry) => entry.status === "regression");
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
  lines.push("| Case | Category | Opt | Mode | Median ms | p95 ms | Min ms | Mean ms | Stddev ms | Fastest | vs C O3 |");
  lines.push("| --- | --- | --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");

  for (const result of summary.results) {
    lines.push(
      `| ${result.name} | ${result.category} | ${result.optLevel} | ${result.overflowMode} | ${ms(result.medianSeconds)} | ${ms(result.p95Seconds)} | ${ms(result.minSeconds)} | ${ms(result.meanSeconds)} | ${ms(result.stddevSeconds)} | ${result.relativeToFastest.toFixed(2)}x | ${ratio(result.ratioToCUncheckedO3)} |`
    );
  }

  if (comparison.length > 0) {
    lines.push("");
    lines.push("## Baseline Comparison");
    lines.push("");
    lines.push("| Case | Status | Current ms | Baseline ms | Ratio | Slower |");
    lines.push("| --- | --- | ---: | ---: | ---: | ---: |");

    for (const entry of comparison) {
      const baseline = entry.baselineMedianSeconds === null ? "n/a" : ms(entry.baselineMedianSeconds);
      const ratioText = entry.ratio === null ? "n/a" : `${entry.ratio.toFixed(2)}x`;
      const delta = entry.deltaPercent === null ? "n/a" : `${entry.deltaPercent.toFixed(2)}%`;
      lines.push(`| ${entry.name} | ${entry.status} | ${ms(entry.currentMedianSeconds)} | ${baseline} | ${ratioText} | ${delta} |`);
    }
  }

  lines.push("");
  return `${lines.join("\n")}\n`;
}

function ms(seconds) {
  return (seconds * 1000).toFixed(3);
}

function ratio(value) {
  return value === null ? "n/a" : `${value.toFixed(2)}x`;
}
