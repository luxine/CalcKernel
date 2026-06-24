import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-llvm-pricing-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM pricing end-to-end", () => {
  it("compiles generated LLVM IR and runs calc_items through a C harness", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "pricing.ik"), readFileSync("examples/pricing.ik", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "pricing.ik", "--out", "build/pricing.ll"], {
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
    const executable = join(cwd, "build/pricing_llvm_test");
    writeFileSync(
      harnessFile,
      `#include <stdint.h>

typedef struct Item {
  int64_t price;
  int64_t qty;
  int64_t discount;
  int64_t tax_rate_ppm;
} Item;

int32_t calc_items(Item* items, int32_t len, int64_t* out);

int main(void) {
  Item items[2] = {
    { .price = 10000, .qty = 2, .discount = 1000, .tax_rate_ppm = 82500 },
    { .price = 2500, .qty = 4, .discount = 0, .tax_rate_ppm = 100000 }
  };
  int64_t out[2] = {0, 0};

  if (calc_items(items, 2, out) != 0) {
    return 10;
  }
  if (out[0] != 20567) {
    return 20;
  }
  if (out[1] != 11000) {
    return 21;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/pricing.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });
});
