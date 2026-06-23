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

describe("MIR WAT emitter pricing", () => {
  it("emits examples/pricing.ik as stable WAT", () => {
    const sourceText = readFileSync("examples/pricing.ik", "utf8");
    const checked = check(new SourceFile("pricing.ik", sourceText));
    expect(checked.diagnostics).toEqual([]);
    const mir = lowerToMir(checked.checkedProgram);
    expect(validateMirModule(mir).errors).toEqual([]);

    const wat = emitMirWatModule(mir);

    expect(wat).toContain('(func $calc_items (export "calc_items")');
    expect(wat).toContain("i64.load offset=0 align=8");
    expect(wat).toContain("i64.store offset=0 align=8");
    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/pricing.wat.snap", "utf8")));
  });
});
