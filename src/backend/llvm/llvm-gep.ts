import type { MirType } from "../../mir/mir.js";
import { llvmStructName } from "./llvm-names.js";
import { llvmStorageType } from "./llvm-types.js";

export type LlvmIndexExtension =
  | { kind: "sext"; fromType: "i32"; toType: "i64" }
  | { kind: "zext"; fromType: "i32"; toType: "i64" }
  | { kind: "none"; type: "i64" };

export function getElementLlvmType(ptrElementType: MirType): string {
  return llvmStorageType(ptrElementType);
}

// ptr<T>[i] lowers to: getelementptr T, ptr base, i64 index.
export function formatLlvmPointerIndexGep(elementType: MirType, basePointer: string, indexValue: string): string {
  return `getelementptr ${getElementLlvmType(elementType)}, ptr ${basePointer}, i64 ${indexValue}`;
}

// field access lowers to: getelementptr %struct.Name, ptr base, i32 0, i32 fieldIndex.
export function formatLlvmFieldGep(structName: string, basePointer: string, fieldIndex: number): string {
  return `getelementptr ${llvmStructName(structName)}, ptr ${basePointer}, i32 0, i32 ${fieldIndex}`;
}

export function getIndexExtension(indexType: MirType): LlvmIndexExtension {
  if (indexType.kind !== "primitive") {
    throw new Error("LLVM GEP index must be an integer.");
  }

  switch (indexType.name) {
    case "i32":
      return { kind: "sext", fromType: "i32", toType: "i64" };
    case "u32":
      return { kind: "zext", fromType: "i32", toType: "i64" };
    case "i64":
    case "u64":
      return { kind: "none", type: "i64" };
    case "bool":
      throw new Error("LLVM GEP index must be an integer.");
  }
}
