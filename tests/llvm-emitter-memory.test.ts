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
  const sourceText = readFileSync("examples/llvm_memory.ck", "utf8");
  const checked = check(new SourceFile("llvm_memory.ck", sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName: "llvm_memory.ck" });
}

function emitSourceLlvm(sourceText: string, sourceFileName: string): string {
  const checked = check(new SourceFile(sourceFileName, sourceText));
  expect(checked.diagnostics).toEqual([]);

  const mir = lowerToMir(checked.checkedProgram);
  expect(validateMirModule(mir).errors).toEqual([]);

  return emitMirLlvmModule(mir, { sourceFileName });
}

describe("LLVM memory emitter", () => {
  it("emits stable LLVM IR for ptr/index/field load and store", () => {
    const llvm = emitFixtureLlvm();

    expect(llvm).toContain("%struct.Item = type { i64, i64, i64, i64 }");
    expect(llvm).toContain("getelementptr %struct.Item");
    expect(llvm).toContain("getelementptr i64");
    expect(llvm).toBe(normalizeNewlines(readFileSync("tests/snapshots/llvm_memory.ll.snap", "utf8")));
  });

  it("emits LLVM IR for ptr<f64> load/store and struct f64 fields", () => {
    const llvm = emitSourceLlvm(
      `
        struct Quote {
          price: f64;
          tax: f64;
        }

        export fn write_f64(values: ptr<f64>, i: i32, value: f64) -> f64 {
          values[i] = value;
          return values[i];
        }

        export fn quote_total(quotes: ptr<Quote>, i: i32) -> f64 {
          return quotes[i].price + quotes[i].tax;
        }
      `,
      "llvm_f64_memory.ck"
    );

    expect(llvm).toContain("%struct.Quote = type { double, double }");
    expect(llvm).toContain("getelementptr double, ptr");
    expect(llvm).toContain("load double");
    expect(llvm).toContain("store double");
    expect(llvm).toContain("getelementptr %struct.Quote");
    expect(llvm).toContain("fadd double");
    expect(llvm).not.toContain("fast");
  });
});
