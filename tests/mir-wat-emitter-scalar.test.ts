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
  const checked = check(new SourceFile("scalar_wat.ik", sourceText));
  expect(checked.diagnostics).toEqual([]);
  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);
  return emitMirWatModule(mir);
}

describe("MIR WAT emitter scalar unchecked", () => {
  it("emits scalar straight-line MIR as stable WAT", () => {
    const wat = lowerAndEmitWat(`
      fn identity_i64(a: i64) -> i64 {
        return a;
      }

      export fn scalar_i32(a: i32, b: i32) -> i32 {
        let x: i32 = a + b;
        x = x - b;
        x = x * b;
        x = x / b;
        return x % b;
      }

      export fn add_i64(a: i64, b: i64) -> i64 {
        return a + b;
      }

      export fn less_i64(a: i64, b: i64) -> bool {
        return a < b;
      }

      export fn div_u64(a: u64, b: u64) -> u64 {
        return a / b;
      }

      export fn gt_u32(a: u32, b: u32) -> bool {
        return a > b;
      }

      export fn neg_i64(a: i64) -> i64 {
        return -a;
      }

      export fn not_bool(a: bool) -> bool {
        return !a;
      }

      export fn bool_true() -> bool {
        return true;
      }
    `);

    expect(wat).toBe(normalizeNewlines(readFileSync("tests/snapshots/scalar.wat.snap", "utf8")));
  });
});
