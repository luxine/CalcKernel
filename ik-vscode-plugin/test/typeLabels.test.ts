import { SourceFile, check, getFunctionInfo, getStructInfo } from "intkernel";
import { describe, expect, it } from "vitest";
import { formatFunctionSignature, formatSymbolLabel, formatTypeLabel } from "../src/typeLabels";

const sourceText = `
struct Item {
  price: i64;
}

fn add_tax(price: i64, tax: i64) -> i64 {
  return price + tax;
}
`.trimStart();

describe("typeLabels", () => {
  const checked = check(new SourceFile("sample.ik", sourceText)).checkedProgram;

  it("formats primitive, pointer, and struct types", () => {
    expect(formatTypeLabel({ kind: "primitive", name: "i64" })).toBe("i64");
    expect(formatTypeLabel({ kind: "struct", name: "Item" })).toBe("Item");
    expect(formatTypeLabel({ kind: "pointer", elementType: { kind: "struct", name: "Item" } })).toBe("ptr<Item>");
  });

  it("formats function signatures from checked program info", () => {
    const info = getFunctionInfo(checked, "add_tax");
    expect(info && formatFunctionSignature(info)).toBe("fn add_tax(price: i64, tax: i64) -> i64");
  });

  it("formats symbol labels", () => {
    const structInfo = getStructInfo(checked, "Item");
    expect(structInfo && formatSymbolLabel("struct", "Item", undefined, structInfo.name)).toBe("struct Item");
    expect(formatSymbolLabel("field", "price", "i64", "Item")).toBe("field Item.price: i64");
    expect(formatSymbolLabel("function", "fn add_tax(price: i64, tax: i64) -> i64")).toBe(
      "fn add_tax(price: i64, tax: i64) -> i64"
    );
    expect(formatSymbolLabel("parameter", "price", "i64")).toBe("parameter price: i64");
    expect(formatSymbolLabel("local", "total", "i64")).toBe("local total: i64");
    expect(formatSymbolLabel("type", "Item")).toBe("type Item");
  });
});
