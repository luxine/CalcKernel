import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { emitMirLlvmModule } from "../src/backend/llvm/mir-llvm-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import type { OptimizationLevel } from "../src/optimization/options.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function normalizeNewlines(text: string): string {
  return text.replaceAll("\r\n", "\n");
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

function emitFixtureLlvm(): string {
  const sourceText = readFileSync("examples/llvm_scalar.ik", "utf8");
  const checked = check(new SourceFile("llvm_scalar.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName: "llvm_scalar.ik" });
}

function emitSourceLlvm(sourceText: string, sourceFileName: string, optLevel?: OptimizationLevel): string {
  const checked = check(new SourceFile(sourceFileName, sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName, optLevel });
}

describe("LLVM scalar straight-line emitter", () => {
  it("emits stable LLVM IR for scalar straight-line MIR", () => {
    expect(emitFixtureLlvm()).toBe(normalizeNewlines(readFileSync("tests/snapshots/llvm_scalar.ll.snap", "utf8")));
  });

  it("emitted LLVM IR parses with clang when available", () => {
    if (!hasClang()) {
      console.warn("skipped because clang was not found");
      return;
    }

    const cwd = mkdtempSync(join(tmpdir(), "intkernel-llvm-"));
    const buildDir = join(cwd, "build");
    mkdirSync(buildDir, { recursive: true });

    const llvmFile = join(buildDir, "llvm_scalar.ll");
    const objectFile = join(buildDir, "llvm_scalar.o");
    writeFileSync(llvmFile, emitFixtureLlvm());

    const result = spawnSync("clang", ["-Werror", "-Wno-error=override-module", "-c", llvmFile, "-o", objectFile], { encoding: "utf8" });

    expect(result.status, result.stderr || result.stdout).toBe(0);
  });

  it("emits SSA-like LLVM IR for simple scalar straight-line functions at O2", () => {
    const sourceText = `
      export fn add_i64(a: i64, b: i64) -> i64 {
        return a + b;
      }
    `;
    const checked = check(new SourceFile("llvm_scalar_o2.ik", sourceText));
    expect(checked.diagnostics).toEqual([]);

    const mir = lowerToMir(checked.checkedProgram);
    expect(validateMirModule(mir).errors).toEqual([]);

    const llvm = emitMirLlvmModule(mir, { sourceFileName: "llvm_scalar_o2.ik", optLevel: 2 });

    expect(llvm).toContain("%v0 = add i64 %a, %b");
    expect(llvm).toContain("ret i64 %v0");
    expect(llvm).not.toContain("alloca");
    expect(llvm).not.toContain("load i64");
    expect(llvm).not.toContain("store i64");
  });

  it("emits strict LLVM IR for f64 scalar arithmetic, unary neg, and comparisons", () => {
    const llvm = emitSourceLlvm(
      `
        export fn calc_f64(a: f64, b: f64) -> f64 {
          let one: f64 = 1.0;
          let sum: f64 = a + b;
          let diff: f64 = sum - one;
          let prod: f64 = diff * b;
          return prod / 2.0;
        }

        export fn neg_f64(a: f64) -> f64 {
          return -a;
        }

        export fn eq_f64(a: f64, b: f64) -> bool {
          return a == b;
        }

        export fn ne_f64(a: f64, b: f64) -> bool {
          return a != b;
        }

        export fn lt_f64(a: f64, b: f64) -> bool {
          return a < b;
        }

        export fn le_f64(a: f64, b: f64) -> bool {
          return a <= b;
        }

        export fn gt_f64(a: f64, b: f64) -> bool {
          return a > b;
        }

        export fn ge_f64(a: f64, b: f64) -> bool {
          return a >= b;
        }
      `,
      "llvm_f64_scalar.ik"
    );

    expect(llvm).toContain("define double @calc_f64(double %a, double %b)");
    expect(llvm).toContain("store double 1.0");
    expect(llvm).toContain("store double 2.0");
    expect(llvm).toContain("fadd double");
    expect(llvm).toContain("fsub double");
    expect(llvm).toContain("fmul double");
    expect(llvm).toContain("fdiv double");
    expect(llvm).toContain("fneg double");
    expect(llvm).toContain("fcmp oeq double");
    expect(llvm).toContain("fcmp une double");
    expect(llvm).toContain("fcmp olt double");
    expect(llvm).toContain("fcmp ole double");
    expect(llvm).toContain("fcmp ogt double");
    expect(llvm).toContain("fcmp oge double");
    expect(llvm).not.toContain("fadd fast");
    expect(llvm).not.toContain("fsub fast");
    expect(llvm).not.toContain("fmul fast");
    expect(llvm).not.toContain("fdiv fast");
    expect(llvm).not.toContain("sub double 0");
  });

  it("emits SSA-like no-fast-math LLVM IR for simple f64 functions at O2", () => {
    const llvm = emitSourceLlvm(
      `
        export fn add_f64(a: f64, b: f64) -> f64 {
          return a + b;
        }
      `,
      "llvm_f64_scalar_o2.ik",
      2
    );

    expect(llvm).toContain("%v0 = fadd double %a, %b");
    expect(llvm).toContain("ret double %v0");
    expect(llvm).not.toContain("alloca");
    expect(llvm).not.toContain("load double");
    expect(llvm).not.toContain("store double");
    expect(llvm).not.toContain("fast");
  });
});
