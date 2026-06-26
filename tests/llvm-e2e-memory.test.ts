import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "calckernel-llvm-memory-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM memory end-to-end", () => {
  it("compiles generated LLVM IR and runs ptr/index/field load/store functions", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_memory.ck"), readFileSync("examples/llvm_memory.ck", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_memory.ck", "--out", "build/llvm_memory.ll"], {
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
    const executable = join(cwd, "build/llvm_memory_test");
    writeFileSync(
      harnessFile,
      `#include <stdint.h>

typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;

int64_t first_price(Item* items);
int64_t get_price(Item* items, int32_t i);
int32_t write_i64(int64_t* out, int64_t value);

int main(void) {
  Item items[2] = {
    {100, 2, 5, 100000},
    {250, 3, 10, 200000},
  };
  int64_t out[1] = {0};

  if (first_price(items) != 100) {
    return 1;
  }
  if (get_price(items, 1) != 250) {
    return 2;
  }
  if (write_i64(out, 12345) != 0) {
    return 3;
  }
  if (out[0] != 12345) {
    return 4;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_memory.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });

  it("compiles generated LLVM IR and runs ptr<f64> and struct f64 load/store functions", () => {
    const cwd = tempDir();
    writeFileSync(
      join(cwd, "llvm_f64_memory.ck"),
      `
        struct Quote {
          price: f64;
          tax: f64;
        }

        export fn write_scale(values: ptr<f64>, i: i32, factor: f64) -> f64 {
          values[i] = values[i] * factor;
          return values[i];
        }

        export fn quote_total(quotes: ptr<Quote>, i: i32) -> f64 {
          return quotes[i].price + quotes[i].tax;
        }
      `
    );

    const emitExitCode = runCli(["emit-llvm", "llvm_f64_memory.ck", "--out", "build/llvm_f64_memory.ll"], {
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
    const executable = join(cwd, "build/llvm_f64_memory_test");
    writeFileSync(
      harnessFile,
      `#include <math.h>
#include <stdint.h>

typedef struct Quote {
  double price;
  double tax;
} Quote;

double write_scale(double* values, int32_t i, double factor);
double quote_total(Quote* quotes, int32_t i);

static int close_double(double actual, double expected) {
  return fabs(actual - expected) < 0.0000001;
}

int main(void) {
  double values[3] = {1.0, 2.5, 4.0};
  Quote quotes[2] = {
    {10.25, 0.75},
    {20.50, 1.25},
  };

  if (!close_double(write_scale(values, 1, 4.0), 10.0)) {
    return 1;
  }
  if (!close_double(values[1], 10.0)) {
    return 2;
  }
  if (!close_double(quote_total(quotes, 1), 21.75)) {
    return 3;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_f64_memory.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });
});
