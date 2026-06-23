import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { computeWasmStructLayout, sizeOfWasmType, alignOfWasmType } from "../src/backend/wasm/wasm-layout.js";
import { toWasmAbiType } from "../src/backend/wasm/wasm-types.js";
import { SourceFile } from "../src/source/source-file.js";
import { check, getStructInfo } from "../src/typeck/checker.js";
import { pointerType, primitiveType } from "../src/typeck/types.js";

function checkedProgram(fileName: string, sourceText: string) {
  const result = check(new SourceFile(fileName, sourceText));
  expect(result.diagnostics).toEqual([]);
  return result.checkedProgram;
}

describe("WASM layout", () => {
  it("maps IntKernel ABI types to WASM value types", () => {
    expect(toWasmAbiType(primitiveType("i32"))).toEqual({ valueType: "i32" });
    expect(toWasmAbiType(primitiveType("u32"))).toEqual({ valueType: "i32" });
    expect(toWasmAbiType(primitiveType("bool"))).toEqual({ valueType: "i32" });
    expect(toWasmAbiType(primitiveType("i64"))).toEqual({ valueType: "i64" });
    expect(toWasmAbiType(primitiveType("u64"))).toEqual({ valueType: "i64" });
    expect(toWasmAbiType(pointerType(primitiveType("i64")))).toEqual({ valueType: "i32" });
  });

  it("computes primitive and pointer sizes and alignments", () => {
    expect(sizeOfWasmType(primitiveType("i32"))).toBe(4);
    expect(alignOfWasmType(primitiveType("i32"))).toBe(4);
    expect(sizeOfWasmType(primitiveType("u32"))).toBe(4);
    expect(alignOfWasmType(primitiveType("u32"))).toBe(4);
    expect(sizeOfWasmType(primitiveType("bool"))).toBe(4);
    expect(alignOfWasmType(primitiveType("bool"))).toBe(4);
    expect(sizeOfWasmType(primitiveType("i64"))).toBe(8);
    expect(alignOfWasmType(primitiveType("i64"))).toBe(8);
    expect(sizeOfWasmType(primitiveType("u64"))).toBe(8);
    expect(alignOfWasmType(primitiveType("u64"))).toBe(8);
    expect(sizeOfWasmType(pointerType(primitiveType("i64")))).toBe(4);
    expect(alignOfWasmType(pointerType(primitiveType("i64")))).toBe(4);
  });

  it("computes the pricing Item layout from examples/pricing.ik", () => {
    const program = checkedProgram("pricing.ik", readFileSync("examples/pricing.ik", "utf8"));
    const item = getStructInfo(program, "Item");
    expect(item).toBeDefined();

    const layout = computeWasmStructLayout(item!);

    expect(layout).toEqual({
      name: "Item",
      size: 32,
      align: 8,
      fields: [
        { name: "price", type: primitiveType("i64"), offset: 0, size: 8, align: 8 },
        { name: "qty", type: primitiveType("i64"), offset: 8, size: 8, align: 8 },
        { name: "discount", type: primitiveType("i64"), offset: 16, size: 8, align: 8 },
        { name: "tax_rate_ppm", type: primitiveType("i64"), offset: 24, size: 8, align: 8 }
      ]
    });
  });

  it("adds padding for mixed-width struct fields", () => {
    const program = checkedProgram(
      "mixed.ik",
      `
        struct Mixed {
          a: i32;
          b: i64;
          c: bool;
          d: u32;
        }

        export fn ok() -> i32 {
          return 0;
        }
      `
    );
    const mixed = getStructInfo(program, "Mixed");
    expect(mixed).toBeDefined();

    const layout = computeWasmStructLayout(mixed!);

    expect(layout).toEqual({
      name: "Mixed",
      size: 24,
      align: 8,
      fields: [
        { name: "a", type: primitiveType("i32"), offset: 0, size: 4, align: 4 },
        { name: "b", type: primitiveType("i64"), offset: 8, size: 8, align: 8 },
        { name: "c", type: primitiveType("bool"), offset: 16, size: 4, align: 4 },
        { name: "d", type: primitiveType("u32"), offset: 20, size: 4, align: 4 }
      ]
    });
  });
});
