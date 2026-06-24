export type MirPrimitiveTypeName = "i32" | "i64" | "u32" | "u64" | "bool";

export type MirType =
  | { kind: "primitive"; name: MirPrimitiveTypeName }
  | { kind: "pointer"; elementType: MirType }
  | { kind: "struct"; name: string };

export interface MirModule {
  structs: MirStruct[];
  functions: MirFunction[];
}

export interface MirStruct {
  name: string;
  fields: MirStructField[];
}

export interface MirStructField {
  name: string;
  type: MirType;
}

export interface MirFunction {
  name: string;
  exported: boolean;
  params: MirParam[];
  returnType: MirType;
  locals: MirLocal[];
  blocks: MirBlock[];
}

export interface MirParam {
  name: string;
  type: MirType;
}

export interface MirLocal {
  name: string;
  type: MirType;
}

export interface MirBlock {
  label: string;
  instructions: MirInstruction[];
  terminator: MirTerminator;
}

export type MirValue =
  | { kind: "param"; name: string; type: MirType }
  | { kind: "local"; name: string; type: MirType }
  | { kind: "temp"; name: string; type: MirType }
  | { kind: "const_int"; text: string; type: MirType }
  | { kind: "const_bool"; value: boolean; type: MirType };

export type MirPlace =
  | { kind: "param"; name: string; type: MirType }
  | { kind: "local"; name: string; type: MirType }
  | { kind: "deref"; pointer: MirValue; type: MirType }
  | { kind: "index"; base: MirPlace; index: MirValue; type: MirType }
  | { kind: "field"; base: MirPlace; fieldName: string; type: MirType };

export type MirBinaryOp = "+" | "-" | "*" | "/" | "%";
export type MirCompareOp = "==" | "!=" | "<" | "<=" | ">" | ">=";
export type MirUnaryOp = "neg" | "not";

export type MirInstruction =
  | MirConstIntInstruction
  | MirConstBoolInstruction
  | MirMoveInstruction
  | MirBinaryInstruction
  | MirUnaryInstruction
  | MirCompareInstruction
  | MirAddressInstruction
  | MirLoadInstruction
  | MirStoreInstruction
  | MirCallInstruction;

export interface MirConstIntInstruction {
  kind: "const_int";
  target: MirValue;
  value: string;
}

export interface MirConstBoolInstruction {
  kind: "const_bool";
  target: MirValue;
  value: boolean;
}

export interface MirMoveInstruction {
  kind: "move";
  target: MirValue;
  value: MirValue;
}

export interface MirBinaryInstruction {
  kind: "binary";
  target: MirValue;
  op: MirBinaryOp;
  left: MirValue;
  right: MirValue;
}

export interface MirUnaryInstruction {
  kind: "unary";
  target: MirValue;
  op: MirUnaryOp;
  operand: MirValue;
}

export interface MirCompareInstruction {
  kind: "compare";
  target: MirValue;
  op: MirCompareOp;
  left: MirValue;
  right: MirValue;
}

export interface MirAddressInstruction {
  kind: "address";
  target: MirValue;
  place: MirPlace;
}

export interface MirLoadInstruction {
  kind: "load";
  target: MirValue;
  place: MirPlace;
}

export interface MirStoreInstruction {
  kind: "store";
  place: MirPlace;
  value: MirValue;
}

export interface MirCallInstruction {
  kind: "call";
  target: MirValue;
  functionName: string;
  args: MirValue[];
}

export type MirTerminator =
  | { kind: "return"; value: MirValue }
  | { kind: "jump"; label: string }
  | { kind: "branch"; condition: MirValue; thenLabel: string; elseLabel: string };
