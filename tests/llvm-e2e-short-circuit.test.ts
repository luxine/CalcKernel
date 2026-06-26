import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "calckernel-llvm-short-circuit-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM short-circuit end-to-end", () => {
  it("does not evaluate RHS on short-circuit paths", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_short_circuit.ck"), readFileSync("examples/llvm_short_circuit.ck", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_short_circuit.ck", "--out", "build/llvm_short_circuit.ll"], {
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
    const executable = join(cwd, "build/llvm_short_circuit_test");
    writeFileSync(
      harnessFile,
      `#include <stdbool.h>
#include <stdint.h>

bool and_short_circuit(int64_t a, int64_t b);
bool or_short_circuit(int64_t a, int64_t b);

int main(void) {
  if (and_short_circuit(0, 10) != false) {
    return 1;
  }
  if (and_short_circuit(2, 10) != true) {
    return 2;
  }
  if (or_short_circuit(0, 10) != true) {
    return 3;
  }
  if (or_short_circuit(2, 10) != true) {
    return 4;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_short_circuit.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });
});
