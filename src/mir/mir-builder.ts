import type { MirBlock, MirLocal, MirParam, MirPrimitiveTypeName, MirType, MirValue } from "./mir.js";

export function mirPrimitive(name: MirPrimitiveTypeName): MirType {
  return { kind: "primitive", name };
}

export function mirPointer(elementType: MirType): MirType {
  return { kind: "pointer", elementType };
}

export function mirStruct(name: string): MirType {
  return { kind: "struct", name };
}

export class MirBuilder {
  private tempCounter = 0;
  private blockCounter = 0;

  temp(type: MirType): MirValue {
    const name = `t${this.tempCounter}`;
    this.tempCounter += 1;
    return { kind: "temp", name, type };
  }

  param(name: string, type: MirType): MirParam {
    return { name, type };
  }

  paramValue(param: MirParam): MirValue {
    return { kind: "param", name: param.name, type: param.type };
  }

  local(name: string, type: MirType): MirLocal {
    return { name, type };
  }

  localValue(local: MirLocal): MirValue {
    return { kind: "local", name: local.name, type: local.type };
  }

  constInt(text: string, type: MirType): MirValue {
    return { kind: "const_int", text, type };
  }

  constFloat(text: string, type: MirType): MirValue {
    return { kind: "const_float", text, type };
  }

  constBool(value: boolean): MirValue {
    return { kind: "const_bool", value, type: mirPrimitive("bool") };
  }

  block(instructions: MirBlock["instructions"], terminator: MirBlock["terminator"], label = this.nextBlockLabel()): MirBlock {
    return { label, instructions, terminator };
  }

  nextBlockLabel(): string {
    const label = `bb${this.blockCounter}`;
    this.blockCounter += 1;
    return label;
  }
}
