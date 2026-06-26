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
  const sourceText = readFileSync("examples/llvm_short_circuit.ck", "utf8");
  const checked = check(new SourceFile("llvm_short_circuit.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName: "llvm_short_circuit.ck" });
}

describe("LLVM short-circuit emitter", () => {
  it("emits RHS evaluation in separate blocks for && and ||", () => {
    const llvm = emitFixtureLlvm();

    expect(llvm).toBe(normalizeNewlines(readFileSync("tests/snapshots/llvm_short_circuit.ll.snap", "utf8")));
    expect(llvm).toContain("br i1 %v3, label %bb1, label %bb2");
    expect(llvm).toContain("bb1:\n  %v4 = load i64, ptr %b.addr");
    expect(llvm).toContain("bb2:\n  %v4 = load i64, ptr %b.addr");
    expect(llvm.match(/sdiv i64/g)).toHaveLength(2);
  });
});
