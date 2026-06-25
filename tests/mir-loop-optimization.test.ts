import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";
import { lowerToMir } from "../src/mir/lower.js";
import type { MirFunction, MirModule } from "../src/mir/mir.js";
import { printMirModule } from "../src/mir/mir-printer.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { runMirPassPipeline } from "../src/opt/mir-pass-manager.js";
import { buildMirOptimizationPipeline } from "../src/opt/pipeline.js";
import { analyzeInductionVariables } from "../src/opt/passes/induction-simplify.js";
import { analyzeNaturalLoops } from "../src/opt/passes/loop-analysis.js";
import { loopInvariantCodeMotionPass } from "../src/opt/passes/loop-invariant-code-motion.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror", "-DIK_BUILD_DLL"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-loop-opt-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

function lower(sourceText: string): MirModule {
  const checked = check(new SourceFile("loop.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return mir;
}

function firstFunction(module: MirModule): MirFunction {
  return module.functions[0]!;
}

function runLicm(module: MirModule, overflowMode: "unchecked" | "checked" = "unchecked"): MirModule {
  const result = runMirPassPipeline(module, { optLevel: 3, passes: [loopInvariantCodeMotionPass], validateAfterEachPass: true }, {
    optLevel: 3,
    overflowMode,
    targetBackend: "mir",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

function optimize(sourceText: string, optLevel: 0 | 1 | 2 | 3, overflowMode: "unchecked" | "checked" = "unchecked"): MirModule {
  const module = lower(sourceText);
  const result = runMirPassPipeline(module, buildMirOptimizationPipeline(optLevel), {
    optLevel,
    overflowMode,
    targetBackend: "mir",
    debug: {}
  });
  expect(result.validationErrors).toEqual([]);
  return result.module;
}

function invariantLoopSource(): string {
  return `
    export fn calc(n: i64, a: i64, b: i64) -> i64 {
      let i: i64 = 0;
      let sum: i64 = 0;

      while i < n {
        sum = sum + (a * b + 7);
        i = i + 1;
      }

      return sum;
    }
  `;
}

describe("MIR loop optimization", () => {
  it("recognizes a simple natural while loop", () => {
    const func = firstFunction(
      lower(`
        export fn sum_to_n(n: i64) -> i64 {
          let i: i64 = 0;
          let sum: i64 = 0;

          while i < n {
            sum = sum + i;
            i = i + 1;
          }

          return sum;
        }
      `)
    );

    const loops = analyzeNaturalLoops(func);

    expect(loops).toHaveLength(1);
    expect(loops[0]).toMatchObject({
      header: "bb1",
      preheader: "bb0",
      backEdge: { from: "bb2", to: "bb1" },
      exitBlocks: ["bb3"]
    });
    expect([...loops[0]!.blocks].sort()).toEqual(["bb1", "bb2"]);
  });

  it("hoists invariant const, add, and mul out of a simple unchecked loop", () => {
    const optimized = runLicm(lower(invariantLoopSource()));
    const text = printMirModule(optimized);
    const func = firstFunction(optimized);
    const preheader = func.blocks.find((block) => block.label === "bb0")!;
    const body = func.blocks.find((block) => block.label === "bb2")!;

    expect(preheader.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "*" && instruction.left.kind === "param")).toBe(true);
    expect(preheader.instructions.some((instruction) => instruction.kind === "const_int" && instruction.value === "7")).toBe(true);
    expect(preheader.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "+")).toBe(true);
    expect(body.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "*" && instruction.left.kind === "param")).toBe(false);
    expect(text).toContain("jump bb1");
  });

  it("does not hoist load, store, call-risky memory work, or division", () => {
    const optimized = runLicm(
      lower(`
        struct Item {
          price: i64;
        }

        export fn calc(items: ptr<Item>, len: i32, out: ptr<i64>, a: i64, b: i64) -> i32 {
          let i: i32 = 0;

          while i < len {
            let price: i64 = items[i].price;
            let quotient: i64 = a / b;
            out[i] = price + quotient;
            i = i + 1;
          }

          return 0;
        }
      `)
    );
    const func = firstFunction(optimized);
    const preheader = func.blocks.find((block) => block.label === "bb0")!;
    const body = func.blocks.find((block) => block.label === "bb2")!;

    expect(preheader.instructions.some((instruction) => instruction.kind === "load")).toBe(false);
    expect(preheader.instructions.some((instruction) => instruction.kind === "store")).toBe(false);
    expect(preheader.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "/")).toBe(false);
    expect(body.instructions.some((instruction) => instruction.kind === "load")).toBe(true);
    expect(body.instructions.some((instruction) => instruction.kind === "store")).toBe(true);
    expect(body.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "/")).toBe(true);
  });

  it("does not hoist f64 arithmetic out of loops at O3", () => {
    const optimized = runLicm(
      lower(`
        export fn calc(n: i64, a: f64, b: f64) -> f64 {
          let i: i64 = 0;
          let sum: f64 = 0.0;

          while i < n {
            let product: f64 = a * b;
            sum = sum + product;
            i = i + 1;
          }

          return sum;
        }
      `)
    );
    const func = firstFunction(optimized);
    const preheader = func.blocks.find((block) => block.label === "bb0")!;
    const body = func.blocks.find((block) => block.label === "bb2")!;

    expect(preheader.instructions.some((instruction) => instruction.kind === "binary" && instruction.target.type.kind === "primitive" && instruction.target.type.name === "f64")).toBe(false);
    expect(body.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "*" && instruction.target.type.kind === "primitive" && instruction.target.type.name === "f64")).toBe(true);
  });

  it("does not hoist arithmetic in checked mode", () => {
    const optimized = runLicm(lower(invariantLoopSource()), "checked");
    const func = firstFunction(optimized);
    const preheader = func.blocks.find((block) => block.label === "bb0")!;
    const body = func.blocks.find((block) => block.label === "bb2")!;

    expect(preheader.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "*" && instruction.left.kind === "param")).toBe(false);
    expect(body.instructions.some((instruction) => instruction.kind === "binary" && instruction.op === "*" && instruction.left.kind === "param")).toBe(true);
  });

  it("records simple i = i + 1 induction variables without changing MIR", () => {
    const module = lower(`
      export fn sum_to_n(n: i64) -> i64 {
        let i: i64 = 0;
        let sum: i64 = 0;

        while i < n {
          sum = sum + i;
          i = i + 1;
        }

        return sum;
      }
    `);
    const func = firstFunction(module);
    const loop = analyzeNaturalLoops(func)[0]!;
    const before = printMirModule(module);
    const inductions = analyzeInductionVariables(func, loop);
    const after = printMirModule(module);

    expect(inductions).toEqual([{ localName: "i", step: "1", blockLabel: "bb2" }]);
    expect(after).toBe(before);
  });

  it("skips f64 induction-like updates", () => {
    const module = lower(`
      export fn walk(n: f64) -> f64 {
        let i: f64 = 0.0;

        while i < n {
          i = i + 1.0;
        }

        return i;
      }
    `);
    const func = firstFunction(module);
    const loop = analyzeNaturalLoops(func)[0]!;
    const before = printMirModule(module);
    const inductions = analyzeInductionVariables(func, loop);
    const after = printMirModule(module);

    expect(inductions).toEqual([]);
    expect(after).toBe(before);
  });

  it("enables loop optimization only at O3", () => {
    const o2 = printMirModule(optimize(invariantLoopSource(), 2));
    const o3 = printMirModule(optimize(invariantLoopSource(), 3));

    expect(o2.indexOf("mul a, b")).toBeGreaterThan(o2.indexOf("bb2:"));
    expect(o3.indexOf("mul a, b")).toBeLessThan(o3.indexOf("bb1:"));
  });
});

describe("MIR loop optimization e2e", () => {
  const clangAvailable = hasClang();

  it.skipIf(!clangAvailable)(
    clangAvailable ? "compiles and runs an O3 optimized sum_to_n loop" : "compiles and runs an O3 optimized sum_to_n loop (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      writeFileSync(
        join(cwd, "sum.ik"),
        `
export fn sum_to_n(n: i64) -> i64 {
  let i: i64 = 0;
  let sum: i64 = 0;

  while i < n {
    sum = sum + i;
    i = i + 1;
  }

  return sum;
}
`.trimStart()
      );
      const cFile = join(cwd, "build/sum.c");
      const hFile = join(cwd, "build/sum.h");
      expect(runCli(["emit-c", "sum.ik", "--out", cFile, "--header", hFile, "-O3"], { cwd, stdout: () => {}, stderr: () => {} })).toBe(0);

      const harnessFile = join(cwd, "build/sum_harness.c");
      const executable = join(cwd, "build/sum_harness");
      writeFileSync(
        harnessFile,
        `
#include "sum.h"

int main(void) {
  if (sum_to_n(5) != 10) {
    return 10;
  }
  if (sum_to_n(100) != 4950) {
    return 11;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], { encoding: "utf8" });
      expect(compile.status, compile.stderr).toBe(0);
      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );

  it.skipIf(!clangAvailable)(
    clangAvailable ? "compiles and runs O3 optimized pricing" : "compiles and runs O3 optimized pricing (skipped because clang was not found)",
    () => {
      const cwd = tempDir();
      writeFileSync(join(cwd, "pricing.ik"), readFileSync("examples/pricing.ik", "utf8"));
      const cFile = join(cwd, "build/pricing.c");
      const hFile = join(cwd, "build/pricing.h");
      expect(runCli(["emit-c", "pricing.ik", "--out", cFile, "--header", hFile, "-O3"], { cwd, stdout: () => {}, stderr: () => {} })).toBe(0);

      const harnessFile = join(cwd, "build/pricing_harness.c");
      const executable = join(cwd, "build/pricing_harness");
      writeFileSync(
        harnessFile,
        `
#include "pricing.h"

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};

  if (calc_items(items, 2, out) != 0) {
    return 10;
  }
  if (out[0] != 20567 || out[1] != 11000) {
    return 11;
  }
  return 0;
}
`.trimStart()
      );

      const compile = spawnSync("clang", [...strictClangFlags, cFile, harnessFile, "-I", join(cwd, "build"), "-o", executable], { encoding: "utf8" });
      expect(compile.status, compile.stderr).toBe(0);
      const run = spawnSync(executable, [], { encoding: "utf8" });
      expect(run.status, run.stderr || run.stdout).toBe(0);
    }
  );
});
