export type PrimitiveTypeName = "i32" | "i64" | "u32" | "u64" | "f64" | "bool";

export type IntKernelType =
  | { kind: "primitive"; name: PrimitiveTypeName }
  | { kind: "pointer"; elementType: IntKernelType }
  | { kind: "struct"; name: string }
  | { kind: "integerLiteral" }
  | { kind: "unknown" };

export const unknownType: IntKernelType = { kind: "unknown" };
export const integerLiteralType: IntKernelType = { kind: "integerLiteral" };

const integerPrimitiveNames: readonly PrimitiveTypeName[] = ["i32", "i64", "u32", "u64"];
const indexIntegerPrimitiveNames: readonly PrimitiveTypeName[] = ["i32", "u32"];
const floatPrimitiveNames: readonly PrimitiveTypeName[] = ["f64"];

export function primitiveType(name: PrimitiveTypeName): IntKernelType {
  return { kind: "primitive", name };
}

export function pointerType(elementType: IntKernelType): IntKernelType {
  return { kind: "pointer", elementType };
}

export function structType(name: string): IntKernelType {
  return { kind: "struct", name };
}

export function isUnknown(type: IntKernelType): boolean {
  return type.kind === "unknown";
}

export function isBool(type: IntKernelType): boolean {
  return type.kind === "primitive" && type.name === "bool";
}

export function isIntegerPrimitiveName(name: PrimitiveTypeName): boolean {
  return integerPrimitiveNames.includes(name);
}

export function isFloatPrimitiveName(name: PrimitiveTypeName): boolean {
  return floatPrimitiveNames.includes(name);
}

export function isIntegerPrimitive(type: IntKernelType): boolean {
  return type.kind === "primitive" && isIntegerPrimitiveName(type.name);
}

export function isFloatType(type: IntKernelType): boolean {
  return type.kind === "primitive" && isFloatPrimitiveName(type.name);
}

export function isInteger(type: IntKernelType): boolean {
  return type.kind === "integerLiteral" || isIntegerPrimitive(type);
}

export function isNumericType(type: IntKernelType): boolean {
  return isInteger(type) || isFloatType(type);
}

export function isIndexInteger(type: IntKernelType): boolean {
  return type.kind === "integerLiteral" || (type.kind === "primitive" && indexIntegerPrimitiveNames.includes(type.name));
}

export function sameType(left: IntKernelType, right: IntKernelType): boolean {
  if (left.kind === "unknown" || right.kind === "unknown") {
    return true;
  }

  if (left.kind === "integerLiteral" && isInteger(right)) {
    return true;
  }

  if (right.kind === "integerLiteral" && isInteger(left)) {
    return true;
  }

  if (left.kind !== right.kind) {
    return false;
  }

  switch (left.kind) {
    case "primitive":
      return right.kind === "primitive" && left.name === right.name;
    case "pointer":
      return right.kind === "pointer" && sameType(left.elementType, right.elementType);
    case "struct":
      return right.kind === "struct" && left.name === right.name;
    case "integerLiteral":
      return true;
  }
}

export function canAssign(target: IntKernelType, value: IntKernelType): boolean {
  return sameType(target, value);
}

export function materializeIntegerLiteral(type: IntKernelType, fallback: IntKernelType = primitiveType("i32")): IntKernelType {
  return type.kind === "integerLiteral" ? fallback : type;
}

export function typeToString(type: IntKernelType): string {
  switch (type.kind) {
    case "primitive":
      return type.name;
    case "pointer":
      return `ptr<${typeToString(type.elementType)}>`;
    case "struct":
      return type.name;
    case "integerLiteral":
      return "i32";
    case "unknown":
      return "unknown";
  }
}
