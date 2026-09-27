import type { MirStruct, MirType } from "../../mir/mir.js";
import { llvmStructName } from "./llvm-names.js";
import { llvmStorageType } from "./llvm-types.js";

export interface LlvmStructFieldLayout {
  name: string;
  type: MirType;
  llvmType: string;
  index: number;
}

export interface LlvmStructLayout {
  name: string;
  llvmType: string;
  fields: LlvmStructFieldLayout[];
}

export interface LlvmLayout {
  structs: LlvmStructLayout[];
  structsByName: Map<string, LlvmStructLayout>;
}

export function createLlvmLayout(structs: MirStruct[]): LlvmLayout {
  const layouts = structs.map(createLlvmStructLayout);
  return {
    structs: layouts,
    structsByName: new Map(layouts.map((layout) => [layout.name, layout]))
  };
}

export function createLlvmStructLayout(structInfo: MirStruct): LlvmStructLayout {
  return {
    name: structInfo.name,
    llvmType: llvmStructName(structInfo.name),
    fields: structInfo.fields.map((field, index) => ({
      name: field.name,
      type: field.type,
      llvmType: llvmStorageType(field.type),
      index
    }))
  };
}

export function getStructLlvmType(layout: LlvmLayout, structName: string): string {
  return getStructLayout(layout, structName).llvmType;
}

export function getStructFieldIndex(layout: LlvmLayout, structName: string, fieldName: string): number {
  return getStructField(layout, structName, fieldName).index;
}

export function getStructFieldType(layout: LlvmLayout, structName: string, fieldName: string): MirType {
  return getStructField(layout, structName, fieldName).type;
}

export function emitLlvmStructDeclaration(layout: LlvmStructLayout): string {
  const fields = layout.fields.map((field) => field.llvmType).join(", ");
  return `${layout.llvmType} = type { ${fields} }`;
}

function getStructLayout(layout: LlvmLayout, structName: string): LlvmStructLayout {
  const structLayout = layout.structsByName.get(structName);
  if (structLayout === undefined) {
    throw new Error(`Unknown LLVM struct ${structName}.`);
  }

  return structLayout;
}

function getStructField(layout: LlvmLayout, structName: string, fieldName: string): LlvmStructFieldLayout {
  const structLayout = getStructLayout(layout, structName);
  const field = structLayout.fields.find((candidate) => candidate.name === fieldName);
  if (field === undefined) {
    throw new Error(`Unknown field ${structName}.${fieldName}.`);
  }

  return field;
}
