import type { CalcKernelType } from "../../typeck/types.js";

export type WasmValueType = "i32" | "i64" | "f64";

export interface WasmAbiType {
  valueType: WasmValueType;
}

export function toWasmAbiType(type: CalcKernelType): WasmAbiType {
  return { valueType: toWasmValueType(type) };
}

export function toWasmValueType(type: CalcKernelType): WasmValueType {
  switch (type.kind) {
    case "primitive":
      switch (type.name) {
        case "i32":
        case "u32":
        case "bool":
          return "i32";
        case "i64":
        case "u64":
          return "i64";
        case "f64":
          return "f64";
      }
    case "pointer":
      return "i32";
    case "struct":
      throw new Error(`Struct type '${type.name}' is a memory layout type and does not map directly to a WASM value type.`);
    case "integerLiteral":
      throw new Error("Integer literal types must be materialized before WASM ABI mapping.");
    case "unknown":
      throw new Error("Cannot map unknown type to a WASM value type.");
  }
}
