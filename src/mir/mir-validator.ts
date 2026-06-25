import { printMirType } from "./mir-printer.js";
import type {
  MirBlock,
  MirFunction,
  MirInstruction,
  MirModule,
  MirPlace,
  MirPrimitiveTypeName,
  MirTerminator,
  MirType,
  MirValue
} from "./mir.js";

export interface MirValidationError {
  message: string;
  functionName?: string;
  blockLabel?: string;
}

export interface MirValidationResult {
  errors: MirValidationError[];
}

interface FunctionContext {
  module: MirModule;
  functions: Map<string, MirFunction>;
  structs: Map<string, MirModule["structs"][number]>;
  func: MirFunction;
  labels: Set<string>;
  params: Map<string, MirType>;
  locals: Map<string, MirType>;
  temps: Map<string, MirType>;
  errors: MirValidationError[];
}

export function validateMirModule(module: MirModule): MirValidationResult {
  const errors: MirValidationError[] = [];
  const functions = new Map<string, MirFunction>();
  const structs = new Map<string, MirModule["structs"][number]>();

  for (const struct of module.structs) {
    if (structs.has(struct.name)) {
      errors.push({ message: `Duplicate struct '${struct.name}'.` });
    } else {
      structs.set(struct.name, struct);
    }
  }

  for (const func of module.functions) {
    if (functions.has(func.name)) {
      errors.push({ message: `Duplicate function '${func.name}'.`, functionName: func.name });
    } else {
      functions.set(func.name, func);
    }
  }

  for (const func of module.functions) {
    validateFunction({ module, functions, structs, func, labels: new Set(), params: new Map(), locals: new Map(), temps: new Map(), errors });
  }

  return { errors };
}

function validateFunction(ctx: FunctionContext): void {
  collectParams(ctx);
  collectLocals(ctx);
  collectLabels(ctx);
  collectTemps(ctx);

  if (ctx.func.blocks.length === 0) {
    addError(ctx, `Function '${ctx.func.name}' has no entry block.`);
    return;
  }

  for (const block of ctx.func.blocks) {
    validateBlock(ctx, block);
  }
}

function collectParams(ctx: FunctionContext): void {
  for (const param of ctx.func.params) {
    if (ctx.params.has(param.name)) {
      addError(ctx, `Duplicate parameter '${param.name}' in function '${ctx.func.name}'.`);
    } else {
      ctx.params.set(param.name, param.type);
    }
  }
}

function collectLocals(ctx: FunctionContext): void {
  for (const local of ctx.func.locals) {
    if (ctx.locals.has(local.name)) {
      addError(ctx, `Duplicate local '${local.name}' in function '${ctx.func.name}'.`);
    } else {
      ctx.locals.set(local.name, local.type);
    }
  }
}

function collectLabels(ctx: FunctionContext): void {
  for (const block of ctx.func.blocks) {
    if (ctx.labels.has(block.label)) {
      addError(ctx, `Duplicate block label '${block.label}' in function '${ctx.func.name}'.`, block.label);
    } else {
      ctx.labels.add(block.label);
    }
  }
}

function collectTemps(ctx: FunctionContext): void {
  for (const block of ctx.func.blocks) {
    for (const instruction of block.instructions) {
      const target = getInstructionTarget(instruction);
      if (target?.kind !== "temp") {
        continue;
      }

      if (ctx.temps.has(target.name)) {
        addError(ctx, `Duplicate temp '%${target.name}' in function '${ctx.func.name}'.`, block.label);
      } else {
        ctx.temps.set(target.name, target.type);
      }
    }
  }
}

function validateBlock(ctx: FunctionContext, block: MirBlock): void {
  for (const instruction of block.instructions) {
    validateInstruction(ctx, block, instruction);
  }

  const terminator = block.terminator as MirTerminator | undefined;
  if (!terminator) {
    addError(ctx, `Block '${block.label}' in function '${ctx.func.name}' is missing a terminator.`, block.label);
    return;
  }

  validateTerminator(ctx, block, terminator);
}

function validateInstruction(ctx: FunctionContext, block: MirBlock, instruction: MirInstruction): void {
  switch (instruction.kind) {
    case "const_int":
      validateTarget(ctx, block, instruction.target);
      if (!isIntegerType(instruction.target.type)) {
        addError(ctx, `const_int target in function '${ctx.func.name}' must be integer, got ${printMirType(instruction.target.type)}.`, block.label);
      }
      return;
    case "const_float":
      validateTarget(ctx, block, instruction.target);
      if (!isFloatType(instruction.target.type)) {
        addError(ctx, `const_float target in function '${ctx.func.name}' must be f64, got ${printMirType(instruction.target.type)}.`, block.label);
      }
      return;
    case "const_bool":
      validateTarget(ctx, block, instruction.target);
      if (!isBoolType(instruction.target.type)) {
        addError(ctx, `const_bool target in function '${ctx.func.name}' must be bool, got ${printMirType(instruction.target.type)}.`, block.label);
      }
      return;
    case "move":
      validateTarget(ctx, block, instruction.target);
      validateValue(ctx, block, instruction.value);
      if (!sameType(instruction.target.type, instruction.value.type)) {
        addError(ctx, `Move type mismatch in function '${ctx.func.name}': expected ${printMirType(instruction.target.type)}, got ${printMirType(instruction.value.type)}.`, block.label);
      }
      return;
    case "binary":
      validateTarget(ctx, block, instruction.target);
      validateValue(ctx, block, instruction.left);
      validateValue(ctx, block, instruction.right);
      if (!sameType(instruction.left.type, instruction.right.type)) {
        addError(ctx, `Binary operands for '${instruction.op}' in function '${ctx.func.name}' must have the same type, got ${printMirType(instruction.left.type)} and ${printMirType(instruction.right.type)}.`, block.label);
      }
      if (instruction.op === "%") {
        if (isFloatType(instruction.left.type) || isFloatType(instruction.right.type)) {
          addError(ctx, `Binary operator '%' in function '${ctx.func.name}' does not support f64 operands.`, block.label);
        } else if (!isIntegerType(instruction.left.type) || !isIntegerType(instruction.right.type)) {
          addError(ctx, `Binary operands for '%' in function '${ctx.func.name}' must be integers.`, block.label);
        }
      } else if (!isNumericType(instruction.left.type) || !isNumericType(instruction.right.type)) {
        addError(ctx, `Binary operands for '${instruction.op}' in function '${ctx.func.name}' must be numeric.`, block.label);
      }
      if (!sameType(instruction.target.type, instruction.left.type)) {
        addError(ctx, `Binary result for '${instruction.op}' in function '${ctx.func.name}' must be ${printMirType(instruction.left.type)}, got ${printMirType(instruction.target.type)}.`, block.label);
      }
      return;
    case "unary":
      validateTarget(ctx, block, instruction.target);
      validateValue(ctx, block, instruction.operand);
      if (instruction.op === "neg") {
        if (!isNumericType(instruction.operand.type)) {
          addError(ctx, `Unary neg in function '${ctx.func.name}' requires numeric operand, got ${printMirType(instruction.operand.type)}.`, block.label);
        }
        if (!sameType(instruction.target.type, instruction.operand.type)) {
          addError(ctx, `Unary neg result in function '${ctx.func.name}' must be ${printMirType(instruction.operand.type)}, got ${printMirType(instruction.target.type)}.`, block.label);
        }
      } else {
        if (!isBoolType(instruction.operand.type)) {
          addError(ctx, `Unary not in function '${ctx.func.name}' requires bool operand, got ${printMirType(instruction.operand.type)}.`, block.label);
        }
        if (!isBoolType(instruction.target.type)) {
          addError(ctx, `Unary not result in function '${ctx.func.name}' must be bool, got ${printMirType(instruction.target.type)}.`, block.label);
        }
      }
      return;
    case "compare":
      validateTarget(ctx, block, instruction.target);
      validateValue(ctx, block, instruction.left);
      validateValue(ctx, block, instruction.right);
      if (!sameType(instruction.left.type, instruction.right.type)) {
        addError(ctx, `Compare operands for '${instruction.op}' in function '${ctx.func.name}' must have the same type, got ${printMirType(instruction.left.type)} and ${printMirType(instruction.right.type)}.`, block.label);
      }
      if (!isBoolType(instruction.target.type)) {
        addError(ctx, `Compare result for '${instruction.op}' in function '${ctx.func.name}' must be bool, got ${printMirType(instruction.target.type)}.`, block.label);
      }
      return;
    case "address":
      validateTarget(ctx, block, instruction.target);
      validatePlace(ctx, block, instruction.place);
      if (instruction.target.type.kind !== "pointer") {
        addError(ctx, `Address result in function '${ctx.func.name}' must be pointer, got ${printMirType(instruction.target.type)}.`, block.label);
      } else if (!sameType(instruction.target.type.elementType, instruction.place.type)) {
        addError(
          ctx,
          `Address result in function '${ctx.func.name}' must point to ${printMirType(instruction.place.type)}, got ${printMirType(instruction.target.type)}.`,
          block.label
        );
      }
      return;
    case "load":
      validateTarget(ctx, block, instruction.target);
      validatePlace(ctx, block, instruction.place);
      if (!sameType(instruction.target.type, instruction.place.type)) {
        addError(ctx, `Load type mismatch in function '${ctx.func.name}': place is ${printMirType(instruction.place.type)}, target is ${printMirType(instruction.target.type)}.`, block.label);
      }
      return;
    case "store":
      validatePlace(ctx, block, instruction.place);
      validateValue(ctx, block, instruction.value);
      if (!sameType(instruction.place.type, instruction.value.type)) {
        addError(ctx, `Store type mismatch in function '${ctx.func.name}': place is ${printMirType(instruction.place.type)}, value is ${printMirType(instruction.value.type)}.`, block.label);
      }
      return;
    case "call":
      validateTarget(ctx, block, instruction.target);
      for (const arg of instruction.args) {
        validateValue(ctx, block, arg);
      }
      validateCall(ctx, block, instruction.functionName, instruction.args, instruction.target);
      return;
  }
}

function validateTerminator(ctx: FunctionContext, block: MirBlock, terminator: MirTerminator): void {
  switch (terminator.kind) {
    case "return":
      validateValue(ctx, block, terminator.value);
      if (!sameType(terminator.value.type, ctx.func.returnType)) {
        addError(ctx, `Return type mismatch in function '${ctx.func.name}': expected ${printMirType(ctx.func.returnType)}, got ${printMirType(terminator.value.type)}.`, block.label);
      }
      return;
    case "jump":
      if (!ctx.labels.has(terminator.label)) {
        addError(ctx, `Jump target '${terminator.label}' does not exist in function '${ctx.func.name}'.`, block.label);
      }
      return;
    case "branch":
      validateValue(ctx, block, terminator.condition);
      if (!isBoolType(terminator.condition.type)) {
        addError(ctx, `Branch condition in function '${ctx.func.name}' must be bool, got ${printMirType(terminator.condition.type)}.`, block.label);
      }
      if (!ctx.labels.has(terminator.thenLabel)) {
        addError(ctx, `Branch target '${terminator.thenLabel}' does not exist in function '${ctx.func.name}'.`, block.label);
      }
      if (!ctx.labels.has(terminator.elseLabel)) {
        addError(ctx, `Branch target '${terminator.elseLabel}' does not exist in function '${ctx.func.name}'.`, block.label);
      }
      return;
  }
}

function validateTarget(ctx: FunctionContext, block: MirBlock, target: MirValue): void {
  switch (target.kind) {
    case "temp":
      if (!ctx.temps.has(target.name)) {
        addError(ctx, `Unknown temp '%${target.name}' in function '${ctx.func.name}'.`, block.label);
      }
      return;
    case "local":
      validateValue(ctx, block, target);
      return;
    case "param":
      validateValue(ctx, block, target);
      return;
    case "const_int":
    case "const_float":
    case "const_bool":
      addError(ctx, `Instruction target in function '${ctx.func.name}' must be a temp, local, or param.`, block.label);
      return;
  }
}

function validateValue(ctx: FunctionContext, block: MirBlock, value: MirValue): void {
  switch (value.kind) {
    case "param": {
      const declared = ctx.params.get(value.name);
      if (!declared) {
        addError(ctx, `Unknown param '${value.name}' in function '${ctx.func.name}'.`, block.label);
      } else if (!sameType(declared, value.type)) {
        addError(ctx, `Param '${value.name}' in function '${ctx.func.name}' has type ${printMirType(declared)}, got ${printMirType(value.type)}.`, block.label);
      }
      return;
    }
    case "local": {
      const declared = ctx.locals.get(value.name);
      if (!declared) {
        addError(ctx, `Unknown local '${value.name}' in function '${ctx.func.name}'.`, block.label);
      } else if (!sameType(declared, value.type)) {
        addError(ctx, `Local '${value.name}' in function '${ctx.func.name}' has type ${printMirType(declared)}, got ${printMirType(value.type)}.`, block.label);
      }
      return;
    }
    case "temp": {
      const declared = ctx.temps.get(value.name);
      if (!declared) {
        addError(ctx, `Unknown temp '%${value.name}' in function '${ctx.func.name}'.`, block.label);
      } else if (!sameType(declared, value.type)) {
        addError(ctx, `Temp '%${value.name}' in function '${ctx.func.name}' has type ${printMirType(declared)}, got ${printMirType(value.type)}.`, block.label);
      }
      return;
    }
    case "const_int":
    case "const_float":
    case "const_bool":
      return;
  }
}

function validatePlace(ctx: FunctionContext, block: MirBlock, place: MirPlace): void {
  switch (place.kind) {
    case "param":
    case "local":
      validateValue(ctx, block, place);
      return;
    case "deref":
      validateValue(ctx, block, place.pointer);
      if (place.pointer.type.kind !== "pointer") {
        addError(ctx, `Deref place in function '${ctx.func.name}' requires pointer value, got ${printMirType(place.pointer.type)}.`, block.label);
      } else if (!sameType(place.pointer.type.elementType, place.type)) {
        addError(ctx, `Deref place type mismatch in function '${ctx.func.name}': pointer element is ${printMirType(place.pointer.type.elementType)}, place is ${printMirType(place.type)}.`, block.label);
      }
      return;
    case "index":
      validatePlace(ctx, block, place.base);
      validateValue(ctx, block, place.index);
      if (!isIndexType(place.index.type)) {
        addError(ctx, `Index place in function '${ctx.func.name}' requires i32 or u32 index, got ${printMirType(place.index.type)}.`, block.label);
      }
      if (place.base.type.kind !== "pointer") {
        addError(ctx, `Index base in function '${ctx.func.name}' must be pointer, got ${printMirType(place.base.type)}.`, block.label);
      } else if (!sameType(place.base.type.elementType, place.type)) {
        addError(ctx, `Index place type mismatch in function '${ctx.func.name}': expected ${printMirType(place.base.type.elementType)}, got ${printMirType(place.type)}.`, block.label);
      }
      return;
    case "field": {
      validatePlace(ctx, block, place.base);
      if (place.base.type.kind !== "struct") {
        addError(ctx, `Field base in function '${ctx.func.name}' must be struct, got ${printMirType(place.base.type)}.`, block.label);
        return;
      }

      const struct = ctx.structs.get(place.base.type.name);
      if (!struct) {
        addError(ctx, `Unknown struct '${place.base.type.name}' in function '${ctx.func.name}'.`, block.label);
        return;
      }

      const field = struct.fields.find((candidate) => candidate.name === place.fieldName);
      if (!field) {
        addError(ctx, `Unknown field '${place.fieldName}' on struct '${struct.name}' in function '${ctx.func.name}'.`, block.label);
      } else if (!sameType(field.type, place.type)) {
        addError(ctx, `Field place type mismatch in function '${ctx.func.name}': field '${place.fieldName}' is ${printMirType(field.type)}, place is ${printMirType(place.type)}.`, block.label);
      }
      return;
    }
  }
}

function validateCall(ctx: FunctionContext, block: MirBlock, functionName: string, args: MirValue[], target: MirValue): void {
  const callee = ctx.functions.get(functionName);
  if (!callee) {
    addError(ctx, `Unknown function '${functionName}' in function '${ctx.func.name}'.`, block.label);
    return;
  }

  if (args.length !== callee.params.length) {
    addError(ctx, `Call to '${functionName}' in function '${ctx.func.name}' expects ${callee.params.length} argument(s), got ${args.length}.`, block.label);
    return;
  }

  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    const param = callee.params[i];
    if (!sameType(arg.type, param.type)) {
      addError(ctx, `Call argument ${i + 1} to '${functionName}' in function '${ctx.func.name}' must be ${printMirType(param.type)}, got ${printMirType(arg.type)}.`, block.label);
    }
  }

  if (!sameType(target.type, callee.returnType)) {
    addError(ctx, `Call result for '${functionName}' in function '${ctx.func.name}' must be ${printMirType(callee.returnType)}, got ${printMirType(target.type)}.`, block.label);
  }
}

function getInstructionTarget(instruction: MirInstruction): MirValue | undefined {
  switch (instruction.kind) {
    case "const_int":
    case "const_float":
    case "const_bool":
    case "move":
    case "binary":
    case "unary":
    case "compare":
    case "address":
    case "load":
    case "call":
      return instruction.target;
    case "store":
      return undefined;
  }
}

function sameType(left: MirType, right: MirType): boolean {
  if (left.kind !== right.kind) {
    return false;
  }

  switch (left.kind) {
    case "primitive":
      return right.kind === "primitive" && left.name === right.name;
    case "struct":
      return right.kind === "struct" && left.name === right.name;
    case "pointer":
      return right.kind === "pointer" && sameType(left.elementType, right.elementType);
  }
}

function isBoolType(type: MirType): boolean {
  return type.kind === "primitive" && type.name === "bool";
}

function isIntegerPrimitiveName(name: MirPrimitiveTypeName): boolean {
  return name === "i32" || name === "i64" || name === "u32" || name === "u64";
}

function isIntegerType(type: MirType): boolean {
  return type.kind === "primitive" && isIntegerPrimitiveName(type.name);
}

function isFloatType(type: MirType): boolean {
  return type.kind === "primitive" && type.name === "f64";
}

function isNumericType(type: MirType): boolean {
  return isIntegerType(type) || isFloatType(type);
}

function isIndexType(type: MirType): boolean {
  return type.kind === "primitive" && (type.name === "i32" || type.name === "u32");
}

function addError(ctx: FunctionContext, message: string, blockLabel?: string): void {
  ctx.errors.push({ message, functionName: ctx.func.name, blockLabel });
}
