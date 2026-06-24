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
  const sourceText = readFileSync("examples/llvm_calls.ik", "utf8");
  const checked = check(new SourceFile("llvm_calls.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName: "llvm_calls.ik" });
}

describe("LLVM function-call emitter", () => {
  it("emits stable LLVM IR for nested function calls", () => {
    const llvm = emitFixtureLlvm();

    expect(llvm).toContain("define internal i64 @add_i64");
    expect(llvm).toContain("define internal i64 @double_i64");
    expect(llvm).toContain("define i64 @calc");
    expect(llvm).toBe(normalizeNewlines(readFileSync("tests/snapshots/llvm_calls.ll.snap", "utf8")));
  });
});
