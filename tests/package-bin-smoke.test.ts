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

function expectNonEmptyFile(path: string): void {
  expect(existsSync(path), `${path} should exist`).toBe(true);
  expect(statSync(path).size, `${path} should be non-empty`).toBeGreaterThan(0);
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
    const pack = run("npm", ["pack", "--silent"], rootDir);
    const tarballName = pack.stdout.trim().split(/\r?\n/).at(-1);
    expect(tarballName).toMatch(/^calckernel-.+\.tgz$/);

    const tarballPath = resolve(rootDir, tarballName!);
    const cwd = mkdtempSync(join(tmpdir(), "calckernel-package-bin-"));

    try {
      run("npm", ["init", "-y"], cwd);
      run("npm", ["install", tarballPath], cwd);
      writeFileSync(join(cwd, "smoke.ck"), f64SmokeSource);

      const ckc = join(cwd, "node_modules/.bin/ckc");
      const legacyBin = join(cwd, "node_modules/.bin", "i" + "kc");
      expect(existsSync(legacyBin)).toBe(false);

      const help = run(ckc, ["--help"], cwd);
      expect(help.stdout).toContain("ckc check <file>");

      const check = run(ckc, ["check", "smoke.ck"], cwd);
      expect(check.stdout).toContain("OK: smoke.ck");

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

      run(ckc, ["emit-llvm", "smoke.ck", "-o", "smoke.ll"], cwd);
      expectNonEmptyFile(join(cwd, "smoke.ll"));
      expect(readFileSync(join(cwd, "smoke.ll"), "utf8")).toContain("define double @calc_f64");
    } finally {
      rmSync(cwd, { recursive: true, force: true });
      unlinkSync(tarballPath);
    }
  });
});
