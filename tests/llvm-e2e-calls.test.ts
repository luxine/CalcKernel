import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-llvm-calls-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM function-call end-to-end", () => {
  it("compiles generated LLVM IR and runs nested calls through the exported function", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_calls.ik"), readFileSync("examples/llvm_calls.ik", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_calls.ik", "--out", "build/llvm_calls.ll"], {
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
    const executable = join(cwd, "build/llvm_calls_test");
    writeFileSync(
      harnessFile,
      `#include <stdint.h>

int64_t calc(int64_t a, int64_t b);

int main(void) {
  if (calc(1, 2) != 6) {
    return 1;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_calls.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });
});
