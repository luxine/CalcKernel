import type {
  MirBinaryOp,
  MirBlock,
  MirCompareOp,
  MirFunction,
  MirInstruction,
  MirModule,
  MirPlace,
  MirStruct,
  MirTerminator,
  MirType,
  MirUnaryOp,
  MirValue
} from "../../mir/mir.js";
import type { WasmValueType } from "./wasm-types.js";
import { emitWatModule, type WatFunction, type WatLocal } from "./wat-emitter.js";
import { toWasmIdentifier } from "./wasm-names.js";

export function emitMirWatModule(module: MirModule): string {
  const context = createMirWasmContext(module.structs);
  return emitWatModule({
    functions: module.functions.map((func) => emitMirWatFunction(func, context))
  });
}

interface MirWasmContext {
  structs: Map<string, MirStructLayout>;
}

interface MirStructLayout {
  name: string;
  size: number;
  align: number;
  fields: Map<string, MirStructFieldLayout>;
}

interface MirStructFieldLayout {
  name: string;
  type: MirType;
  offset: number;
  size: number;
  align: number;
}

function emitMirWatFunction(func: MirFunction, context: MirWasmContext): WatFunction {
  const dispatcher = func.blocks.length > 1 ? createDispatcherContext(func) : null;
  const locals = collectWatLocals(func, dispatcher);
  const body = emitFunctionBody(func, dispatcher, context);

  return {
    name: func.name,
    exportName: func.exported ? func.name : undefined,
    params: func.params.map((param) => ({ name: param.name, type: mirTypeToWasmValueType(param.type) })),
    result: mirTypeToWasmValueType(func.returnType),
    locals,
    body
  };
}

interface DispatcherContext {
  blockIndexes: Map<string, number>;
  bbLocalName: string;
  returnLocalName: string;
  exitLabel: string;
  dispatchLabel: string;
  caseLabels: string[];
}

function emitFunctionBody(func: MirFunction, dispatcher: DispatcherContext | null, context: MirWasmContext): string[] {
  if (func.blocks.length === 1) {
    return emitSingleReturnBlock(func, func.blocks[0]!, context);
  }

  if (!dispatcher) {
    throw new Error(`Cannot emit WAT for function '${func.name}': missing dispatcher context.`);
  }

  return emitDispatchedFunction(func, dispatcher, context);
}

function emitSingleReturnBlock(func: MirFunction, block: MirBlock, context: MirWasmContext): string[] {
  if (block.terminator.kind !== "return") {
    throw new Error(`Cannot emit WAT for function '${func.name}': scalar WAT backend supports only return terminators.`);
  }

  const body: string[] = [];
  emitBlockInstructions(body, block, context);
  emitTerminator(body, block.terminator);
  return body;
}

function emitBlockInstructions(body: string[], block: MirBlock, context: MirWasmContext): void {
  for (const instruction of block.instructions) {
    emitInstruction(body, instruction, context);
  }
}

function collectWatLocals(func: MirFunction, dispatcher: DispatcherContext | null): WatLocal[] {
  const locals: WatLocal[] = func.locals.map((local) => ({ name: local.name, type: mirTypeToWasmValueType(local.type) }));
  const seen = new Set(locals.map((local) => local.name));

  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target?.kind === "temp" && !seen.has(target.name)) {
        locals.push({ name: target.name, type: mirTypeToWasmValueType(target.type) });
        seen.add(target.name);
      }
    }
  }

  if (dispatcher) {
    locals.push({ name: dispatcher.bbLocalName, type: "i32" });
    locals.push({ name: dispatcher.returnLocalName, type: mirTypeToWasmValueType(func.returnType) });
  }

  return locals;
}

function createDispatcherContext(func: MirFunction): DispatcherContext {
  const usedNames = new Set<string>();
  for (const param of func.params) {
    usedNames.add(param.name);
  }
  for (const local of func.locals) {
    usedNames.add(local.name);
  }
  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target?.kind === "temp") {
        usedNames.add(target.name);
      }
    }
  }

  const blockIndexes = new Map<string, number>();
  func.blocks.forEach((block, index) => {
    blockIndexes.set(block.label, index);
  });

  return {
    blockIndexes,
    bbLocalName: uniqueInternalName("ik_bb", usedNames),
    returnLocalName: uniqueInternalName("ik_ret", usedNames),
    exitLabel: "ik_exit",
    dispatchLabel: "ik_dispatch",
    caseLabels: func.blocks.map((_, index) => `ik_case${index}`)
  };
}

function uniqueInternalName(baseName: string, usedNames: Set<string>): string {
  if (!usedNames.has(baseName)) {
    usedNames.add(baseName);
    return baseName;
  }

  for (let index = 0; ; index++) {
    const candidate = `${baseName}${index}`;
    if (!usedNames.has(candidate)) {
      usedNames.add(candidate);
      return candidate;
    }
  }
}

function emitDispatchedFunction(func: MirFunction, dispatcher: DispatcherContext, context: MirWasmContext): string[] {
  const body: string[] = [];
  const defaultLabel = dispatcher.caseLabels[0]!;

  body.push("i32.const 0");
  body.push(`local.set ${toWasmIdentifier(dispatcher.bbLocalName)}`);
  body.push(`block ${toWasmIdentifier(dispatcher.exitLabel)}`);
  body.push(`  loop ${toWasmIdentifier(dispatcher.dispatchLabel)}`);

  for (let index = 0; index < dispatcher.caseLabels.length; index++) {
    body.push(`${indent(2 + index)}block ${toWasmIdentifier(dispatcher.caseLabels[index]!)}`);
  }

  body.push(`${indent(2 + dispatcher.caseLabels.length)}local.get ${toWasmIdentifier(dispatcher.bbLocalName)}`);
  body.push(
    `${indent(2 + dispatcher.caseLabels.length)}br_table ${dispatcher.caseLabels
      .map((label) => toWasmIdentifier(label))
      .join(" ")} ${toWasmIdentifier(defaultLabel)}`
  );

  for (let index = func.blocks.length - 1; index >= 0; index--) {
    body.push(`${indent(2 + index)}end`);
    emitDispatchedBlock(body, func.blocks[index]!, dispatcher, 2 + index, context);
  }

  body.push("  end");
  body.push("end");
  body.push(`local.get ${toWasmIdentifier(dispatcher.returnLocalName)}`);
  return body;
}

function emitDispatchedBlock(body: string[], block: MirBlock, dispatcher: DispatcherContext, depth: number, context: MirWasmContext): void {
  const blockLines: string[] = [];
  emitBlockInstructions(blockLines, block, context);
  emitDispatchedTerminator(blockLines, block.terminator, dispatcher);

  for (const line of blockLines) {
    body.push(`${indent(depth)}${line}`);
  }
}

function emitDispatchedTerminator(body: string[], terminator: MirTerminator, dispatcher: DispatcherContext): void {
  switch (terminator.kind) {
    case "return":
      emitValue(body, terminator.value);
      body.push(`local.set ${toWasmIdentifier(dispatcher.returnLocalName)}`);
      body.push(`br ${toWasmIdentifier(dispatcher.exitLabel)}`);
      return;
    case "jump":
      emitBlockJump(body, terminator.label, dispatcher);
      return;
    case "branch":
      emitValue(body, terminator.condition);
      body.push("if");
      body.push(`  i32.const ${blockIndexFor(terminator.thenLabel, dispatcher)}`);
      body.push(`  local.set ${toWasmIdentifier(dispatcher.bbLocalName)}`);
      body.push("else");
      body.push(`  i32.const ${blockIndexFor(terminator.elseLabel, dispatcher)}`);
      body.push(`  local.set ${toWasmIdentifier(dispatcher.bbLocalName)}`);
      body.push("end");
      body.push(`br ${toWasmIdentifier(dispatcher.dispatchLabel)}`);
      return;
  }
}

function emitBlockJump(body: string[], label: string, dispatcher: DispatcherContext): void {
  body.push(`i32.const ${blockIndexFor(label, dispatcher)}`);
  body.push(`local.set ${toWasmIdentifier(dispatcher.bbLocalName)}`);
  body.push(`br ${toWasmIdentifier(dispatcher.dispatchLabel)}`);
}

function blockIndexFor(label: string, dispatcher: DispatcherContext): number {
  const index = dispatcher.blockIndexes.get(label);
  if (index === undefined) {
    throw new Error(`Cannot emit WAT for MIR branch target '${label}': unknown block.`);
  }

  return index;
}

function indent(level: number): string {
  return "  ".repeat(level);
}

function instructionTarget(instruction: MirInstruction): MirValue | null {
  switch (instruction.kind) {
    case "const_int":
    case "const_bool":
    case "move":
    case "binary":
    case "unary":
    case "compare":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return null;
  }
}

function emitInstruction(body: string[], instruction: MirInstruction, context: MirWasmContext): void {
  switch (instruction.kind) {
    case "const_int":
      body.push(`${mirTypeToWasmValueType(instruction.target.type)}.const ${instruction.value}`);
      emitSet(body, instruction.target);
      return;
    case "const_bool":
      body.push(`i32.const ${instruction.value ? "1" : "0"}`);
      emitSet(body, instruction.target);
      return;
    case "move":
      emitValue(body, instruction.value);
      emitSet(body, instruction.target);
      return;
    case "binary":
      emitValue(body, instruction.left);
      emitValue(body, instruction.right);
      body.push(binaryWatInstruction(instruction.op, instruction.left.type));
      emitSet(body, instruction.target);
      return;
    case "unary":
      emitUnaryInstruction(body, instruction.op, instruction.operand, instruction.target);
      return;
    case "compare":
      emitValue(body, instruction.left);
      emitValue(body, instruction.right);
      body.push(compareWatInstruction(instruction.op, instruction.left.type));
      emitSet(body, instruction.target);
      return;
    case "load":
      emitAddress(body, instruction.place, context);
      body.push(`${mirTypeToWasmValueType(instruction.target.type)}.load offset=0 align=${alignOfMirWasmType(instruction.target.type, context)}`);
      emitSet(body, instruction.target);
      return;
    case "store":
      emitAddress(body, instruction.place, context);
      emitValue(body, instruction.value);
      body.push(`${mirTypeToWasmValueType(instruction.value.type)}.store offset=0 align=${alignOfMirWasmType(instruction.value.type, context)}`);
      return;
    case "call":
      for (const arg of instruction.args) {
        emitValue(body, arg);
      }
      body.push(`call ${toWasmIdentifier(instruction.functionName)}`);
      emitSet(body, instruction.target);
      return;
  }
}

function emitUnaryInstruction(body: string[], op: MirUnaryOp, operand: MirValue, target: MirValue): void {
  switch (op) {
    case "neg":
      body.push(`${mirTypeToWasmValueType(operand.type)}.const 0`);
      emitValue(body, operand);
      body.push(`${mirTypeToWasmValueType(operand.type)}.sub`);
      emitSet(body, target);
      return;
    case "not":
      emitValue(body, operand);
      body.push("i32.eqz");
      emitSet(body, target);
      return;
  }
}

function emitTerminator(body: string[], terminator: MirTerminator): void {
  switch (terminator.kind) {
    case "return":
      emitValue(body, terminator.value);
      body.push("return");
      return;
    case "jump":
      throw new Error("Cannot emit WAT for MIR jump terminators in the scalar WAT backend.");
    case "branch":
      throw new Error("Cannot emit WAT for MIR branch terminators in the scalar WAT backend.");
  }
}

function emitValue(body: string[], value: MirValue): void {
  switch (value.kind) {
    case "param":
    case "local":
    case "temp":
      body.push(`local.get ${toWasmIdentifier(value.name)}`);
      return;
    case "const_int":
      body.push(`${mirTypeToWasmValueType(value.type)}.const ${value.text}`);
      return;
    case "const_bool":
      body.push(`i32.const ${value.value ? "1" : "0"}`);
      return;
  }
}

function emitAddress(body: string[], place: MirPlace, context: MirWasmContext): void {
  switch (place.kind) {
    case "param":
    case "local":
      if (place.type.kind !== "pointer") {
        throw new Error(`Cannot emit WASM address for non-pointer place '${place.name}'.`);
      }
      body.push(`local.get ${toWasmIdentifier(place.name)}`);
      return;
    case "index": {
      if (place.base.type.kind !== "pointer") {
        throw new Error("Cannot emit WASM index address for non-pointer base.");
      }
      emitAddress(body, place.base, context);
      emitValue(body, place.index);
      body.push(`i32.const ${sizeOfMirWasmType(place.base.type.elementType, context)}`);
      body.push("i32.mul");
      body.push("i32.add");
      return;
    }
    case "field": {
      if (place.base.type.kind !== "struct") {
        throw new Error("Cannot emit WASM field address for non-struct base.");
      }
      emitAddress(body, place.base, context);
      const field = requireMirFieldLayout(place.base.type.name, place.fieldName, context);
      if (field.offset !== 0) {
        body.push(`i32.const ${field.offset}`);
        body.push("i32.add");
      }
      return;
    }
  }
}

function emitSet(body: string[], target: MirValue): void {
  switch (target.kind) {
    case "param":
    case "local":
    case "temp":
      body.push(`local.set ${toWasmIdentifier(target.name)}`);
      return;
    case "const_int":
    case "const_bool":
      throw new Error("Cannot assign to a MIR constant value.");
  }
}

function binaryWatInstruction(op: MirBinaryOp, type: MirType): string {
  const wasmType = mirTypeToWasmValueType(type);
  switch (op) {
    case "+":
      return `${wasmType}.add`;
    case "-":
      return `${wasmType}.sub`;
    case "*":
      return `${wasmType}.mul`;
    case "/":
      return `${wasmType}.div_${isUnsignedIntegerType(type) ? "u" : "s"}`;
    case "%":
      return `${wasmType}.rem_${isUnsignedIntegerType(type) ? "u" : "s"}`;
  }
}

function compareWatInstruction(op: MirCompareOp, type: MirType): string {
  const wasmType = mirTypeToWasmValueType(type);
  switch (op) {
    case "==":
      return `${wasmType}.eq`;
    case "!=":
      return `${wasmType}.ne`;
    case "<":
      return `${wasmType}.lt_${isUnsignedIntegerType(type) ? "u" : "s"}`;
    case "<=":
      return `${wasmType}.le_${isUnsignedIntegerType(type) ? "u" : "s"}`;
    case ">":
      return `${wasmType}.gt_${isUnsignedIntegerType(type) ? "u" : "s"}`;
    case ">=":
      return `${wasmType}.ge_${isUnsignedIntegerType(type) ? "u" : "s"}`;
  }
}

function createMirWasmContext(structs: MirStruct[]): MirWasmContext {
  const layouts = new Map<string, MirStructLayout>();
  const pending = new Set(structs);
  const context: MirWasmContext = { structs: layouts };

  while (pending.size > 0) {
    let madeProgress = false;

    for (const struct of pending) {
      try {
        layouts.set(struct.name, computeMirStructLayout(struct, context));
        pending.delete(struct);
        madeProgress = true;
      } catch (error) {
        if (!(error instanceof MissingMirStructLayoutError)) {
          throw error;
        }
      }
    }

    if (!madeProgress) {
      const names = [...pending].map((struct) => struct.name).join(", ");
      throw new Error(`Cannot calculate WASM struct layouts; unresolved struct field layout for: ${names}`);
    }
  }

  return context;
}

function computeMirStructLayout(struct: MirStruct, context: MirWasmContext): MirStructLayout {
  let offset = 0;
  let structAlign = 1;
  const fields = new Map<string, MirStructFieldLayout>();

  for (const field of struct.fields) {
    const fieldAlign = alignOfMirWasmType(field.type, context);
    const fieldSize = sizeOfMirWasmType(field.type, context);
    offset = alignUp(offset, fieldAlign);
    structAlign = Math.max(structAlign, fieldAlign);
    fields.set(field.name, {
      name: field.name,
      type: field.type,
      offset,
      size: fieldSize,
      align: fieldAlign
    });
    offset += fieldSize;
  }

  return {
    name: struct.name,
    size: alignUp(offset, structAlign),
    align: structAlign,
    fields
  };
}

function sizeOfMirWasmType(type: MirType, context: MirWasmContext): number {
  switch (type.kind) {
    case "primitive":
      switch (type.name) {
        case "i32":
        case "u32":
        case "bool":
          return 4;
        case "i64":
        case "u64":
          return 8;
      }
    case "pointer":
      return 4;
    case "struct":
      return requireMirStructLayout(type.name, context).size;
  }
}

function alignOfMirWasmType(type: MirType, context: MirWasmContext): number {
  switch (type.kind) {
    case "primitive":
      switch (type.name) {
        case "i32":
        case "u32":
        case "bool":
          return 4;
        case "i64":
        case "u64":
          return 8;
      }
    case "pointer":
      return 4;
    case "struct":
      return requireMirStructLayout(type.name, context).align;
  }
}

function requireMirStructLayout(name: string, context: MirWasmContext): MirStructLayout {
  const layout = context.structs.get(name);
  if (!layout) {
    throw new MissingMirStructLayoutError(name);
  }

  return layout;
}

function requireMirFieldLayout(structName: string, fieldName: string, context: MirWasmContext): MirStructFieldLayout {
  const layout = requireMirStructLayout(structName, context);
  const field = layout.fields.get(fieldName);
  if (!field) {
    throw new Error(`Unknown field '${fieldName}' on struct '${structName}' while emitting WASM.`);
  }

  return field;
}

function alignUp(value: number, alignment: number): number {
  return Math.ceil(value / alignment) * alignment;
}

class MissingMirStructLayoutError extends Error {
  constructor(readonly structName: string) {
    super(`Missing WASM layout for struct '${structName}'.`);
  }
}

function mirTypeToWasmValueType(type: MirType): WasmValueType {
  switch (type.kind) {
    case "primitive":
      switch (type.name) {
        case "i32":
        case "u32":
        case "bool":
          return "i32";
        case "i64":
        case "u64":
          return "i64";
      }
    case "pointer":
      return "i32";
    case "struct":
      throw new Error(`Struct type '${type.name}' is a memory layout type and does not map directly to a WASM value type.`);
  }
}

function isUnsignedIntegerType(type: MirType): boolean {
  return type.kind === "primitive" && (type.name === "u32" || type.name === "u64");
}
