import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { emitMirLlvmModule } from "../src/backend/llvm/mir-llvm-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function normalizeNewlines(text: string): string {
  return text.replaceAll("\r\n", "\n");
}

function emitFixtureLlvm(): string {
  const sourceText = readFileSync("examples/llvm_memory.ik", "utf8");
  const checked = check(new SourceFile("llvm_memory.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName: "llvm_memory.ik" });
}

describe("LLVM memory emitter", () => {
  it("emits stable LLVM IR for ptr/index/field load and store", () => {
    const llvm = emitFixtureLlvm();

    expect(llvm).toContain("%struct.Item = type { i64, i64, i64, i64 }");
    expect(llvm).toContain("getelementptr %struct.Item");
    expect(llvm).toContain("getelementptr i64");
    expect(llvm).toBe(normalizeNewlines(readFileSync("tests/snapshots/llvm_memory.ll.snap", "utf8")));
  });
});
