import { spawnSync } from "node:child_process";
import { mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { runCli } from "../src/cli.js";

const strictClangFlags = ["-std=c11", "-O3", "-Wall", "-Wextra", "-Werror"];

function tempDir(): string {
  return mkdtempSync(join(tmpdir(), "intkernel-llvm-bool-e2e-"));
}

function hasClang(): boolean {
  return spawnSync("clang", ["--version"], { encoding: "utf8" }).status === 0;
}

describe("LLVM bool ABI end-to-end", () => {
  it("compiles generated LLVM IR and calls bool params/returns from a C harness", () => {
    const cwd = tempDir();
    writeFileSync(join(cwd, "llvm_bool.ik"), readFileSync("examples/llvm_bool.ik", "utf8"));

    const emitExitCode = runCli(["emit-llvm", "llvm_bool.ik", "--out", "build/llvm_bool.ll"], {
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
    const executable = join(cwd, "build/llvm_bool_test");
    writeFileSync(
      harnessFile,
      `#include <stdbool.h>
#include <stdint.h>

bool not_bool(bool a);
bool bool_local(bool a);
int32_t choose_bool(bool a, int32_t x, int32_t y);

int main(void) {
  if (not_bool(true) != false) {
    return 1;
  }
  if (not_bool(false) != true) {
    return 2;
  }
  if (bool_local(true) != false) {
    return 3;
  }
  if (bool_local(false) != true) {
    return 4;
  }
  if (choose_bool(true, 10, 20) != 10) {
    return 5;
  }
  if (choose_bool(false, 10, 20) != 20) {
    return 6;
  }
  return 0;
}
`
    );

    const compile = spawnSync(
      "clang",
      [...strictClangFlags, join(cwd, "build/llvm_bool.ll"), harnessFile, "-o", executable],
      { encoding: "utf8" }
    );
    expect(compile.status, compile.stderr || compile.stdout).toBe(0);

    const run = spawnSync(executable, [], { encoding: "utf8" });
    expect(run.status, run.stderr || run.stdout).toBe(0);
  });
});
