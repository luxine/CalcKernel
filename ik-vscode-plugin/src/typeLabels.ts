import type { FunctionInfo, IntKernelType } from "intkernel";

export type LabelSymbolKind = "struct" | "field" | "function" | "parameter" | "local" | "type";

export function formatTypeLabel(type: IntKernelType): string {
  switch (type.kind) {
    case "primitive":
      return type.name;
    case "pointer":
      return `ptr<${formatTypeLabel(type.elementType)}>`;
    case "struct":
      return type.name;
    case "integerLiteral":
      return "integer";
    case "unknown":
      return "unknown";
  }
}

export function formatFunctionSignature(info: FunctionInfo): string {
  const params = info.params.map((param) => `${param.name}: ${formatTypeLabel(param.type)}`).join(", ");
  return `fn ${info.name}(${params}) -> ${formatTypeLabel(info.returnType)}`;
}

export function formatSymbolLabel(kind: LabelSymbolKind, name: string, typeLabel?: string, containerName?: string): string {
  if (kind === "function") {
    return name;
  }
  if (kind === "struct") {
    return `struct ${name}`;
  }
  if (kind === "field") {
    return `field ${containerName ? `${containerName}.` : ""}${name}${typeLabel ? `: ${typeLabel}` : ""}`;
  }
  if (kind === "parameter") {
    return `parameter ${name}${typeLabel ? `: ${typeLabel}` : ""}`;
  }
  if (kind === "local") {
    return `local ${name}${typeLabel ? `: ${typeLabel}` : ""}`;
  }
  return `type ${name}`;
}
