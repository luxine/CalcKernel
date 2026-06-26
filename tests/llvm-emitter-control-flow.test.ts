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
  const sourceText = readFileSync("examples/llvm_control_flow.ck", "utf8");
  const checked = check(new SourceFile("llvm_control_flow.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName: "llvm_control_flow.ck" });
}

describe("LLVM control-flow emitter", () => {
  it("emits stable LLVM IR for if/else and while MIR blocks", () => {
    expect(emitFixtureLlvm()).toBe(normalizeNewlines(readFileSync("tests/snapshots/llvm_control_flow.ll.snap", "utf8")));
  });
});
