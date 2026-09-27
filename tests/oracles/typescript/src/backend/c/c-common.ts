export type CPrimitiveTypeName = "i32" | "i64" | "u32" | "u64" | "f64" | "bool";

export function emitCPrimitiveType(name: CPrimitiveTypeName): string {
  switch (name) {
    case "i32":
      return "int32_t";
    case "i64":
      return "int64_t";
    case "u32":
      return "uint32_t";
    case "u64":
      return "uint64_t";
    case "f64":
      return "double";
    case "bool":
      return "bool";
  }
}

export function escapeCIncludePath(path: string): string {
  return path.replaceAll("\\", "\\\\").replaceAll('"', '\\"');
}
