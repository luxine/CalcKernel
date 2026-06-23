export function toWasmIdentifier(name: string): string {
  const encoded = [...name].map(encodeIdentifierChar).join("");
  return `$${encoded.length === 0 ? "ik_empty" : encoded}`;
}

export function escapeWatString(value: string): string {
  return [...value].map(escapeWatStringChar).join("");
}

function encodeIdentifierChar(char: string): string {
  if (/^[A-Za-z0-9_]$/.test(char)) {
    return char;
  }

  return `_x${char.codePointAt(0)!.toString(16)}_`;
}

function escapeWatStringChar(char: string): string {
  switch (char) {
    case "\"":
      return "\\22";
    case "\\":
      return "\\5c";
    case "\n":
      return "\\0a";
    case "\r":
      return "\\0d";
    case "\t":
      return "\\09";
    default:
      return char;
  }
}
