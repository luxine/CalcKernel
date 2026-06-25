import { describe, expect, it } from "vitest";
import {
  canAssign,
  integerLiteralType,
  isBool,
  isFloatPrimitiveName,
  isFloatType,
  isIndexInteger,
  isInteger,
  isIntegerPrimitive,
  isIntegerPrimitiveName,
  isNumericType,
  materializeIntegerLiteral,
  pointerType,
  primitiveType,
  sameType
} from "../src/typeck/types.js";

describe("type helpers", () => {
  it("classifies integer primitive names explicitly", () => {
    expect(isIntegerPrimitiveName("i32")).toBe(true);
    expect(isIntegerPrimitiveName("i64")).toBe(true);
    expect(isIntegerPrimitiveName("u32")).toBe(true);
    expect(isIntegerPrimitiveName("u64")).toBe(true);
    expect(isIntegerPrimitiveName("f64")).toBe(false);
    expect(isIntegerPrimitiveName("bool")).toBe(false);
  });

  it("distinguishes integer primitives from literal and bool types", () => {
    expect(isIntegerPrimitive(primitiveType("i32"))).toBe(true);
    expect(isIntegerPrimitive(primitiveType("u64"))).toBe(true);
    expect(isIntegerPrimitive(primitiveType("f64"))).toBe(false);
    expect(isIntegerPrimitive(primitiveType("bool"))).toBe(false);
    expect(isIntegerPrimitive(integerLiteralType)).toBe(false);

    expect(isInteger(integerLiteralType)).toBe(true);
    expect(isInteger(primitiveType("i64"))).toBe(true);
    expect(isInteger(primitiveType("f64"))).toBe(false);
    expect(isInteger(primitiveType("bool"))).toBe(false);
  });

  it("keeps bool and index integer classification unchanged", () => {
    expect(isBool(primitiveType("bool"))).toBe(true);
    expect(isBool(primitiveType("i32"))).toBe(false);

    expect(isIndexInteger(integerLiteralType)).toBe(true);
    expect(isIndexInteger(primitiveType("i32"))).toBe(true);
    expect(isIndexInteger(primitiveType("u32"))).toBe(true);
    expect(isIndexInteger(primitiveType("i64"))).toBe(false);
    expect(isIndexInteger(primitiveType("u64"))).toBe(false);
    expect(isIndexInteger(primitiveType("f64"))).toBe(false);
    expect(isIndexInteger(primitiveType("bool"))).toBe(false);
  });

  it("classifies f64 as numeric but not integer", () => {
    expect(isNumericType(integerLiteralType)).toBe(true);
    expect(isNumericType(primitiveType("i32"))).toBe(true);
    expect(isNumericType(primitiveType("u64"))).toBe(true);
    expect(isNumericType(primitiveType("f64"))).toBe(true);
    expect(isNumericType(primitiveType("bool"))).toBe(false);
    expect(isNumericType(pointerType(primitiveType("i32")))).toBe(false);
  });

  it("classifies f64 as the only float primitive", () => {
    expect(isFloatPrimitiveName("i32")).toBe(false);
    expect(isFloatPrimitiveName("i64")).toBe(false);
    expect(isFloatPrimitiveName("u32")).toBe(false);
    expect(isFloatPrimitiveName("u64")).toBe(false);
    expect(isFloatPrimitiveName("f64")).toBe(true);
    expect(isFloatPrimitiveName("bool")).toBe(false);

    expect(isFloatType(primitiveType("i32"))).toBe(false);
    expect(isFloatType(primitiveType("f64"))).toBe(true);
    expect(isFloatType(integerLiteralType)).toBe(false);
    expect(isFloatType(pointerType(primitiveType("i32")))).toBe(false);
  });

  it("keeps integer literal assignment and materialization behavior unchanged", () => {
    expect(sameType(integerLiteralType, primitiveType("i64"))).toBe(true);
    expect(sameType(primitiveType("u32"), integerLiteralType)).toBe(true);
    expect(sameType(integerLiteralType, primitiveType("f64"))).toBe(false);
    expect(sameType(integerLiteralType, primitiveType("bool"))).toBe(false);

    expect(canAssign(primitiveType("i32"), integerLiteralType)).toBe(true);
    expect(canAssign(primitiveType("f64"), integerLiteralType)).toBe(false);
    expect(canAssign(primitiveType("bool"), integerLiteralType)).toBe(false);

    expect(materializeIntegerLiteral(integerLiteralType, primitiveType("i64"))).toEqual(primitiveType("i64"));
    expect(materializeIntegerLiteral(primitiveType("u64"), primitiveType("i32"))).toEqual(primitiveType("u64"));
  });
});
