import { describe, expect, it } from "vitest";
import {
  formatLlvmFieldGep,
  formatLlvmPointerIndexGep,
  getElementLlvmType,
  getIndexExtension
} from "../src/backend/llvm/llvm-gep.js";
import type { MirType } from "../src/mir/mir.js";

const i32: MirType = { kind: "primitive", name: "i32" };
const i64: MirType = { kind: "primitive", name: "i64" };
const u32: MirType = { kind: "primitive", name: "u32" };
const u64: MirType = { kind: "primitive", name: "u64" };
const item: MirType = { kind: "struct", name: "Item" };

describe("LLVM GEP helpers", () => {
  it("maps pointer element types to LLVM source element types", () => {
    expect(getElementLlvmType(i64)).toBe("i64");
    expect(getElementLlvmType(item)).toBe("%struct.Item");
  });

  it("formats pointer index and struct field GEP operations", () => {
    expect(formatLlvmPointerIndexGep(item, "%items", "%idx64")).toBe("getelementptr %struct.Item, ptr %items, i64 %idx64");
    expect(formatLlvmPointerIndexGep(i64, "%out", "%idx64")).toBe("getelementptr i64, ptr %out, i64 %idx64");
    expect(formatLlvmFieldGep("Item", "%item_ptr", 2)).toBe("getelementptr %struct.Item, ptr %item_ptr, i32 0, i32 2");
  });

  it("describes index extension to i64 for GEP indexing", () => {
    expect(getIndexExtension(i32)).toEqual({ kind: "sext", fromType: "i32", toType: "i64" });
    expect(getIndexExtension(u32)).toEqual({ kind: "zext", fromType: "i32", toType: "i64" });
    expect(getIndexExtension(i64)).toEqual({ kind: "none", type: "i64" });
    expect(getIndexExtension(u64)).toEqual({ kind: "none", type: "i64" });
  });

  it("rejects non-integer index types", () => {
    expect(() => getIndexExtension({ kind: "primitive", name: "bool" })).toThrow("LLVM GEP index must be an integer");
  });
});
