import { describe, expect, it } from "vitest";
import {
  llvmBlockLabel,
  llvmFunctionName,
  llvmLocalName,
  llvmStructName,
  normalizeLlvmName
} from "../src/backend/llvm/llvm-names.js";

describe("LLVM name utilities", () => {
  it("formats function, local, block, and struct names", () => {
    expect(llvmFunctionName("calc_items")).toBe("@calc_items");
    expect(llvmLocalName("ik_tmp0")).toBe("%ik_tmp0");
    expect(llvmBlockLabel("bb0")).toBe("bb0");
    expect(llvmStructName("Item")).toBe("%struct.Item");
  });

  it("normalizes unsafe characters deterministically", () => {
    expect(normalizeLlvmName("items[i].price")).toBe("items_x5b_i_x5d__x2e_price");
    expect(llvmFunctionName("add-i64")).toBe("@add_x2d_i64");
    expect(llvmLocalName("arg 0")).toBe("%arg_x20_0");
    expect(llvmStructName("Order.Item")).toBe("%struct.Order_x2e_Item");
  });

  it("handles empty names with stable fallbacks", () => {
    expect(normalizeLlvmName("")).toBe("ik_empty");
    expect(llvmFunctionName("")).toBe("@ik_empty");
    expect(llvmLocalName("")).toBe("%ik_empty");
    expect(llvmBlockLabel("")).toBe("ik_empty");
    expect(llvmStructName("")).toBe("%struct.ik_empty");
  });

  it("prefixes names that would start with a digit", () => {
    expect(normalizeLlvmName("123abc")).toBe("ik_123abc");
    expect(llvmFunctionName("123abc")).toBe("@ik_123abc");
  });
});
