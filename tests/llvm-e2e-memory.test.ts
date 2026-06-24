import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-llvm-memory-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM memory end-to-end", () => {
  it("compiles generated LLVM IR and runs ptr/index/field load/store functions", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_memory.ik"), readFileSync("examples/llvm_memory.ik", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_memory.ik", "--out", "build/llvm_memory.ll"], {
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
});
