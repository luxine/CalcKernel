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

function lowerAndEmitWat(sourceText: string): string {
  const checked = check(new SourceFile("wasm_f64_memory.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return emitMirWatModule(mir);
}

describe("MIR WAT emitter memory access", () => {
  it("emits ptr/index/field load and store as stable WAT", () => {
    const sourceText = readFileSync("examples/wasm_memory.ck", "utf8");
    const checked = check(new SourceFile("wasm_memory.ck", sourceText));
    expect(checked.diagnostics).toEqual([]);
    const mir = lowerToMir(checked.checkedProgram);
    expect(validateMirModule(mir).errors).toEqual([]);

    const wat = emitMirWatModule(mir);

    expect(wat).toContain("i64.load offset=0 align=8");
    expect(wat).toContain("i64.store offset=0 align=8");
    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/wasm_memory.wat.snap", "utf8")));
  });

  it("emits f64 load/store for ptr<f64> and struct f64 fields", () => {
    const wat = lowerAndEmitWat(`
      struct Quote {
        price: f64;
        tax: f64;
      }

      export fn write_scale(values: ptr<f64>, i: i32, factor: f64) -> f64 {
        values[i] = values[i] * factor;
        return values[i];
      }

      export fn quote_total(quotes: ptr<Quote>, i: i32) -> f64 {
        return quotes[i].price + quotes[i].tax;
      }
    `);

    expect(wat).toContain("(param $values i32)");
    expect(wat).toContain("(param $factor f64)");
    expect(wat).toContain("(result f64)");
    expect(wat).toContain("i32.const 8");
    expect(wat).toContain("i32.const 16");
    expect(wat).toContain("f64.load offset=0 align=8");
    expect(wat).toContain("f64.store offset=0 align=8");
    expect(wat).toContain("f64.mul");
    expect(wat).toContain("f64.add");
    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/wasm_f64_memory.wat.snap", "utf8")));
  });
});
