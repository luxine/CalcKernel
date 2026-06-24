#!/usr/bin/env node
import { cpus } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { parseArgs } from "./lib/args.mjs";
import { buildHyperfineArgs } from "./lib/hyperfine.mjs";
import { compareSummaries, formatSummaryMarkdown, summarizeHyperfine } from "./lib/summary.mjs";

const scriptDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = resolve(scriptDir, "../..");
const buildDir = join(repoRoot, "build/perf");
const generatedDir = join(buildDir, "generated");
const binDir = join(buildDir, "bin");
const latestHyperfineJson = join(buildDir, "latest.hyperfine.json");
const latestHyperfineMarkdown = join(buildDir, "latest.hyperfine.md");
const latestSummaryJson = join(buildDir, "latest.summary.json");
const latestSummaryMarkdown = join(buildDir, "latest.summary.md");
const baselinePath = join(buildDir, "baseline.local.json");

try {
  main();
} catch (error) {
  process.stderr.write(`${error instanceof Error ? error.message : String(error)}\n`);
  process.exitCode = 1;
}

function main() {
  const config = parseArgs(process.argv.slice(2));

  mkdirSync(generatedDir, { recursive: true });
  mkdirSync(binDir, { recursive: true });

  const toolVersions = checkTools();
  run("pnpm", ["build"]);
  generateArtifacts();
  compileCCommands();

  const commands = benchmarkCommands(config);
  smoke(commands);

  const hyperfineArgs = buildHyperfineArgs(
    config,
    commands,
    {
      json: relativeFromRoot(latestHyperfineJson),
      markdown: relativeFromRoot(latestHyperfineMarkdown)
    }
  );
  run("hyperfine", hyperfineArgs, { stdio: "inherit" });

  const summary = summarizeHyperfine(JSON.parse(readFileSync(latestHyperfineJson, "utf8")), {
    ...config,
    ...metadata(toolVersions)
  });
  let comparison = [];

  if (config.compare) {
    if (!existsSync(baselinePath)) {
      throw new Error(`No local baseline found at ${relativeFromRoot(baselinePath)}. Run with --save-baseline first.`);
    }

    comparison = compareSummaries(summary, JSON.parse(readFileSync(baselinePath, "utf8")));
  }

  writeFileSync(latestSummaryJson, `${JSON.stringify({ ...summary, comparison }, null, 2)}\n`);
  writeFileSync(latestSummaryMarkdown, formatSummaryMarkdown(summary, comparison));

  if (config.saveBaseline) {
    writeFileSync(baselinePath, `${JSON.stringify(summary, null, 2)}\n`);
  }

  process.stdout.write(`Wrote ${relativeFromRoot(latestSummaryJson)}\n`);
  process.stdout.write(`Wrote ${relativeFromRoot(latestSummaryMarkdown)}\n`);
  if (config.saveBaseline) {
    process.stdout.write(`Saved local baseline to ${relativeFromRoot(baselinePath)}\n`);
  }

  if (config.failOnRegression && comparison.some((entry) => entry.status === "regression")) {
    process.exitCode = 1;
  }
}

function checkTools() {
  const versions = {
    nodeVersion: process.version,
    pnpmVersion: requiredVersion("pnpm", ["--version"]),
    clangVersion: firstLine(requiredVersion("clang", ["--version"])),
    hyperfineVersion: requiredVersion("hyperfine", ["--version"])
  };

  return versions;
}

function requiredVersion(command, args) {
  const result = spawnSync(command, args, { cwd: repoRoot, encoding: "utf8" });
  if (result.error?.code === "ENOENT") {
    if (command === "hyperfine") {
      throw new Error("hyperfine is required. Install it with `brew install hyperfine` or `cargo install hyperfine`.");
    }
    throw new Error(`${command} is required but was not found on PATH.`);
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed:\n${result.stderr || result.stdout}`);
  }

  return (result.stdout || result.stderr).trim();
}

function generateArtifacts() {
  run("pnpm", [
    "ikc",
    "emit-c",
    "examples/pricing.ik",
    "--out",
    relativeFromRoot(join(generatedDir, "pricing.c")),
    "--header",
    relativeFromRoot(join(generatedDir, "pricing.h")),
    "--overflow",
    "unchecked"
  ]);
  run("pnpm", [
    "ikc",
    "emit-c",
    "examples/pricing.ik",
    "--out",
    relativeFromRoot(join(generatedDir, "pricing.checked.c")),
    "--header",
    relativeFromRoot(join(generatedDir, "pricing.checked.h")),
    "--overflow",
    "checked"
  ]);
  run("pnpm", [
    "ikc",
    "emit-wasm",
    "examples/pricing.ik",
    "--out",
    relativeFromRoot(join(generatedDir, "pricing.wasm")),
    "--overflow",
    "unchecked"
  ]);
}

function compileCCommands() {
  const common = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror", "-DIK_BUILD_DLL"];
  run("clang", [
    ...common,
    relativeFromRoot(join(generatedDir, "pricing.c")),
    "bench/perf/cases/pricing-c-unchecked.c",
    "-I",
    relativeFromRoot(generatedDir),
    "-o",
    relativeFromRoot(join(binDir, executableName("pricing-c-unchecked")))
  ]);
  run("clang", [
    ...common,
    relativeFromRoot(join(generatedDir, "pricing.checked.c")),
    "bench/perf/cases/pricing-c-checked.c",
    "-I",
    relativeFromRoot(generatedDir),
    "-o",
    relativeFromRoot(join(binDir, executableName("pricing-c-checked")))
  ]);
}

function benchmarkCommands(config) {
  const itemArgs = `--items ${config.items} --iterations ${config.iterations}`;
  return [
    {
      name: "pricing-c-unchecked",
      command: `${relativeFromRoot(join(binDir, executableName("pricing-c-unchecked")))} ${itemArgs}`
    },
    {
      name: "pricing-c-checked",
      command: `${relativeFromRoot(join(binDir, executableName("pricing-c-checked")))} ${itemArgs}`
    },
    {
      name: "pricing-wasm-unchecked",
      command: `node bench/perf/cases/pricing-wasm.mjs --wasm ${relativeFromRoot(join(generatedDir, "pricing.wasm"))} ${itemArgs}`
    },
    {
      name: "pricing-js-bigint",
      command: `node bench/perf/cases/pricing-js-bigint.mjs ${itemArgs}`
    }
  ];
}

function smoke(commands) {
  for (const command of commands) {
    const result = spawnSync(command.command, { cwd: repoRoot, encoding: "utf8", shell: true });
    if (result.status !== 0) {
      throw new Error(`Smoke run failed for ${command.name}:\n${result.stderr || result.stdout}`);
    }
  }
}

function metadata(toolVersions) {
  return {
    generatedAt: new Date().toISOString(),
    gitSha: capture("git", ["rev-parse", "HEAD"]).trim(),
    dirty: capture("git", ["status", "--short"]).trim().length > 0,
    platform: process.platform,
    arch: process.arch,
    cpuModel: cpus()[0]?.model ?? "unknown",
    ...toolVersions
  };
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: options.stdio ?? "pipe"
  });

  if (result.error?.code === "ENOENT") {
    throw new Error(`${command} is required but was not found on PATH.`);
  }
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed:\n${result.stderr || result.stdout}`);
  }

  return result;
}

function capture(command, args) {
  const result = run(command, args);
  return result.stdout;
}

function firstLine(text) {
  return text.split(/\r?\n/)[0] ?? text;
}

function executableName(name) {
  return process.platform === "win32" ? `${name}.exe` : name;
}

function relativeFromRoot(path) {
  return relativePath(repoRoot, path);
}

function relativePath(from, to) {
  return resolve(to).startsWith(resolve(from)) ? resolve(to).slice(resolve(from).length + 1) : to;
}
