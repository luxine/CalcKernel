import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { emitMirWatModule } from "../src/backend/wasm/mir-wat-emitter.js";
import { lowerToMir } from "../src/mir/lower.js";
import { validateMirModule } from "../src/mir/mir-validator.js";
import { SourceFile } from "../src/source/source-file.js";
import { check } from "../src/typeck/checker.js";

function normalizeNewlines(text: string): string {
  return text.replaceAll("\r\n", "\n");
}

describe("MIR WAT emitter control flow", () => {
  it("emits basic blocks with jump, branch, and return as stable WAT", () => {
    const sourceText = readFileSync("examples/wasm_control_flow.ck", "utf8");
    const checked = check(new SourceFile("wasm_control_flow.ck", sourceText));
    expect(checked.diagnostics).toEqual([]);
    const mir = lowerToMir(checked.checkedProgram);
    expect(validateMirModule(mir).errors).toEqual([]);

    const wat = emitMirWatModule(mir);

    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/wasm_control_flow.wat.snap", "utf8")));
  });
});
