import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-llvm-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM scalar end-to-end", () => {
  it("compiles generated LLVM IR with a C harness and runs scalar functions", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_scalar.ik"), readFileSync("examples/llvm_scalar.ik", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_scalar.ik", "--out", "build/llvm_scalar.ll"], {
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
});
