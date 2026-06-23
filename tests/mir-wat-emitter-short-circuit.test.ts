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

describe("MIR WAT emitter short-circuit regression", () => {
  it("emits short-circuit logical operators as control-flow WAT", () => {
    const sourceText = readFileSync("examples/wasm_short_circuit.ik", "utf8");
    const checked = check(new SourceFile("wasm_short_circuit.ik", sourceText));
    expect(checked.diagnostics).toEqual([]);
    const mir = lowerToMir(checked.checkedProgram);
    expect(validateMirModule(mir).errors).toEqual([]);

    const wat = emitMirWatModule(mir);

    expect(wat).toContain("br_table");
    expect(wat).toContain("i64.div_s");
    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/wasm_short_circuit.wat.snap", "utf8")));
  });
});
