import { describe, expect, it } from "vitest";
import {
  isIntegerLike,
  isSignedInteger,
  isUnsignedInteger,
  llvmParamType,
  llvmReturnType,
  llvmStorageType,
  llvmValueType
} from "../src/backend/llvm/llvm-types.js";
import { integerLiteralType, pointerType, primitiveType, structType, unknownType } from "../src/typeck/types.js";

describe("LLVM type mapping", () => {
  it("maps scalar IntKernel types to LLVM value types", () => {
    expect(llvmValueType(primitiveType("i32"))).toBe("i32");
    expect(llvmValueType(primitiveType("u32"))).toBe("i32");
    expect(llvmValueType(primitiveType("i64"))).toBe("i64");
    expect(llvmValueType(primitiveType("u64"))).toBe("i64");
    expect(llvmValueType(primitiveType("f64"))).toBe("double");
    expect(llvmValueType(primitiveType("bool"))).toBe("i1");
  });

  it("maps pointer and struct types", () => {
    expect(llvmValueType(pointerType(primitiveType("i64")))).toBe("ptr");
    expect(llvmValueType(pointerType(primitiveType("f64")))).toBe("ptr");
    expect(llvmValueType(pointerType(structType("Item")))).toBe("ptr");
    expect(llvmValueType(structType("Item"))).toBe("%struct.Item");
  });

  it("uses the current value mapping for storage, params, and returns", () => {
    expect(llvmStorageType(primitiveType("f64"))).toBe("double");
    expect(llvmParamType(primitiveType("f64"))).toBe("double");
    expect(llvmReturnType(primitiveType("f64"))).toBe("double");
    expect(llvmStorageType(primitiveType("bool"))).toBe("i1");
    expect(llvmParamType(pointerType(primitiveType("i32")))).toBe("ptr");
    expect(llvmReturnType(structType("Mixed"))).toBe("%struct.Mixed");
  });

  it("detects signed, unsigned, and integer-like types", () => {
    expect(isSignedInteger(primitiveType("i32"))).toBe(true);
    expect(isSignedInteger(primitiveType("i64"))).toBe(true);
    expect(isSignedInteger(primitiveType("u32"))).toBe(false);

    expect(isUnsignedInteger(primitiveType("u32"))).toBe(true);
    expect(isUnsignedInteger(primitiveType("u64"))).toBe(true);
    expect(isUnsignedInteger(primitiveType("i64"))).toBe(false);

    expect(isIntegerLike(primitiveType("i32"))).toBe(true);
    expect(isIntegerLike(primitiveType("u64"))).toBe(true);
    expect(isIntegerLike(integerLiteralType)).toBe(true);
    expect(isIntegerLike(primitiveType("f64"))).toBe(false);
    expect(isIntegerLike(primitiveType("bool"))).toBe(false);
  });

  it("rejects unresolved checker-only types during LLVM mapping", () => {
    expect(() => llvmValueType(integerLiteralType)).toThrow("Integer literal types must be materialized");
    expect(() => llvmValueType(unknownType)).toThrow("Cannot map unknown type");
  });
});
