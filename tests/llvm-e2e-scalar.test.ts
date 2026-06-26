import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "calckernel-llvm-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM scalar end-to-end", () => {
  it("compiles generated LLVM IR with a C harness and runs scalar functions", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_scalar.ck"), readFileSync("examples/llvm_scalar.ck", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_scalar.ck", "--out", "build/llvm_scalar.ll"], {
      cwd,
      stdout: () => {},
      stderr: () => {}
    });

    expect(emitExitCode).toBe(0);

    if (!hasClang()) {
      console.warn("skipped because clang was not found");
      return;
    }

    const harnessFile = join(cwd, "build/harness.c");
    const executable = join(cwd, "build/llvm_scalar_test");
    writeFileSync(
      harnessFile,
      `#include <stdbool.h>
#include <stdint.h>

int64_t add_i64(int64_t a, int64_t b);
int32_t mul_i32(int32_t a, int32_t b);
bool less_i64(int64_t a, int64_t b);
uint64_t div_u64(uint64_t a, uint64_t b);

int main(void) {
  if (add_i64(1, 2) != 3) {
    return 1;
  }
  if (mul_i32(3, 4) != 12) {
    return 2;
  }
  if (!less_i64(1, 2)) {
    return 3;
  }
  if (div_u64(10, 2) != 5) {
    return 4;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_scalar.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });

  it("compiles generated LLVM IR with a C harness and runs f64 scalar functions", () => {
    const cwd = tempDir();
    writeFileSync(
      join(cwd, "llvm_f64.ck"),
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

        export fn le_f64(a: f64, b: f64) -> bool {
          return a <= b;
        }
      `
    );

    const emitExitCode = runCli(["emit-llvm", "llvm_f64.ck", "--out", "build/llvm_f64.ll"], {
      cwd,
      stdout: () => {},
      stderr: () => {}
    });

    expect(emitExitCode).toBe(0);

    if (!hasClang()) {
      console.warn("skipped because clang was not found");
      return;
    }

    const harnessFile = join(cwd, "build/harness.c");
    const executable = join(cwd, "build/llvm_f64_test");
    writeFileSync(
      harnessFile,
      `#include <math.h>
#include <stdbool.h>

double calc_f64(double a, double b);
double neg_f64(double a);
bool le_f64(double a, double b);

static int close_double(double actual, double expected) {
  return fabs(actual - expected) < 0.0000001;
}

int main(void) {
  if (!close_double(calc_f64(5.0, 3.0), 10.5)) {
    return 1;
  }
  if (!close_double(neg_f64(7.25), -7.25)) {
    return 2;
  }
  if (!le_f64(3.5, 3.5)) {
    return 3;
  }
  if (le_f64(4.5, 3.5)) {
    return 4;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_f64.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });

  it("compiles generated LLVM IR with explicit i32/u32 to f64 casts and runs them from C", () => {
    const cwd = tempDir();
    writeFileSync(
      join(cwd, "llvm_casts.ck"),
      `
        export fn cast_i32_to_f64(value: i32) -> f64 {
          return i32_to_f64(value);
        }

        export fn cast_u32_to_f64(value: u32) -> f64 {
          return u32_to_f64(value);
        }

        export fn cast_expr(a: i32, b: u32) -> f64 {
          return i32_to_f64(a) * 2.0 + u32_to_f64(b) / 2.0;
        }
      `
    );

    const emitExitCode = runCli(["emit-llvm", "llvm_casts.ck", "--out", "build/llvm_casts.ll"], {
      cwd,
      stdout: () => {},
      stderr: () => {}
    });

    expect(emitExitCode).toBe(0);
    const llvm = readFileSync(join(cwd, "build/llvm_casts.ll"), "utf8");
    expect(llvm).toContain("sitofp i32");
    expect(llvm).toContain("uitofp i32");
    expect(llvm).not.toContain("fast");

    if (!hasClang()) {
      console.warn("skipped because clang was not found");
      return;
    }

    const harnessFile = join(cwd, "build/harness.c");
    const executable = join(cwd, "build/llvm_casts_test");
    writeFileSync(
      harnessFile,
      `#include <math.h>
#include <stdint.h>

double cast_i32_to_f64(int32_t value);
double cast_u32_to_f64(uint32_t value);
double cast_expr(int32_t a, uint32_t b);

static int close_double(double actual, double expected) {
  return fabs(actual - expected) < 0.0000001;
}

int main(void) {
  if (!close_double(cast_i32_to_f64(-7), -7.0)) {
    return 1;
  }
  if (!close_double(cast_u32_to_f64((uint32_t)0xfffffffeu), 4294967294.0)) {
    return 2;
  }
  if (!close_double(cast_expr(4, (uint32_t)6), 11.0)) {
    return 3;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_casts.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });

  it("build-llvm emits a native object for f64 scalar functions when clang is available", () => {
    if (!hasClang()) {
      console.warn("skipped because clang was not found");
      return;
    }

    const cwd = tempDir();
    writeFileSync(
      join(cwd, "llvm_f64.ck"),
      `
        export fn add_f64(a: f64, b: f64) -> f64 {
          return a + b;
        }
      `
    );

    let stdout = "";
    let stderr = "";
    const exitCode = runCli(["build-llvm", "llvm_f64.ck", "--kind", "object", "--out", "build/llvm_f64.o"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(0);
    expect(stderr).toBe("");
    expect(stdout).toBe(`OK: built LLVM object\n${join(cwd, "build/llvm_f64.o")}\n`);
    expect(existsSync(join(cwd, "build/llvm_f64.o"))).toBe(true);
    expect(readFileSync(join(cwd, "build/llvm_f64.o")).byteLength).toBeGreaterThan(0);
  });

  it("build-llvm emits a native object for explicit i32/u32 to f64 casts when clang is available", () => {
    if (!hasClang()) {
      console.warn("skipped because clang was not found");
      return;
    }

    const cwd = tempDir();
    writeFileSync(
      join(cwd, "llvm_casts.ck"),
      `
        export fn cast_expr(a: i32, b: u32) -> f64 {
          return i32_to_f64(a) + u32_to_f64(b);
        }
      `
    );

    let stdout = "";
    let stderr = "";
    const exitCode = runCli(["build-llvm", "llvm_casts.ck", "--kind", "object", "--out", "build/llvm_casts.o"], {
      cwd,
      stdout: (text) => {
        stdout += text;
      },
      stderr: (text) => {
        stderr += text;
      }
    });

    expect(exitCode).toBe(0);
    expect(stderr).toBe("");
    expect(stdout).toBe(`OK: built LLVM object\n${join(cwd, "build/llvm_casts.o")}\n`);
    expect(readFileSync(join(cwd, "build/llvm_casts.ll"), "utf8")).toContain("sitofp i32");
    expect(readFileSync(join(cwd, "build/llvm_casts.ll"), "utf8")).toContain("uitofp i32");
    expect(existsSync(join(cwd, "build/llvm_casts.o"))).toBe(true);
    expect(readFileSync(join(cwd, "build/llvm_casts.o")).byteLength).toBeGreaterThan(0);
  });
});
