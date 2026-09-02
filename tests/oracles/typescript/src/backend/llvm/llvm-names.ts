export function normalizeLlvmName(name: string): string {
  const encoded = [...name].map(encodeLlvmNameChar).join("");
  const nonEmpty = encoded.length === 0 ? "ik_empty" : encoded;
  return /^[A-Za-z_]/.test(nonEmpty) ? nonEmpty : `ik_${nonEmpty}`;
}

export function llvmFunctionName(name: string): string {
  return `@${normalizeLlvmName(name)}`;
}

export function llvmLocalName(name: string): string {
  return `%${normalizeLlvmName(name)}`;
}

export function llvmBlockLabel(label: string): string {
  return normalizeLlvmName(label);
}

export function llvmStructName(name: string): string {
  return `%struct.${normalizeLlvmName(name)}`;
}

function encodeLlvmNameChar(char: string): string {
  if (/^[A-Za-z0-9_]$/.test(char)) {
    return char;
  }

  return `_x${char.codePointAt(0)!.toString(16)}_`;
}
