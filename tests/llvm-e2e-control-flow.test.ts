import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "calckernel-llvm-control-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM control-flow end-to-end", () => {
  it("compiles generated LLVM IR and runs if/else and while functions", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_control_flow.ck"), readFileSync("examples/llvm_control_flow.ck", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_control_flow.ck", "--out", "build/llvm_control_flow.ll"], {
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
    const executable = join(cwd, "build/llvm_control_flow_test");
    writeFileSync(
      harnessFile,
      `#include <stdint.h>

int32_t max_i32(int32_t a, int32_t b);
int64_t sum_to_n(int64_t n);

int main(void) {
  if (max_i32(10, 3) != 10) {
    return 1;
  }
  if (max_i32(1, 3) != 3) {
    return 2;
  }
  if (sum_to_n(5) != 10) {
    return 3;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_control_flow.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });
});
