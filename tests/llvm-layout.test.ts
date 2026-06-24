import { describe, expect, it } from "vitest";
import {
  createLlvmLayout,
  emitLlvmStructDeclaration,
  getStructFieldIndex,
  getStructLlvmType
} from "../src/backend/llvm/llvm-layout.js";
import type { MirStruct, MirType } from "../src/mir/mir.js";

const i32: MirType = { kind: "primitive", name: "i32" };
const i64: MirType = { kind: "primitive", name: "i64" };
const u32: MirType = { kind: "primitive", name: "u32" };
const bool: MirType = { kind: "primitive", name: "bool" };

describe("LLVM struct layout helpers", () => {
  it("builds a field index map and declaration for Item", () => {
    const item: MirStruct = {
      name: "Item",
      fields: [
        { name: "price", type: i64 },
        { name: "qty", type: i64 },
        { name: "discount", type: i64 },
        { name: "tax_rate_ppm", type: i64 }
      ]
    };
    const layout = createLlvmLayout([item]);

    expect(getStructLlvmType(layout, "Item")).toBe("%struct.Item");
    expect(getStructFieldIndex(layout, "Item", "price")).toBe(0);
    expect(getStructFieldIndex(layout, "Item", "qty")).toBe(1);
    expect(getStructFieldIndex(layout, "Item", "discount")).toBe(2);
    expect(getStructFieldIndex(layout, "Item", "tax_rate_ppm")).toBe(3);
    expect(emitLlvmStructDeclaration(layout.structs[0])).toBe("%struct.Item = type { i64, i64, i64, i64 }");
  });

  it("emits mixed struct declarations using LLVM storage types", () => {
    const mixed: MirStruct = {
      name: "Mixed",
      fields: [
        { name: "a", type: i32 },
        { name: "b", type: i64 },
        { name: "c", type: bool },
        { name: "d", type: u32 }
      ]
    };
    const layout = createLlvmLayout([mixed]);

    expect(emitLlvmStructDeclaration(layout.structs[0])).toBe("%struct.Mixed = type { i32, i64, i1, i32 }");
    expect(getStructFieldIndex(layout, "Mixed", "a")).toBe(0);
    expect(getStructFieldIndex(layout, "Mixed", "b")).toBe(1);
    expect(getStructFieldIndex(layout, "Mixed", "c")).toBe(2);
    expect(getStructFieldIndex(layout, "Mixed", "d")).toBe(3);
  });

  it("reports unknown structs and fields clearly", () => {
    const layout = createLlvmLayout([{ name: "Item", fields: [{ name: "price", type: i64 }] }]);

    expect(() => getStructLlvmType(layout, "Missing")).toThrow("Unknown LLVM struct Missing");
    expect(() => getStructFieldIndex(layout, "Item", "qty")).toThrow("Unknown field Item.qty");
  });
});
