import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { describe, expect, it } from "vitest";

const rootDir = process.cwd();

function run(command: string, args: string[], cwd: string): { stdout: string; stderr: string } {
  const result = spawnSync(command, args, { cwd, encoding: "utf8" });
  expect(result.status, `${command} ${args.join(" ")}\nstdout:\n${result.stdout}\nstderr:\n${result.stderr}`).toBe(0);
  return { stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

function commandSucceeds(command: string, args: string[], cwd: string): boolean {
  return spawnSync(command, args, { cwd, encoding: "utf8" }).status === 0;
}

function expectNonEmptyFile(path: string): void {
  expect(existsSync(path), `${path} should exist`).toBe(true);
  expect(statSync(path).size, `${path} should be non-empty`).toBeGreaterThan(0);
}

function expectPackageContentsClean(paths: string[]): void {
  expect(paths).toContain("dist/src/index.d.ts");
  expect(paths).toContain("dist/src/index.js");
  expect(paths).toContain("dist/src/cli.js");
  expect(paths).toContain("dist/src/wasm/ck-wasm-arena.d.ts");
  expect(paths).toContain("dist/src/wasm/ck-wasm-arena.js");
  expect(paths).toContain("README.md");
  expect(paths).toContain("docs/wasm-interop.md");
  expect(paths).toContain("docs/releases/v0.8.0.md");
  expect(paths).toContain("examples/wasm/f64-sum/run.mjs");
  expect(paths).toContain("examples/wasm/f64-axpy/run.mjs");
  expect(paths).toContain("examples/wasm/pricing-soa/run.mjs");

  if (existsSync(join(rootDir, "LICENSE"))) {
    expect(paths).toContain("LICENSE");
  }

  const forbiddenPatterns = [
    /^node_modules\//,
    /(^|\/)node_modules\//,
    /^bench\/docs\//,
    /^bench\/plans\//,
    /^coverage\//,
    /(^|\/)coverage\//,
    /(^|\/)\.cache\//,
    /(^|\/)cache\//,
    /(^|\/)(npm-debug|yarn-debug|yarn-error|pnpm-debug)\.log$/,
    /(^|\/)debug\.log$/,
    /^build\/perf\/latest\./,
    /^build\/perf\/baseline\.local\.json$/,
    /^build\/perf\//,
    /^tmp\//,
    /(^|\/)tmp\//,
    /^temp\//,
    /(^|\/)temp\//,
    /(^|\/)\.DS_Store$/,
    /(^|\/)calckernel-.*\.tgz$/
  ];

  const forbidden = paths.filter((path) => forbiddenPatterns.some((pattern) => pattern.test(path)));
  expect(forbidden).toEqual([]);
  expectActivePackageDocsUseCurrentNames(paths);
}

function expectActivePackageDocsUseCurrentNames(paths: string[]): void {
  const legacyPatterns = [
    { label: "legacy project name", pattern: /\bIntKernel\b/g },
    { label: "legacy package name", pattern: /\bintkernel\b/g },
    { label: "legacy compiler command", pattern: /\bikc\b/g },
    { label: "legacy source suffix", pattern: new RegExp("\\." + "i" + "k\\b", "g") },
    { label: "legacy C ABI prefix", pattern: /\bIK_/g }
  ];
  const historicalPackageDocs = [
    /^docs\/(?:zh-CN\/)?MIGRATION(?:_IK_TO_CK)?\.md$/,
    /^docs\/plans\/PHASE_21_RENAME_/
  ];
  const scannedExtensions = /\.(?:md|ts|js|mjs|json|snap|ck|wat)$/;
  const hits: string[] = [];

  for (const path of paths) {
    if (!scannedExtensions.test(path) || historicalPackageDocs.some((pattern) => pattern.test(path))) {
      continue;
    }

    const absolutePath = join(rootDir, path);
    if (!existsSync(absolutePath)) {
      continue;
    }

    const lines = readFileSync(absolutePath, "utf8").split(/\r?\n/);
    for (let index = 0; index < lines.length; index += 1) {
      for (const { label, pattern } of legacyPatterns) {
        pattern.lastIndex = 0;
        if (pattern.test(lines[index]!)) {
          hits.push(`${path}:${index + 1}: ${label}: ${lines[index]!.trim()}`);
        }
      }
    }
  }

  expect(hits).toEqual([]);
}

function npmPackDryRunPaths(): string[] {
  const dryRun = run("npm", ["pack", "--dry-run", "--json"], rootDir);
  const entries = JSON.parse(dryRun.stdout) as Array<{ files?: Array<{ path: string }> }>;
  return entries[0]?.files?.map((file) => file.path) ?? [];
}

const f64SmokeSource = `struct Quote {
  price: f64;
  tax: f64;
}

export fn calc_f64(a: f64, b: f64) -> f64 {
  let neg: f64 = -a;
  return (neg + b) * a / b;
}

export fn le_f64(a: f64, b: f64) -> bool {
  return a <= b;
}

export fn write_scale(values: ptr<f64>, len: i32, factor: f64) -> f64 {
  let i: i32 = 0;
  let sum: f64 = 0.0;
  while i < len {
    let next: f64 = values[i] * factor;
    values[i] = next;
    sum = sum + next;
    i = i + 1;
  }
  return sum;
}

export fn quote_total(quotes: ptr<Quote>, len: i32) -> f64 {
  let i: i32 = 0;
  let total: f64 = 0.0;
  while i < len {
    total = total + quotes[i].price + quotes[i].tax;
    i = i + 1;
  }
  return total;
}
`;

describe("package bin smoke", () => {
  it("runs ckc from an npm fresh install through node_modules/.bin without a legacy bin alias", () => {
    run("pnpm", ["build"], rootDir);
    expectPackageContentsClean(npmPackDryRunPaths());

    const pack = run("npm", ["pack", "--silent"], rootDir);
    const tarballName = pack.stdout.trim().split(/\r?\n/).at(-1);
    expect(tarballName).toMatch(/^calckernel-.+\.tgz$/);

    const tarballPath = resolve(rootDir, tarballName!);
    const cwd = mkdtempSync(join(tmpdir(), "calckernel-package-bin-"));

    try {
      run("npm", ["init", "-y"], cwd);
      run("npm", ["install", tarballPath], cwd);
      writeFileSync(join(cwd, "smoke.ck"), f64SmokeSource);
      writeFileSync(
        join(cwd, "consumer.mjs"),
        `
          import { CKWasmArena, createCKWasmArena } from "calckernel";

          const memory = new WebAssembly.Memory({ initial: 1 });
          const arena = createCKWasmArena({ memory, __ck_heap_base: { value: 0 } });
          const { ptr, view } = arena.copyInF64(new Float64Array([1.5, 2.5]));
          const outputView = arena.viewF64(ptr, view.length);
          const copy = arena.copyOutF64(ptr, outputView.length);

          if (outputView.buffer !== memory.buffer) {
            throw new Error("output view should use WASM memory");
          }
          outputView[0] = 9.5;
          if (copy[0] !== 1.5) {
            throw new Error("copyOutF64 should return a JS-owned copy");
          }

          console.log(\`OK \${CKWasmArena.name} \${outputView[0]}\`);
        `
      );
      writeFileSync(
        join(cwd, "consumer.ts"),
        `
          import { CKWasmArena, createCKWasmArena, type CKWasmArenaCopy, type CKWasmInstanceLike } from "calckernel";

          const memory = new WebAssembly.Memory({ initial: 1 });
          const instanceLike: CKWasmInstanceLike = { exports: { memory, __ck_heap_base: { value: 0 } } };
          const arena: CKWasmArena = createCKWasmArena(instanceLike);
          const copied: CKWasmArenaCopy<Float64Array> = arena.copyInF64(new Float64Array([1.5, 2.5]));
          const outputView: Float64Array = arena.viewF64(copied.ptr, copied.view.length);
          const outputCopy: Float64Array = arena.copyOutF64(copied.ptr, outputView.length);
          outputView.set(outputCopy);
        `
      );
      writeFileSync(
        join(cwd, "wasm-consumer.mjs"),
        `
          import { readFileSync } from "node:fs";
          import { CKWasmArena, createCKWasmArena } from "calckernel";

          function close(actual, expected) {
            return Math.abs(actual - expected) < 0.0000001;
          }

          const bytes = readFileSync(new URL("./smoke.wasm", import.meta.url));
          const { instance } = await WebAssembly.instantiate(bytes);
          const writeScale = instance.exports.write_scale;

          if (typeof writeScale !== "function") {
            throw new Error("generated WASM did not export write_scale");
          }

          const arena = createCKWasmArena(instance);
          if (!(arena instanceof CKWasmArena)) {
            throw new Error("createCKWasmArena should return CKWasmArena");
          }

          const { ptr, view } = arena.copyInF64(new Float64Array([1.0, 2.0, 3.0, 4.0]));
          const checksum = writeScale(ptr, view.length, 2.5);
          const outputView = arena.viewF64(ptr, view.length);
          const outputCopy = arena.copyOutF64(ptr, outputView.length);
          const expected = [2.5, 5.0, 7.5, 10.0];

          if (!close(checksum, 25.0)) {
            throw new Error(\`unexpected checksum: \${checksum}\`);
          }
          for (let i = 0; i < expected.length; i += 1) {
            if (!close(outputView[i], expected[i]) || !close(outputCopy[i], expected[i])) {
              throw new Error(\`unexpected output at \${i}: view=\${outputView[i]} copy=\${outputCopy[i]}\`);
            }
          }

          outputView[0] = 99.0;
          if (!close(outputCopy[0], 2.5)) {
            throw new Error("copyOutF64 should return a JS-owned copy");
          }

          console.log(\`OK wasm interop checksum=\${checksum} output=\${Array.from(outputCopy).join(",")}\`);
        `
      );

      const ckc = join(cwd, "node_modules/.bin/ckc");
      const legacyBin = join(cwd, "node_modules/.bin", "i" + "kc");
      expect(existsSync(legacyBin)).toBe(false);

      const help = run(ckc, ["--help"], cwd);
      expect(help.stdout).toContain("ckc check <file>");

      const check = run(ckc, ["check", "smoke.ck"], cwd);
      expect(check.stdout).toContain("OK: smoke.ck");

      const consumer = run("node", ["consumer.mjs"], cwd);
      expect(consumer.stdout).toContain("OK CKWasmArena 9.5");

      run(
        "node",
        [
          resolve(rootDir, "node_modules/typescript/bin/tsc"),
          "--target",
          "ES2022",
          "--module",
          "NodeNext",
          "--moduleResolution",
          "NodeNext",
          "--lib",
          "ES2022,DOM",
          "--strict",
          "--noEmit",
          "consumer.ts"
        ],
        cwd
      );

      run(ckc, ["emit-mir", "smoke.ck", "-o", "smoke.mir"], cwd);
      expectNonEmptyFile(join(cwd, "smoke.mir"));
      expect(readFileSync(join(cwd, "smoke.mir"), "utf8")).toContain("const_float");

      run(ckc, ["emit-c", "smoke.ck", "-o", "smoke.c"], cwd);
      expectNonEmptyFile(join(cwd, "smoke.c"));
      expectNonEmptyFile(join(cwd, "smoke.h"));
      expect(readFileSync(join(cwd, "smoke.c"), "utf8")).toContain("double calc_f64");

      run(ckc, ["emit-wat", "smoke.ck", "-o", "smoke.wat"], cwd);
      expectNonEmptyFile(join(cwd, "smoke.wat"));
      expect(readFileSync(join(cwd, "smoke.wat"), "utf8")).toContain("(module");

      run(ckc, ["emit-wasm", "smoke.ck", "-o", "smoke.wasm"], cwd);
      expectNonEmptyFile(join(cwd, "smoke.wasm"));
      expect([...readFileSync(join(cwd, "smoke.wasm")).subarray(0, 4)]).toEqual([0x00, 0x61, 0x73, 0x6d]);

      const wasmInteropConsumer = run("node", ["wasm-consumer.mjs"], cwd);
      expect(wasmInteropConsumer.stdout).toContain("OK wasm interop");

      run(ckc, ["emit-llvm", "smoke.ck", "-o", "smoke.ll"], cwd);
      expectNonEmptyFile(join(cwd, "smoke.ll"));
      expect(readFileSync(join(cwd, "smoke.ll"), "utf8")).toContain("define double @calc_f64");

      if (commandSucceeds("clang", ["--version"], cwd)) {
        run(ckc, ["build-llvm", "smoke.ck", "--kind", "object", "-o", "smoke.o"], cwd);
        expectNonEmptyFile(join(cwd, "smoke.o"));
      }
    } finally {
      rmSync(cwd, { recursive: true, force: true });
      unlinkSync(tarballPath);
    }
  });
});
