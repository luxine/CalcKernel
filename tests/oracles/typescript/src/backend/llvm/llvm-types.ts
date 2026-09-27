import type { MirType } from "../../mir/mir.js";
import type { CalcKernelType } from "../../typeck/types.js";
import { llvmStructName } from "./llvm-names.js";

export type LlvmType = string;
export type LlvmSourceType = CalcKernelType | MirType;

export function llvmValueType(type: LlvmSourceType): LlvmType {
  switch (type.kind) {
    case "primitive":
      switch (type.name) {
        case "i32":
        case "u32":
          return "i32";
        case "i64":
        case "u64":
          return "i64";
        case "f64":
          return "double";
        case "bool":
          return "i1";
      }
    case "pointer":
      return "ptr";
    case "struct":
      return llvmStructName(type.name);
    case "integerLiteral":
      throw new Error("Integer literal types must be materialized before LLVM type mapping.");
    case "unknown":
      throw new Error("Cannot map unknown type to an LLVM type.");
  }
}

export function llvmStorageType(type: LlvmSourceType): LlvmType {
  return llvmValueType(type);
}

export function llvmParamType(type: LlvmSourceType): LlvmType {
  return llvmValueType(type);
}

export function llvmReturnType(type: LlvmSourceType): LlvmType {
  return llvmValueType(type);
}

export function isSignedInteger(type: LlvmSourceType): boolean {
  return type.kind === "primitive" && (type.name === "i32" || type.name === "i64");
}

export function isUnsignedInteger(type: LlvmSourceType): boolean {
  return type.kind === "primitive" && (type.name === "u32" || type.name === "u64");
}

export function isIntegerLike(type: LlvmSourceType): boolean {
  return type.kind === "integerLiteral" || isSignedInteger(type) || isUnsignedInteger(type);
}
