import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { emitMirLlvmModule } from "../src/backend/llvm/mir-llvm-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
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
});
