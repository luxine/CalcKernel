import type {
  AssignmentStatement,
  BinaryExpression,
  BlockStatement,
  BoolLiteral,
  CallExpression,
  Expression,
  FieldExpression,
  IdentifierExpression,
  IfStatement,
  IndexExpression,
  IntegerLiteral,
  LetStatement,
  ParenthesizedExpression,
  ReturnStatement,
  Statement,
  UnaryExpression,
  WhileStatement
} from "../parser/ast.js";
import {
  getExprType,
  getLetType,
  type CheckedProgram
} from "../typeck/checker.js";
import {
  materializeIntegerLiteral,
  primitiveType,
  type IntKernelType,
  type PrimitiveTypeName
} from "../typeck/types.js";
import { MirBuilder } from "./mir-builder.js";
import type {
  MirBinaryOp,
  MirBlock,
  MirCompareOp,
  MirFunction,
  MirInstruction,
  MirLocal,
  MirModule,
  MirParam,
  MirPlace,
  MirPrimitiveTypeName,
  MirTerminator,
  MirType,
  MirValue
} from "./mir.js";

interface FunctionLowerContext {
  readonly checkedProgram: CheckedProgram;
  readonly builder: MirBuilder;
  readonly values: Map<string, MirValue>;
  readonly locals: MirLocal[];
  readonly blocks: MutableMirBlock[];
  currentBlock: MutableMirBlock | null;
  syntheticLocalCounter: number;
}

interface MutableMirBlock {
  label: string;
  instructions: MirInstruction[];
  terminator: MirTerminator | null;
}

export function lowerToMir(checkedProgram: CheckedProgram): MirModule {
  return {
    structs: checkedProgram.structs.map((struct) => ({
      name: struct.name,
      fields: struct.fields.map((field) => ({
        name: field.name,
        type: toMirType(field.type)
      }))
    })),
    functions: checkedProgram.functions.map((func) => lowerFunction(checkedProgram, func.declaration.exported, func.name))
  };
}

function lowerFunction(checkedProgram: CheckedProgram, exported: boolean, functionName: string): MirFunction {
  const functionInfo = checkedProgram.functionMap.get(functionName);
  if (!functionInfo) {
    throw new Error(`MIR lowering invariant violation: missing function '${functionName}'.`);
  }

  const builder = new MirBuilder();
  const values = new Map<string, MirValue>();
  const params: MirParam[] = functionInfo.params.map((param) => ({
    name: param.name,
    type: toMirType(param.type)
  }));

  for (const param of params) {
    values.set(param.name, { kind: "param", name: param.name, type: param.type });
  }

  const context: FunctionLowerContext = {
    checkedProgram,
    builder,
    values,
    locals: [],
    blocks: [],
    currentBlock: null,
    syntheticLocalCounter: 0
  };

  startBlock(context);
  lowerStatements(context, functionInfo.declaration.body.statements);

  if (context.currentBlock) {
    throw new Error(`MIR lowering invariant violation: function '${functionName}' has no return terminator.`);
  }

  return {
    name: functionInfo.name,
    exported,
    params,
    returnType: toMirType(functionInfo.returnType),
    locals: context.locals,
    blocks: finalizeBlocks(context, functionName)
  };
}

function lowerStatements(context: FunctionLowerContext, statements: Statement[]): void {
  for (const statement of statements) {
    if (!context.currentBlock) {
      throw unsupported("statements after return");
    }
    lowerStatement(context, statement);
  }
}

function lowerStatement(context: FunctionLowerContext, statement: Statement): void {
  switch (statement.kind) {
    case "LetStatement":
      lowerLetStatement(context, statement);
      return;
    case "AssignmentStatement":
      lowerAssignmentStatement(context, statement);
      return;
    case "ReturnStatement":
      lowerReturnStatement(context, statement);
      return;
    case "BlockStatement":
      lowerBlockStatement(context, statement);
      return;
    case "IfStatement":
      lowerIfStatement(context, statement);
      return;
    case "WhileStatement":
      lowerWhileStatement(context, statement);
      return;
    case "ErrorStatement":
      throw unsupported(statement.kind);
  }
}

function lowerBlockStatement(context: FunctionLowerContext, statement: BlockStatement): void {
  lowerStatements(context, statement.statements);
}

function lowerLetStatement(context: FunctionLowerContext, statement: LetStatement): void {
  const type = toMirType(requireLetType(context.checkedProgram, statement));
  const local: MirLocal = { name: statement.name.name, type };
  const localValue: MirValue = { kind: "local", name: local.name, type: local.type };
  context.locals.push(local);
  context.values.set(local.name, localValue);

  const initializer = lowerExpression(context, statement.initializer);
  emitInstruction(context, {
    kind: "move",
    target: localValue,
    value: initializer
  });
}

function lowerAssignmentStatement(context: FunctionLowerContext, statement: AssignmentStatement): void {
  if (statement.target.kind === "IdentifierExpression") {
    const target = requireValue(context, statement.target);
    if (target.kind !== "local") {
      throw unsupported("assignment to non-local variable");
    }

    const value = lowerExpression(context, statement.value);
    emitInstruction(context, {
      kind: "move",
      target,
      value
    });
    return;
  }

  const place = lowerPlace(context, statement.target);
  const value = lowerExpression(context, statement.value);
  emitInstruction(context, {
    kind: "store",
    place,
    value
  });
}

function lowerReturnStatement(context: FunctionLowerContext, statement: ReturnStatement): void {
  const value = lowerExpression(context, statement.value);
  setTerminator(context, { kind: "return", value });
}

function lowerIfStatement(context: FunctionLowerContext, statement: IfStatement): void {
  const condition = lowerExpression(context, statement.condition);
  const thenLabel = context.builder.nextBlockLabel();
  const elseOrJoinLabel = context.builder.nextBlockLabel();

  setTerminator(context, {
    kind: "branch",
    condition,
    thenLabel,
    elseLabel: elseOrJoinLabel
  });

  const thenBlock = startBlock(context, thenLabel);
  lowerStatements(context, statement.thenBlock.statements);

  if (!statement.elseBlock) {
    if (!thenBlock.terminator) {
      thenBlock.terminator = { kind: "jump", label: elseOrJoinLabel };
    }
    startBlock(context, elseOrJoinLabel);
    return;
  }

  const elseBlock = startBlock(context, elseOrJoinLabel);
  lowerStatements(context, statement.elseBlock.statements);

  if (thenBlock.terminator && elseBlock.terminator) {
    context.currentBlock = null;
    return;
  }

  const joinLabel = context.builder.nextBlockLabel();
  if (!thenBlock.terminator) {
    thenBlock.terminator = { kind: "jump", label: joinLabel };
  }
  if (!elseBlock.terminator) {
    elseBlock.terminator = { kind: "jump", label: joinLabel };
  }
  startBlock(context, joinLabel);
}

function lowerWhileStatement(context: FunctionLowerContext, statement: WhileStatement): void {
  const condLabel = context.builder.nextBlockLabel();
  const bodyLabel = context.builder.nextBlockLabel();
  const exitLabel = context.builder.nextBlockLabel();

  setTerminator(context, { kind: "jump", label: condLabel });

  startBlock(context, condLabel);
  const condition = lowerExpression(context, statement.condition);
  setTerminator(context, {
    kind: "branch",
    condition,
    thenLabel: bodyLabel,
    elseLabel: exitLabel
  });

  startBlock(context, bodyLabel);
  lowerStatements(context, statement.body.statements);
  if (context.currentBlock) {
    setTerminator(context, { kind: "jump", label: condLabel });
  }

  startBlock(context, exitLabel);
}

function lowerExpression(context: FunctionLowerContext, expression: Expression): MirValue {
  switch (expression.kind) {
    case "IdentifierExpression":
      return requireValue(context, expression);
    case "IntegerLiteral":
      return lowerIntegerLiteral(context, expression);
    case "BoolLiteral":
      return lowerBoolLiteral(context, expression);
    case "UnaryExpression":
      return lowerUnaryExpression(context, expression);
    case "BinaryExpression":
      return lowerBinaryExpression(context, expression);
    case "ParenthesizedExpression":
      return lowerParenthesizedExpression(context, expression);
    case "CallExpression":
      return lowerCallExpression(context, expression);
    case "FieldExpression":
      return lowerLoadExpression(context, expression);
    case "IndexExpression":
      return lowerLoadExpression(context, expression);
    case "ErrorExpression":
      throw unsupported(expression.kind);
  }
}

function lowerLoadExpression(context: FunctionLowerContext, expression: FieldExpression | IndexExpression): MirValue {
  const place = lowerPlace(context, expression);
  const target = context.builder.temp(place.type);
  emitInstruction(context, {
    kind: "load",
    target,
    place
  });
  return target;
}

function lowerIntegerLiteral(context: FunctionLowerContext, expression: IntegerLiteral): MirValue {
  const type = toMirType(requireExpressionType(context.checkedProgram, expression));
  const target = context.builder.temp(type);
  emitInstruction(context, {
    kind: "const_int",
    target,
    value: expression.text
  });
  return target;
}

function lowerBoolLiteral(context: FunctionLowerContext, expression: BoolLiteral): MirValue {
  const type = toMirType(requireExpressionType(context.checkedProgram, expression));
  const target = context.builder.temp(type);
  emitInstruction(context, {
    kind: "const_bool",
    target,
    value: expression.value
  });
  return target;
}

function lowerUnaryExpression(context: FunctionLowerContext, expression: UnaryExpression): MirValue {
  const operand = lowerExpression(context, expression.operand);
  const target = context.builder.temp(toMirType(requireExpressionType(context.checkedProgram, expression)));
  emitInstruction(context, {
    kind: "unary",
    target,
    op: expression.operator === "-" ? "neg" : "not",
    operand
  });
  return target;
}

function lowerBinaryExpression(context: FunctionLowerContext, expression: BinaryExpression): MirValue {
  if (expression.operator === "&&" || expression.operator === "||") {
    return lowerShortCircuitExpression(context, expression);
  }

  const left = lowerExpression(context, expression.left);
  const right = lowerExpression(context, expression.right);
  const target = context.builder.temp(toMirType(requireExpressionType(context.checkedProgram, expression)));

  if (isBinaryOp(expression.operator)) {
    emitInstruction(context, {
      kind: "binary",
      target,
      op: expression.operator,
      left,
      right
    });
    return target;
  }

  if (isCompareOp(expression.operator)) {
    emitInstruction(context, {
      kind: "compare",
      target,
      op: expression.operator,
      left,
      right
    });
    return target;
  }

  throw unsupported(`binary operator '${expression.operator}'`);
}

function lowerShortCircuitExpression(context: FunctionLowerContext, expression: BinaryExpression): MirValue {
  const result = createSyntheticLocal(context, toMirType(requireExpressionType(context.checkedProgram, expression)));
  const left = lowerExpression(context, expression.left);
  const firstLabel = context.builder.nextBlockLabel();
  const secondLabel = context.builder.nextBlockLabel();
  const joinLabel = context.builder.nextBlockLabel();
  const rhsLabel = expression.operator === "&&" ? firstLabel : secondLabel;
  const shortLabel = expression.operator === "&&" ? secondLabel : firstLabel;

  setTerminator(context, {
    kind: "branch",
    condition: left,
    thenLabel: expression.operator === "&&" ? rhsLabel : shortLabel,
    elseLabel: expression.operator === "&&" ? shortLabel : rhsLabel
  });

  if (expression.operator === "&&") {
    lowerShortCircuitRhsBlock(context, rhsLabel, expression.right, result, joinLabel);
    lowerShortCircuitConstantBlock(context, shortLabel, false, result, joinLabel);
  } else {
    lowerShortCircuitConstantBlock(context, shortLabel, true, result, joinLabel);
    lowerShortCircuitRhsBlock(context, rhsLabel, expression.right, result, joinLabel);
  }

  startBlock(context, joinLabel);
  return result;
}

function lowerShortCircuitRhsBlock(
  context: FunctionLowerContext,
  label: string,
  expression: Expression,
  result: MirValue,
  joinLabel: string
): void {
  startBlock(context, label);
  const right = lowerExpression(context, expression);
  emitInstruction(context, {
    kind: "move",
    target: result,
    value: right
  });
  setTerminator(context, { kind: "jump", label: joinLabel });
}

function lowerShortCircuitConstantBlock(
  context: FunctionLowerContext,
  label: string,
  value: boolean,
  result: MirValue,
  joinLabel: string
): void {
  startBlock(context, label);
  emitInstruction(context, {
    kind: "move",
    target: result,
    value: context.builder.constBool(value)
  });
  setTerminator(context, { kind: "jump", label: joinLabel });
}

function lowerCallExpression(context: FunctionLowerContext, expression: CallExpression): MirValue {
  if (expression.callee.kind !== "IdentifierExpression") {
    throw unsupported(`${expression.callee.kind} call callee`);
  }

  const args = expression.args.map((arg) => lowerExpression(context, arg));
  const target = context.builder.temp(toMirType(requireExpressionType(context.checkedProgram, expression)));
  emitInstruction(context, {
    kind: "call",
    target,
    functionName: expression.callee.name,
    args
  });
  return target;
}

function lowerParenthesizedExpression(context: FunctionLowerContext, expression: ParenthesizedExpression): MirValue {
  return lowerExpression(context, expression.expression);
}

function lowerPlace(context: FunctionLowerContext, expression: Expression): MirPlace {
  switch (expression.kind) {
    case "IdentifierExpression": {
      const value = requireValue(context, expression);
      if (value.kind !== "param" && value.kind !== "local") {
        throw unsupported(`${value.kind} place`);
      }
      return { kind: value.kind, name: value.name, type: value.type };
    }
    case "IndexExpression":
      return lowerIndexPlace(context, expression);
    case "FieldExpression":
      return lowerFieldPlace(context, expression);
    case "ParenthesizedExpression":
      return lowerPlace(context, expression.expression);
    default:
      throw unsupported(`${expression.kind} place`);
  }
}

function lowerIndexPlace(context: FunctionLowerContext, expression: IndexExpression): MirPlace {
  return {
    kind: "index",
    base: lowerPlace(context, expression.object),
    index: lowerExpression(context, expression.index),
    type: toMirType(requireExpressionType(context.checkedProgram, expression))
  };
}

function lowerFieldPlace(context: FunctionLowerContext, expression: FieldExpression): MirPlace {
  return {
    kind: "field",
    base: lowerPlace(context, expression.object),
    fieldName: expression.field.name,
    type: toMirType(requireExpressionType(context.checkedProgram, expression))
  };
}

function startBlock(context: FunctionLowerContext, label = context.builder.nextBlockLabel()): MutableMirBlock {
  const block: MutableMirBlock = {
    label,
    instructions: [],
    terminator: null
  };
  context.blocks.push(block);
  context.currentBlock = block;
  return block;
}

function createSyntheticLocal(context: FunctionLowerContext, type: MirType): MirValue {
  let name: string;
  do {
    name = `ik_sc${context.syntheticLocalCounter}`;
    context.syntheticLocalCounter += 1;
  } while (context.values.has(name));

  const local: MirLocal = { name, type };
  const value: MirValue = { kind: "local", name, type };
  context.locals.push(local);
  context.values.set(name, value);
  return value;
}

function emitInstruction(context: FunctionLowerContext, instruction: MirInstruction): void {
  if (!context.currentBlock) {
    throw unsupported("instruction after return");
  }
  context.currentBlock.instructions.push(instruction);
}

function setTerminator(context: FunctionLowerContext, terminator: MirTerminator): void {
  if (!context.currentBlock) {
    throw unsupported("terminator after return");
  }
  context.currentBlock.terminator = terminator;
  context.currentBlock = null;
}

function finalizeBlocks(context: FunctionLowerContext, functionName: string): MirBlock[] {
  return context.blocks.map((block) => {
    if (!block.terminator) {
      throw new Error(`MIR lowering invariant violation: block '${block.label}' in function '${functionName}' has no terminator.`);
    }

    return {
      label: block.label,
      instructions: block.instructions,
      terminator: block.terminator
    };
  });
}

function requireValue(context: FunctionLowerContext, expression: IdentifierExpression): MirValue {
  const value = context.values.get(expression.name);
  if (!value) {
    throw new Error(`MIR lowering invariant violation: unknown value '${expression.name}'.`);
  }
  return value;
}

function requireExpressionType(checkedProgram: CheckedProgram, expression: Expression): IntKernelType {
  const type = getExprType(checkedProgram, expression);
  if (!type) {
    throw new Error(`MIR lowering invariant violation: missing expression type for '${expression.kind}'.`);
  }
  return materializeIntegerLiteral(type, primitiveType("i32"));
}

function requireLetType(checkedProgram: CheckedProgram, statement: LetStatement): IntKernelType {
  const type = getLetType(checkedProgram, statement);
  if (!type) {
    throw new Error(`MIR lowering invariant violation: missing local type for '${statement.name.name}'.`);
  }
  return materializeIntegerLiteral(type, primitiveType("i32"));
}

function toMirType(type: IntKernelType): MirType {
  const materialized = materializeIntegerLiteral(type, primitiveType("i32"));
  switch (materialized.kind) {
    case "primitive":
      return { kind: "primitive", name: toMirPrimitiveTypeName(materialized.name) };
    case "pointer":
      return { kind: "pointer", elementType: toMirType(materialized.elementType) };
    case "struct":
      return { kind: "struct", name: materialized.name };
    case "integerLiteral":
      return { kind: "primitive", name: "i32" };
    case "unknown":
      throw new Error("MIR lowering cannot lower unknown type.");
  }
}

function toMirPrimitiveTypeName(name: PrimitiveTypeName): MirPrimitiveTypeName {
  return name;
}

function isBinaryOp(operator: string): operator is MirBinaryOp {
  return operator === "+" || operator === "-" || operator === "*" || operator === "/" || operator === "%";
}

function isCompareOp(operator: string): operator is MirCompareOp {
  return operator === "==" || operator === "!=" || operator === "<" || operator === "<=" || operator === ">" || operator === ">=";
}

function unsupported(what: string): Error {
  return new Error(`MIR scalar lowering does not support ${what} yet.`);
}
