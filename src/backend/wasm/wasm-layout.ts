import type { StructInfo } from "../../typeck/checker.js";
import type { IntKernelType } from "../../typeck/types.js";

export interface WasmStructFieldLayout {
  name: string;
  type: IntKernelType;
  offset: number;
  size: number;
  align: number;
}

export interface WasmStructLayout {
  name: string;
  size: number;
  align: number;
  fields: WasmStructFieldLayout[];
}

export interface WasmLayoutContext {
  structs?: ReadonlyMap<string, WasmStructLayout>;
}

export function sizeOfWasmType(type: IntKernelType, context: WasmLayoutContext = {}): number {
  switch (type.kind) {
    case "primitive":
      switch (type.name) {
        case "i32":
        case "u32":
        case "bool":
          return 4;
        case "i64":
        case "u64":
        case "f64":
          return 8;
      }
    case "pointer":
      return 4;
    case "struct":
      return requireStructLayout(type.name, context).size;
    case "integerLiteral":
      throw new Error("Integer literal types must be materialized before WASM layout calculation.");
    case "unknown":
      throw new Error("Cannot calculate WASM layout for unknown type.");
  }
}

export function alignOfWasmType(type: IntKernelType, context: WasmLayoutContext = {}): number {
  switch (type.kind) {
    case "primitive":
      switch (type.name) {
        case "i32":
        case "u32":
        case "bool":
          return 4;
        case "i64":
        case "u64":
        case "f64":
          return 8;
      }
    case "pointer":
      return 4;
    case "struct":
      return requireStructLayout(type.name, context).align;
    case "integerLiteral":
      throw new Error("Integer literal types must be materialized before WASM layout calculation.");
    case "unknown":
      throw new Error("Cannot calculate WASM layout for unknown type.");
  }
}

export function computeWasmStructLayout(structInfo: StructInfo, context: WasmLayoutContext = {}): WasmStructLayout {
  let offset = 0;
  let structAlign = 1;
  const fields: WasmStructFieldLayout[] = [];

  for (const field of structInfo.fields) {
    const fieldAlign = alignOfWasmType(field.type, context);
    const fieldSize = sizeOfWasmType(field.type, context);
    offset = alignUp(offset, fieldAlign);
    structAlign = Math.max(structAlign, fieldAlign);

    fields.push({
      name: field.name,
      type: field.type,
      offset,
      size: fieldSize,
      align: fieldAlign
    });

    offset += fieldSize;
  }

  return {
    name: structInfo.name,
    size: alignUp(offset, structAlign),
    align: structAlign,
    fields
  };
}

export function computeWasmStructLayouts(structs: readonly StructInfo[]): Map<string, WasmStructLayout> {
  const layouts = new Map<string, WasmStructLayout>();
  const pending = new Set(structs);

  while (pending.size > 0) {
    let madeProgress = false;

    for (const structInfo of pending) {
      try {
        const layout = computeWasmStructLayout(structInfo, { structs: layouts });
        layouts.set(structInfo.name, layout);
        pending.delete(structInfo);
        madeProgress = true;
      } catch (error) {
        if (!(error instanceof MissingStructLayoutError)) {
          throw error;
        }
      }
    }

    if (!madeProgress) {
      const names = [...pending].map((structInfo) => structInfo.name).join(", ");
      throw new Error(`Cannot calculate WASM struct layouts; unresolved struct field layout for: ${names}`);
    }
  }

  return layouts;
}

function alignUp(value: number, alignment: number): number {
  return Math.ceil(value / alignment) * alignment;
}

function requireStructLayout(name: string, context: WasmLayoutContext): WasmStructLayout {
  const layout = context.structs?.get(name);
  if (!layout) {
    throw new MissingStructLayoutError(name);
  }

  return layout;
}

class MissingStructLayoutError extends Error {
  constructor(readonly structName: string) {
    super(`Missing WASM layout for struct '${structName}'.`);
  }
}
