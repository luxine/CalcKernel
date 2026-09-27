import { basename } from "node:path";
import type {
  MirBinaryInstruction,
  MirBinaryOp,
  MirBlock,
  MirCastInstruction,
  MirCallInstruction,
  MirCompareInstruction,
  MirCompareOp,
  MirFunction,
  MirInstruction,
  MirLoadInstruction,
  MirModule,
  MirPlace,
  MirStoreInstruction,
  MirTerminator,
  MirType,
  MirUnaryInstruction,
  MirValue
} from "../../mir/mir.js";
import { resolveOptimizationLevel } from "../../optimization/options.js";
import { formatLlvmFieldGep, formatLlvmPointerIndexGep, getIndexExtension } from "./llvm-gep.js";
import { LlvmIrWriter } from "./llvm-ir-writer.js";
import {
  createLlvmLayout,
  emitLlvmStructDeclaration,
  getStructFieldIndex,
  type LlvmLayout,
  type LlvmStructLayout
} from "./llvm-layout.js";
import { llvmBlockLabel, llvmFunctionName, llvmLocalName } from "./llvm-names.js";
import { emitLlvmTargetTriple, type LlvmTargetOptions } from "./llvm-target.js";
import { isSignedInteger, isUnsignedInteger, llvmParamType, llvmReturnType, llvmStorageType, llvmValueType } from "./llvm-types.js";
import type { OptimizationOptions } from "../../optimization/options.js";

export interface EmitMirLlvmModuleOptions extends LlvmTargetOptions, OptimizationOptions {
  sourceFileName?: string;
}

type AddressableValue = Extract<MirValue, { kind: "param" | "local" | "temp" }>;

export function emitMirLlvmModule(module: MirModule, options: EmitMirLlvmModuleOptions = {}): string {
  const writer = new LlvmIrWriter();
  const layout = createLlvmLayout(module.structs);
  const optLevel = resolveOptimizationLevel(options);

  writer.line("; ModuleID = 'calckernel'");
  writer.line(`source_filename = "${escapeLlvmString(stableSourceFileName(options.sourceFileName))}"`);

  const targetTriple = emitLlvmTargetTriple(options.targetTriple);
  if (targetTriple !== undefined) {
    writer.line(targetTriple);
  }

  if (module.structs.length > 0 || module.functions.length > 0) {
    writer.blankLine();
  }

  emitStructs(writer, layout.structs);
  emitFunctions(writer, module.functions, layout, optLevel);

  return writer.toString();
}

function emitStructs(writer: LlvmIrWriter, structs: LlvmStructLayout[]): void {
  structs.forEach((structInfo, index) => {
    writer.line(emitLlvmStructDeclaration(structInfo));

    if (index < structs.length - 1 || structs.length > 0) {
      writer.blankLine();
    }
  });
}

function emitFunctions(writer: LlvmIrWriter, functions: MirFunction[], layout: LlvmLayout, optLevel: number): void {
  functions.forEach((func, index) => {
    emitFunctionSkeleton(writer, func, layout, optLevel);

    if (index < functions.length - 1) {
      writer.blankLine();
    }
  });
}

function emitFunctionSkeleton(writer: LlvmIrWriter, func: MirFunction, layout: LlvmLayout, optLevel: number): void {
  const linkage = func.exported ? "" : "internal ";
  const returnType = llvmReturnType(func.returnType);
  const params = func.params.map((param) => `${llvmParamType(param.type)} ${llvmLocalName(param.name)}`).join(", ");

  writer.line(`define ${linkage}${returnType} ${llvmFunctionName(func.name)}(${params}) {`);
  if (func.blocks.length === 0) {
    writer.line("entry:");
    writer.indent(() => {
      writer.line(`ret ${returnType} ${zeroValueForType(func.returnType)}`);
    });
  } else if (optLevel >= 2 && canEmitSsaLikeFunction(func)) {
    emitSsaLikeFunctionBody(writer, func);
  } else {
    emitFunctionBody(writer, func, layout);
  }
  writer.line("}");
}

function canEmitSsaLikeFunction(func: MirFunction): boolean {
  if (func.locals.length > 0 || func.blocks.length !== 1) {
    return false;
  }

  const block = func.blocks[0]!;
  if (block.terminator.kind !== "return") {
    return false;
  }

  return block.instructions.every((instruction) =>
    instruction.kind === "const_int" ||
    instruction.kind === "const_float" ||
    instruction.kind === "const_bool" ||
    instruction.kind === "move" ||
    instruction.kind === "binary" ||
    instruction.kind === "compare" ||
    instruction.kind === "cast" ||
    instruction.kind === "unary"
  );
}

function emitSsaLikeFunctionBody(writer: LlvmIrWriter, func: MirFunction): void {
  const context: SsaFunctionEmitContext = {
    registerCounter: 0,
    values: new Map()
  };
  const block = func.blocks[0]!;

  writer.line("entry:");
  writer.indent(() => {
    for (const param of func.params) {
      context.values.set(valueIdentity({ kind: "param", name: param.name, type: param.type }), llvmLocalName(param.name));
    }

    for (const instruction of block.instructions) {
      emitSsaInstruction(writer, context, instruction);
    }

    const result = ssaValue(context, block.terminator.kind === "return" ? block.terminator.value : unreachableSsaTerminator());
    writer.line(`ret ${llvmReturnType(func.returnType)} ${result}`);
  });
}

interface SsaFunctionEmitContext {
  registerCounter: number;
  values: Map<string, string>;
}

function emitSsaInstruction(writer: LlvmIrWriter, context: SsaFunctionEmitContext, instruction: MirInstruction): void {
  switch (instruction.kind) {
    case "const_int":
      context.values.set(valueIdentity(instruction.target), instruction.value);
      return;
    case "const_float":
      context.values.set(valueIdentity(instruction.target), instruction.value);
      return;
    case "const_bool":
      context.values.set(valueIdentity(instruction.target), instruction.value ? "1" : "0");
      return;
    case "move":
      context.values.set(valueIdentity(instruction.target), ssaValue(context, instruction.value));
      return;
    case "binary": {
      const left = ssaValue(context, instruction.left);
      const right = ssaValue(context, instruction.right);
      const result = nextSsaRegister(context);
      const type = llvmValueType(instruction.target.type);
      writer.line(`${result} = ${llvmBinaryOpcode(instruction.op, instruction.target.type)} ${type} ${left}, ${right}`);
      context.values.set(valueIdentity(instruction.target), result);
      return;
    }
    case "compare": {
      const left = ssaValue(context, instruction.left);
      const right = ssaValue(context, instruction.right);
      const result = nextSsaRegister(context);
      const operandType = llvmValueType(instruction.left.type);
      writer.line(
        `${result} = ${llvmCompareInstruction(instruction.left.type)} ${llvmComparePredicate(instruction.op, instruction.left.type)} ${operandType} ${left}, ${right}`
      );
      context.values.set(valueIdentity(instruction.target), result);
      return;
    }
    case "unary": {
      const operand = ssaValue(context, instruction.operand);
      const result = nextSsaRegister(context);
      const type = llvmValueType(instruction.target.type);
      if (instruction.op === "not") {
        writer.line(`${result} = xor i1 ${operand}, true`);
      } else if (isF64Type(instruction.target.type)) {
        writer.line(`${result} = fneg ${type} ${operand}`);
      } else {
        writer.line(`${result} = sub ${type} 0, ${operand}`);
      }
      context.values.set(valueIdentity(instruction.target), result);
      return;
    }
    case "cast":
      emitSsaCastInstruction(writer, context, instruction);
      return;
    case "address":
    case "load":
    case "store":
    case "call":
      throw unsupported(`SSA-like instruction ${instruction.kind}`);
  }
}

function ssaValue(context: SsaFunctionEmitContext, value: MirValue): string {
  switch (value.kind) {
    case "const_int":
      return value.text;
    case "const_float":
      return value.text;
    case "const_bool":
      return value.value ? "1" : "0";
    case "param":
    case "local":
    case "temp": {
      const result = context.values.get(valueIdentity(value));
      if (!result) {
        throw unsupported(`SSA-like value '${value.name}' before definition`);
      }
      return result;
    }
  }
}

function valueIdentity(value: MirValue): string {
  switch (value.kind) {
    case "param":
    case "local":
    case "temp":
      return `${value.kind}:${value.name}`;
    case "const_int":
      return `const_int:${value.text}:${typeIdentity(value.type)}`;
    case "const_float":
      return `const_float:${value.text}:${typeIdentity(value.type)}`;
    case "const_bool":
      return `const_bool:${value.value ? "true" : "false"}:${typeIdentity(value.type)}`;
  }
}

function typeIdentity(type: MirType): string {
  switch (type.kind) {
    case "primitive":
      return type.name;
    case "pointer":
      return `ptr<${typeIdentity(type.elementType)}>`;
    case "struct":
      return `struct:${type.name}`;
  }
}

function nextSsaRegister(context: SsaFunctionEmitContext): string {
  const name = llvmLocalName(`v${context.registerCounter}`);
  context.registerCounter += 1;
  return name;
}

function unreachableSsaTerminator(): MirValue {
  throw new Error("internal error: SSA-like LLVM function did not have a return terminator.");
}

function emitFunctionBody(writer: LlvmIrWriter, func: MirFunction, layout: LlvmLayout): void {
  const context: FunctionEmitContext = { registerCounter: 0, layout };

  func.blocks.forEach((block, index) => {
    writer.line(`${blockLabel(func, block)}:`);
    writer.indent(() => {
      if (index === 0) {
        emitAllocas(writer, func);
        emitParamStores(writer, func);
      }

      for (const instruction of block.instructions) {
        emitInstruction(writer, context, instruction);
      }

      emitTerminator(writer, context, func, block.terminator);
    });
  });
}

interface FunctionEmitContext {
  registerCounter: number;
  layout: LlvmLayout;
}

function emitAllocas(writer: LlvmIrWriter, func: MirFunction): void {
  const tempSlots = collectTempSlots(func);

  for (const param of func.params) {
    writer.line(`${addressName({ kind: "param", name: param.name, type: param.type })} = alloca ${llvmStorageType(param.type)}`);
  }

  for (const local of func.locals) {
    writer.line(`${addressName({ kind: "local", name: local.name, type: local.type })} = alloca ${llvmStorageType(local.type)}`);
  }

  for (const temp of tempSlots) {
    writer.line(`${addressName(temp)} = alloca ${llvmStorageType(temp.type)}`);
  }
}

function emitParamStores(writer: LlvmIrWriter, func: MirFunction): void {
  for (const param of func.params) {
    writer.line(`store ${llvmParamType(param.type)} ${llvmLocalName(param.name)}, ptr ${addressName({ kind: "param", name: param.name, type: param.type })}`);
  }
}

function emitInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirInstruction): void {
  switch (instruction.kind) {
    case "const_int":
      writer.line(`store ${llvmStorageType(instruction.target.type)} ${instruction.value}, ptr ${addressName(requireAddressable(instruction.target))}`);
      return;
    case "const_float":
      writer.line(`store ${llvmStorageType(instruction.target.type)} ${instruction.value}, ptr ${addressName(requireAddressable(instruction.target))}`);
      return;
    case "const_bool":
      writer.line(`store i1 ${instruction.value ? "1" : "0"}, ptr ${addressName(requireAddressable(instruction.target))}`);
      return;
    case "move": {
      const value = loadValue(writer, context, instruction.value);
      writer.line(`store ${llvmStorageType(instruction.target.type)} ${value}, ptr ${addressName(requireAddressable(instruction.target))}`);
      return;
    }
    case "binary":
      emitBinaryInstruction(writer, context, instruction);
      return;
    case "compare":
      emitCompareInstruction(writer, context, instruction);
      return;
    case "unary":
      emitUnaryInstruction(writer, context, instruction);
      return;
    case "cast":
      emitCastInstruction(writer, context, instruction);
      return;
    case "address":
      emitAddressInstruction(writer, context, instruction);
      return;
    case "call":
      emitCallInstruction(writer, context, instruction);
      return;
    case "load":
      emitLoadInstruction(writer, context, instruction);
      return;
    case "store":
      emitStoreInstruction(writer, context, instruction);
      return;
  }
}

function emitBinaryInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirBinaryInstruction): void {
  const left = loadValue(writer, context, instruction.left);
  const right = loadValue(writer, context, instruction.right);
  const result = nextRegister(context);
  const type = llvmValueType(instruction.target.type);

  writer.line(`${result} = ${llvmBinaryOpcode(instruction.op, instruction.target.type)} ${type} ${left}, ${right}`);
  writer.line(`store ${type} ${result}, ptr ${addressName(requireAddressable(instruction.target))}`);
}

function emitCompareInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirCompareInstruction): void {
  const left = loadValue(writer, context, instruction.left);
  const right = loadValue(writer, context, instruction.right);
  const result = nextRegister(context);
  const operandType = llvmValueType(instruction.left.type);

  writer.line(`${result} = ${llvmCompareInstruction(instruction.left.type)} ${llvmComparePredicate(instruction.op, instruction.left.type)} ${operandType} ${left}, ${right}`);
  writer.line(`store i1 ${result}, ptr ${addressName(requireAddressable(instruction.target))}`);
}

function emitUnaryInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirUnaryInstruction): void {
  const operand = loadValue(writer, context, instruction.operand);
  const result = nextRegister(context);
  const type = llvmValueType(instruction.target.type);

  if (instruction.op === "not") {
    writer.line(`${result} = xor i1 ${operand}, true`);
  } else if (isF64Type(instruction.target.type)) {
    writer.line(`${result} = fneg ${type} ${operand}`);
  } else {
    writer.line(`${result} = sub ${type} 0, ${operand}`);
  }

  writer.line(`store ${type} ${result}, ptr ${addressName(requireAddressable(instruction.target))}`);
}

function emitSsaCastInstruction(writer: LlvmIrWriter, context: SsaFunctionEmitContext, instruction: MirCastInstruction): void {
  const value = ssaValue(context, instruction.value);
  const result = nextSsaRegister(context);
  const sourceType = llvmValueType(instruction.value.type);
  const targetType = llvmValueType(instruction.target.type);

  writer.line(`${result} = ${llvmCastOpcode(instruction)} ${sourceType} ${value} to ${targetType}`);
  context.values.set(valueIdentity(instruction.target), result);
}

function emitCastInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirCastInstruction): void {
  const value = loadValue(writer, context, instruction.value);
  const result = nextRegister(context);
  const sourceType = llvmValueType(instruction.value.type);
  const targetType = llvmValueType(instruction.target.type);

  writer.line(`${result} = ${llvmCastOpcode(instruction)} ${sourceType} ${value} to ${targetType}`);
  writer.line(`store ${targetType} ${result}, ptr ${addressName(requireAddressable(instruction.target))}`);
}

function llvmCastOpcode(instruction: MirCastInstruction): "sitofp" | "uitofp" {
  switch (instruction.op) {
    case "i32_to_f64":
      return "sitofp";
    case "u32_to_f64":
      return "uitofp";
  }
}

function emitCallInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirCallInstruction): void {
  const args = instruction.args.map((arg) => {
    const value = loadValue(writer, context, arg);
    return `${llvmValueType(arg.type)} ${value}`;
  });
  const result = nextRegister(context);
  const returnType = llvmReturnType(instruction.target.type);

  writer.line(`${result} = call ${returnType} ${llvmFunctionName(instruction.functionName)}(${args.join(", ")})`);
  writer.line(`store ${returnType} ${result}, ptr ${addressName(requireAddressable(instruction.target))}`);
}

function emitAddressInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: Extract<MirInstruction, { kind: "address" }>): void {
  const pointer = emitPlacePointer(writer, context, instruction.place);
  writer.line(`store ptr ${pointer}, ptr ${addressName(requireAddressable(instruction.target))}`);
}

function emitLoadInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirLoadInstruction): void {
  const pointer = emitPlacePointer(writer, context, instruction.place);
  const result = nextRegister(context);
  const type = llvmValueType(instruction.target.type);

  writer.line(`${result} = load ${type}, ptr ${pointer}`);
  writer.line(`store ${type} ${result}, ptr ${addressName(requireAddressable(instruction.target))}`);
}

function emitStoreInstruction(writer: LlvmIrWriter, context: FunctionEmitContext, instruction: MirStoreInstruction): void {
  const pointer = emitPlacePointer(writer, context, instruction.place);
  const value = loadValue(writer, context, instruction.value);
  const type = llvmValueType(instruction.value.type);

  writer.line(`store ${type} ${value}, ptr ${pointer}`);
}

function emitTerminator(writer: LlvmIrWriter, context: FunctionEmitContext, func: MirFunction, terminator: MirTerminator): void {
  switch (terminator.kind) {
    case "return": {
      const value = loadValue(writer, context, terminator.value);
      writer.line(`ret ${llvmReturnType(terminator.value.type)} ${value}`);
      return;
    }
    case "jump":
      writer.line(`br label %${blockLabelByName(func, terminator.label)}`);
      return;
    case "branch": {
      const condition = loadValue(writer, context, terminator.condition);
      writer.line(`br i1 ${condition}, label %${blockLabelByName(func, terminator.thenLabel)}, label %${blockLabelByName(func, terminator.elseLabel)}`);
      return;
    }
  }
}

function emitPlacePointer(writer: LlvmIrWriter, context: FunctionEmitContext, place: MirPlace): string {
  switch (place.kind) {
    case "param":
    case "local":
      if (place.type.kind === "pointer") {
        return loadValue(writer, context, { kind: place.kind, name: place.name, type: place.type });
      }

      return addressName({ kind: place.kind, name: place.name, type: place.type });
    case "deref":
      return loadValue(writer, context, place.pointer);
    case "index": {
      if (place.base.type.kind !== "pointer") {
        throw unsupported("index place on non-pointer base");
      }

      const basePointer = emitPlacePointer(writer, context, place.base);
      const indexValue = loadValue(writer, context, place.index);
      const index64 = emitIndexToI64(writer, context, place.index.type, indexValue);
      const result = nextRegister(context);
      writer.line(`${result} = ${formatLlvmPointerIndexGep(place.base.type.elementType, basePointer, index64)}`);
      return result;
    }
    case "field": {
      if (place.base.type.kind !== "struct") {
        throw unsupported("field place on non-struct base");
      }

      const basePointer = emitPlacePointer(writer, context, place.base);
      const fieldIndex = getStructFieldIndex(context.layout, place.base.type.name, place.fieldName);
      const result = nextRegister(context);
      writer.line(`${result} = ${formatLlvmFieldGep(place.base.type.name, basePointer, fieldIndex)}`);
      return result;
    }
  }
}

function emitIndexToI64(writer: LlvmIrWriter, context: FunctionEmitContext, indexType: MirType, indexValue: string): string {
  const extension = getIndexExtension(indexType);
  if (extension.kind === "none") {
    return indexValue;
  }

  const result = nextRegister(context);
  writer.line(`${result} = ${extension.kind} ${extension.fromType} ${indexValue} to ${extension.toType}`);
  return result;
}

function loadValue(writer: LlvmIrWriter, context: FunctionEmitContext, value: MirValue): string {
  switch (value.kind) {
    case "const_int":
      return value.text;
    case "const_float":
      return value.text;
    case "const_bool":
      return value.value ? "1" : "0";
    case "param":
    case "local":
    case "temp": {
      const result = nextRegister(context);
      writer.line(`${result} = load ${llvmValueType(value.type)}, ptr ${addressName(requireAddressable(value))}`);
      return result;
    }
  }
}

function collectTempSlots(func: MirFunction): AddressableValue[] {
  const temps: AddressableValue[] = [];
  const seen = new Set<string>();

  for (const block of func.blocks) {
    for (const instruction of block.instructions) {
      const target = instructionTarget(instruction);
      if (target?.kind === "temp" && !seen.has(target.name)) {
        seen.add(target.name);
        temps.push(requireAddressable(target));
      }
    }
  }

  return temps;
}

function instructionTarget(instruction: MirInstruction): MirValue | undefined {
  switch (instruction.kind) {
    case "const_int":
    case "const_float":
    case "const_bool":
    case "move":
    case "binary":
    case "unary":
    case "compare":
    case "cast":
    case "address":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return undefined;
  }
}

function nextRegister(context: FunctionEmitContext): string {
  const name = llvmLocalName(`v${context.registerCounter}`);
  context.registerCounter += 1;
  return name;
}

function addressName(value: AddressableValue): string {
  return `${llvmLocalName(storageName(value))}.addr`;
}

function storageName(value: AddressableValue): string {
  if (value.kind !== "temp") {
    return value.name;
  }

  const numeric = /^t(\d+)$/.exec(value.name);
  return numeric ? `ik_tmp${numeric[1]}` : `ik_tmp_${value.name}`;
}

function requireAddressable(value: MirValue): AddressableValue {
  if (value.kind === "param" || value.kind === "local" || value.kind === "temp") {
    return value;
  }

  throw unsupported(`${value.kind} stack slot`);
}

function llvmBinaryOpcode(op: MirBinaryOp, type: MirType): string {
  if (isF64Type(type)) {
    switch (op) {
      case "+":
        return "fadd";
      case "-":
        return "fsub";
      case "*":
        return "fmul";
      case "/":
        return "fdiv";
      case "%":
        throw unsupported("f64 modulo");
    }
  }

  switch (op) {
    case "+":
      return "add";
    case "-":
      return "sub";
    case "*":
      return "mul";
    case "/":
      return isUnsignedInteger(type) ? "udiv" : "sdiv";
    case "%":
      return isUnsignedInteger(type) ? "urem" : "srem";
  }
}

function llvmCompareInstruction(type: MirType): "icmp" | "fcmp" {
  return isF64Type(type) ? "fcmp" : "icmp";
}

function llvmComparePredicate(op: MirCompareOp, type: MirType): string {
  if (isF64Type(type)) {
    switch (op) {
      case "==":
        return "oeq";
      case "!=":
        return "une";
      case "<":
        return "olt";
      case "<=":
        return "ole";
      case ">":
        return "ogt";
      case ">=":
        return "oge";
    }
  }

  switch (op) {
    case "==":
      return "eq";
    case "!=":
      return "ne";
    case "<":
      return `${integerComparisonPrefix(type)}lt`;
    case "<=":
      return `${integerComparisonPrefix(type)}le`;
    case ">":
      return `${integerComparisonPrefix(type)}gt`;
    case ">=":
      return `${integerComparisonPrefix(type)}ge`;
  }
}

function integerComparisonPrefix(type: MirType): "s" | "u" {
  if (isUnsignedInteger(type)) {
    return "u";
  }

  if (isSignedInteger(type)) {
    return "s";
  }

  throw unsupported("ordered comparison for non-integer type");
}

function zeroValueForType(type: MirType): string {
  switch (type.kind) {
    case "primitive":
      if (type.name === "f64") {
        return "0.0";
      }

      return "0";
    case "pointer":
      return "null";
    case "struct":
      return "zeroinitializer";
  }
}

function isF64Type(type: MirType): boolean {
  return type.kind === "primitive" && type.name === "f64";
}

function stableSourceFileName(sourceFileName?: string): string {
  return sourceFileName === undefined ? "input.ck" : basename(sourceFileName);
}

function escapeLlvmString(value: string): string {
  return value.replaceAll("\\", "\\\\").replaceAll("\"", "\\\"");
}

function unsupported(what: string): Error {
  return new Error(`LLVM scalar emitter does not support ${what} yet.`);
}

function blockLabel(func: MirFunction, block: MirBlock): string {
  return block.label === func.blocks[0]?.label ? "entry" : llvmBlockLabel(block.label);
}

function blockLabelByName(func: MirFunction, label: string): string {
  return label === func.blocks[0]?.label ? "entry" : llvmBlockLabel(label);
}
