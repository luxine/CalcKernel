import type {
  AssignmentStatement,
  BinaryExpression,
  BlockStatement,
  BoolLiteral,
  CallExpression,
  Expression,
  FieldExpression,
  FunctionDeclaration,
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
} from "../../parser/ast.js";
import type { CheckResult } from "../../typeck/checker.js";
import { primitiveType, type IntKernelType } from "../../typeck/types.js";
import { assertCanEmitC, emitCheckedFunctionSignature, emitCType, emitFunctionSignature } from "./c-header-emitter.js";
import { emitCPrimitiveType, escapeCIncludePath } from "./c-common.js";
import { resolveOverflowMode, type CCodegenOptions } from "./c-options.js";

export interface EmitCSourceOptions extends CCodegenOptions {
  headerFileName: string;
}

/**
 * Legacy AST-to-C source emitter retained for regression comparison and
 * emergency fallback. The default CLI/build pipeline emits C from MIR.
 *
 * @internal
 */
export function emitCSource(checked: CheckResult, options: EmitCSourceOptions): string {
  assertCanEmitC(checked);

  if (resolveOverflowMode(options) === "checked") {
    return emitCheckedCSource(checked, options);
  }

  const functions = checked.ast.declarations.filter(
    (declaration): declaration is FunctionDeclaration => declaration.kind === "FunctionDeclaration"
  );
  const lines = [`#include "${escapeCIncludePath(options.headerFileName)}"`];

  for (const functionDeclaration of functions) {
    lines.push("", emitFunction(functionDeclaration));
  }

  return `${lines.join("\n")}\n`;
}

function emitFunction(functionDeclaration: FunctionDeclaration): string {
  const prefix = functionDeclaration.exported ? "" : "static ";
  const lines = [`${prefix}${emitFunctionSignature(functionDeclaration)} {`];
  lines.push(...emitStatements(functionDeclaration.body.statements, 1, true));
  lines.push("}");
  return lines.join("\n");
}

interface CheckedExpressionResult {
  lines: string[];
  value: string;
  type: IntKernelType;
}

interface CheckedAccessResult {
  lines: string[];
  value: string;
}

interface CheckedEmitContext {
  checked: CheckResult;
  tempCounter: number;
  statusCounter: number;
}

function emitCheckedCSource(checked: CheckResult, options: EmitCSourceOptions): string {
  const functions = checked.ast.declarations.filter(
    (declaration): declaration is FunctionDeclaration => declaration.kind === "FunctionDeclaration"
  );
  const lines = [`#include "${escapeCIncludePath(options.headerFileName)}"`];

  for (const functionDeclaration of functions) {
    lines.push("", emitCheckedFunction(checked, functionDeclaration));
  }

  return `${lines.join("\n")}\n`;
}

function emitCheckedFunction(checked: CheckResult, functionDeclaration: FunctionDeclaration): string {
  const prefix = functionDeclaration.exported ? "" : "static ";
  const context: CheckedEmitContext = { checked, tempCounter: 0, statusCounter: 0 };
  const lines = [`${prefix}${emitCheckedFunctionSignature(functionDeclaration)} {`];

  lines.push("  if (ik_return == NULL) {", "    return IK_ERR_NULL_POINTER;", "  }");

  const bodyLines = emitCheckedStatements(context, functionDeclaration.body.statements, 1, true);
  if (bodyLines.length > 0) {
    lines.push("", ...bodyLines);
  }

  lines.push("}");
  return lines.join("\n");
}

function emitCheckedStatements(
  context: CheckedEmitContext,
  statements: Statement[],
  indentLevel: number,
  separateStatements: boolean
): string[] {
  const lines: string[] = [];

  statements.forEach((statement, index) => {
    if (separateStatements && index > 0) {
      lines.push("");
    }
    lines.push(...emitCheckedStatement(context, statement, indentLevel));
  });

  return lines;
}

function emitCheckedStatement(context: CheckedEmitContext, statement: Statement, indentLevel: number): string[] {
  const pad = indent(indentLevel);

  switch (statement.kind) {
    case "LetStatement":
      return emitCheckedLetStatement(context, statement, indentLevel);
    case "AssignmentStatement":
      return emitCheckedAssignmentStatement(context, statement, indentLevel);
    case "ReturnStatement":
      return emitCheckedReturnStatement(context, statement, indentLevel);
    case "IfStatement":
      return emitCheckedIfStatement(context, statement, indentLevel);
    case "WhileStatement":
      return emitCheckedWhileStatement(context, statement, indentLevel);
    case "BlockStatement": {
      const lines = [`${pad}{`];
      lines.push(...emitCheckedStatements(context, statement.statements, indentLevel + 1, false));
      lines.push(`${pad}}`);
      return lines;
    }
    case "ErrorStatement":
      throw new Error("Cannot emit checked C for error statement.");
  }
}

function emitCheckedLetStatement(context: CheckedEmitContext, statement: LetStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const initializer = lowerCheckedExpression(context, statement.initializer);

  return [
    `${pad}${emitCType(statement.type)} ${statement.name.name};`,
    ...indentCheckedLines(initializer.lines, indentLevel),
    `${pad}${statement.name.name} = ${initializer.value};`
  ];
}

function emitCheckedAssignmentStatement(context: CheckedEmitContext, statement: AssignmentStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const target = lowerCheckedAssignmentTarget(context, statement.target);
  const value = lowerCheckedExpression(context, statement.value);

  return [...indentCheckedLines([...target.lines, ...value.lines], indentLevel), `${pad}${target.value} = ${value.value};`];
}

function emitCheckedReturnStatement(context: CheckedEmitContext, statement: ReturnStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const value = lowerCheckedExpression(context, statement.value);
  const lines = [...indentCheckedLines(value.lines, indentLevel)];

  if (lines.length > 0) {
    lines.push("");
  }

  lines.push(`${pad}*ik_return = ${value.value};`, `${pad}return IK_OK;`);
  return lines;
}

function emitCheckedIfStatement(context: CheckedEmitContext, statement: IfStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const condition = lowerCheckedExpression(context, statement.condition);
  const lines = [...indentCheckedLines(condition.lines, indentLevel), `${pad}if (${condition.value}) {`];

  lines.push(...emitCheckedStatements(context, statement.thenBlock.statements, indentLevel + 1, false));

  if (!statement.elseBlock) {
    lines.push(`${pad}}`);
    return lines;
  }

  lines.push(`${pad}} else {`);
  lines.push(...emitCheckedStatements(context, statement.elseBlock.statements, indentLevel + 1, false));
  lines.push(`${pad}}`);
  return lines;
}

function emitCheckedWhileStatement(context: CheckedEmitContext, statement: WhileStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const condition = lowerCheckedExpression(context, statement.condition);
  const lines = [`${pad}while (true) {`];

  lines.push(...indentCheckedLines(condition.lines, indentLevel + 1));
  lines.push(`${indent(indentLevel + 1)}if (!${condition.value}) {`);
  lines.push(`${indent(indentLevel + 2)}break;`);
  lines.push(`${indent(indentLevel + 1)}}`);
  lines.push(...emitCheckedStatements(context, statement.body.statements, indentLevel + 1, false));
  lines.push(`${pad}}`);
  return lines;
}

function lowerCheckedExpression(context: CheckedEmitContext, expression: Expression): CheckedExpressionResult {
  switch (expression.kind) {
    case "IdentifierExpression":
      return {
        lines: [],
        value: emitIdentifierExpression(expression),
        type: checkedExpressionType(context, expression)
      };
    case "IntegerLiteral":
      return {
        lines: [],
        value: emitIntegerLiteral(expression),
        type: checkedExpressionType(context, expression)
      };
    case "BoolLiteral":
      return {
        lines: [],
        value: emitBoolLiteral(expression),
        type: checkedExpressionType(context, expression)
      };
    case "UnaryExpression":
      return lowerCheckedUnaryExpression(context, expression);
    case "BinaryExpression":
      return lowerCheckedBinaryExpression(context, expression);
    case "CallExpression":
      return lowerCheckedCallExpression(context, expression);
    case "FieldExpression":
      return lowerCheckedFieldExpression(context, expression);
    case "IndexExpression":
      return lowerCheckedIndexExpression(context, expression);
    case "ParenthesizedExpression":
      return lowerCheckedExpression(context, expression.expression);
    case "ErrorExpression":
      throw new Error("Cannot emit checked C for error expression.");
  }
}

function lowerCheckedAssignmentTarget(context: CheckedEmitContext, expression: Expression): CheckedAccessResult {
  switch (expression.kind) {
    case "IdentifierExpression":
      return { lines: [], value: emitIdentifierExpression(expression) };
    case "FieldExpression":
      return lowerCheckedFieldAccess(context, expression);
    case "IndexExpression":
      return lowerCheckedIndexAccess(context, expression);
    case "ParenthesizedExpression":
      return lowerCheckedAssignmentTarget(context, expression.expression);
    default:
      throw unsupportedCheckedCodegen(`${expression.kind} assignment target`);
  }
}

function lowerCheckedAccessValue(context: CheckedEmitContext, expression: Expression): CheckedAccessResult {
  switch (expression.kind) {
    case "IdentifierExpression":
      return { lines: [], value: emitIdentifierExpression(expression) };
    case "FieldExpression":
      return lowerCheckedFieldAccess(context, expression);
    case "IndexExpression":
      return lowerCheckedIndexAccess(context, expression);
    case "ParenthesizedExpression":
      return lowerCheckedAccessValue(context, expression.expression);
    default: {
      const lowered = lowerCheckedExpression(context, expression);
      return { lines: lowered.lines, value: lowered.value };
    }
  }
}

function lowerCheckedFieldExpression(context: CheckedEmitContext, expression: FieldExpression): CheckedExpressionResult {
  const access = lowerCheckedFieldAccess(context, expression);
  return {
    ...access,
    type: checkedExpressionType(context, expression)
  };
}

function lowerCheckedFieldAccess(context: CheckedEmitContext, expression: FieldExpression): CheckedAccessResult {
  const object = lowerCheckedAccessValue(context, expression.object);
  return {
    lines: object.lines,
    value: `${object.value}.${expression.field.name}`
  };
}

function lowerCheckedIndexExpression(context: CheckedEmitContext, expression: IndexExpression): CheckedExpressionResult {
  const access = lowerCheckedIndexAccess(context, expression);
  return {
    ...access,
    type: checkedExpressionType(context, expression)
  };
}

function lowerCheckedIndexAccess(context: CheckedEmitContext, expression: IndexExpression): CheckedAccessResult {
  const object = lowerCheckedAccessValue(context, expression.object);
  const index = lowerCheckedExpression(context, expression.index);
  return {
    lines: [...object.lines, ...index.lines],
    value: `${object.value}[${index.value}]`
  };
}

function lowerCheckedUnaryExpression(context: CheckedEmitContext, expression: UnaryExpression): CheckedExpressionResult {
  const operand = lowerCheckedExpression(context, expression.operand);

  if (expression.operator === "!") {
    const temp = nextCheckedTemp(context);
    return {
      lines: [...operand.lines, `bool ${temp} = (!${operand.value});`],
      value: temp,
      type: primitiveType("bool")
    };
  }

  const type = checkedExpressionType(context, expression);
  const cType = emitCheckedCType(type);
  const temp = nextCheckedTemp(context);

  if (isUnsignedIntegerType(type)) {
    return {
      lines: [
        ...operand.lines,
        `${cType} ${temp};`,
        `if (__builtin_sub_overflow((${cType})0, ${operand.value}, &${temp})) {`,
        "  return IK_ERR_OVERFLOW;",
        "}"
      ],
      value: temp,
      type
    };
  }

  return {
    lines: [
      ...operand.lines,
      `if (${operand.value} == ${signedMinConstant(type)}) {`,
      "  return IK_ERR_OVERFLOW;",
      "}",
      `${cType} ${temp} = -${operand.value};`
    ],
    value: temp,
    type
  };
}

function lowerCheckedCallExpression(context: CheckedEmitContext, expression: CallExpression): CheckedExpressionResult {
  if (expression.callee.kind !== "IdentifierExpression") {
    throw unsupportedCheckedCodegen("non-identifier function callee");
  }

  const args = expression.args.map((arg) => lowerCheckedExpression(context, arg));
  const type = checkedExpressionType(context, expression);
  const cType = emitCheckedCType(type);
  const temp = nextCheckedTemp(context);
  const status = nextCheckedStatus(context);
  const argValues = args.map((arg) => arg.value);
  const callArgs = [...argValues, `&${temp}`].join(", ");

  return {
    lines: [
      ...args.flatMap((arg) => arg.lines),
      `${cType} ${temp};`,
      `IK_Status ${status} = ${emitIdentifierExpression(expression.callee)}(${callArgs});`,
      `if (${status} != IK_OK) {`,
      `  return ${status};`,
      "}"
    ],
    value: temp,
    type
  };
}

function lowerCheckedBinaryExpression(context: CheckedEmitContext, expression: BinaryExpression): CheckedExpressionResult {
  if (expression.operator === "&&" || expression.operator === "||") {
    return lowerCheckedLogicalExpression(context, expression);
  }

  const left = lowerCheckedExpression(context, expression.left);
  const right = lowerCheckedExpression(context, expression.right);
  const type = checkedExpressionType(context, expression);

  if (isCheckedArithmeticOperator(expression.operator)) {
    return lowerCheckedArithmeticExpression(context, expression, left, right, type);
  }

  if (isCheckedComparisonOperator(expression.operator)) {
    const temp = nextCheckedTemp(context);
    return {
      lines: [...left.lines, ...right.lines, `bool ${temp} = (${left.value} ${expression.operator} ${right.value});`],
      value: temp,
      type
    };
  }

  throw unsupportedCheckedCodegen(`binary operator '${expression.operator}'`);
}

function lowerCheckedLogicalExpression(context: CheckedEmitContext, expression: BinaryExpression): CheckedExpressionResult {
  const left = lowerCheckedExpression(context, expression.left);
  const type = checkedExpressionType(context, expression);
  const temp = nextCheckedTemp(context);
  const right = lowerCheckedExpression(context, expression.right);

  if (expression.operator === "&&") {
    return {
      lines: [
        ...left.lines,
        `bool ${temp};`,
        `if (!${left.value}) {`,
        `  ${temp} = false;`,
        "} else {",
        ...indentCheckedLines(right.lines, 1),
        `  ${temp} = ${right.value};`,
        "}"
      ],
      value: temp,
      type
    };
  }

  return {
    lines: [
      ...left.lines,
      `bool ${temp};`,
      `if (${left.value}) {`,
      `  ${temp} = true;`,
      "} else {",
      ...indentCheckedLines(right.lines, 1),
      `  ${temp} = ${right.value};`,
      "}"
    ],
    value: temp,
    type
  };
}

function lowerCheckedArithmeticExpression(
  context: CheckedEmitContext,
  expression: BinaryExpression,
  left: CheckedExpressionResult,
  right: CheckedExpressionResult,
  type: IntKernelType
): CheckedExpressionResult {
  const cType = emitCheckedCType(type);
  const temp = nextCheckedTemp(context);
  const prefixLines = [...left.lines, ...right.lines];

  switch (expression.operator) {
    case "+":
      return {
        lines: [...prefixLines, ...emitCheckedOverflowBuiltin("__builtin_add_overflow", left.value, right.value, temp, cType)],
        value: temp,
        type
      };
    case "-":
      return {
        lines: [...prefixLines, ...emitCheckedOverflowBuiltin("__builtin_sub_overflow", left.value, right.value, temp, cType)],
        value: temp,
        type
      };
    case "*":
      return {
        lines: [...prefixLines, ...emitCheckedOverflowBuiltin("__builtin_mul_overflow", left.value, right.value, temp, cType)],
        value: temp,
        type
      };
    case "/":
    case "%":
      return {
        lines: [...prefixLines, ...emitCheckedDivisionOrModulo(expression.operator, left.value, right.value, temp, type, cType)],
        value: temp,
        type
      };
    default:
      throw unsupportedCheckedCodegen(`arithmetic operator '${expression.operator}'`);
  }
}

function emitCheckedOverflowBuiltin(builtin: string, left: string, right: string, temp: string, cType: string): string[] {
  return [
    `${cType} ${temp};`,
    `if (${builtin}(${left}, ${right}, &${temp})) {`,
    "  return IK_ERR_OVERFLOW;",
    "}"
  ];
}

function emitCheckedDivisionOrModulo(operator: "/" | "%", left: string, right: string, temp: string, type: IntKernelType, cType: string): string[] {
  const lines = [
    `if (${right} == 0) {`,
    "  return IK_ERR_DIV_BY_ZERO;",
    "}"
  ];

  if (isSignedIntegerType(type)) {
    lines.push(
      `if (${left} == ${signedMinConstant(type)} && ${right} == -1) {`,
      "  return IK_ERR_OVERFLOW;",
      "}"
    );
  }

  lines.push(`${cType} ${temp} = (${left} ${operator} ${right});`);
  return lines;
}

function emitStatements(statements: Statement[], indentLevel: number, separateStatements: boolean): string[] {
  const lines: string[] = [];

  statements.forEach((statement, index) => {
    if (separateStatements && index > 0) {
      lines.push("");
    }
    lines.push(...emitStatement(statement, indentLevel));
  });

  return lines;
}

function emitStatement(statement: Statement, indentLevel: number): string[] {
  const pad = indent(indentLevel);

  switch (statement.kind) {
    case "LetStatement":
      return [emitLetStatement(statement, pad)];
    case "AssignmentStatement":
      return [emitAssignmentStatement(statement, pad)];
    case "ReturnStatement":
      return [emitReturnStatement(statement, pad)];
    case "IfStatement":
      return emitIfStatement(statement, indentLevel);
    case "WhileStatement":
      return emitWhileStatement(statement, indentLevel);
    case "BlockStatement":
      return emitBlockStatement(statement, indentLevel);
    case "ErrorStatement":
      throw new Error("Cannot emit C for error statement.");
  }
}

function emitLetStatement(statement: LetStatement, pad: string): string {
  return `${pad}${emitCType(statement.type)} ${statement.name.name} = ${emitExpression(statement.initializer)};`;
}

function emitAssignmentStatement(statement: AssignmentStatement, pad: string): string {
  return `${pad}${emitExpression(statement.target)} = ${emitExpression(statement.value)};`;
}

function emitReturnStatement(statement: ReturnStatement, pad: string): string {
  return `${pad}return ${emitExpression(statement.value)};`;
}

function emitIfStatement(statement: IfStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const lines = [`${pad}if (${emitExpression(statement.condition)}) {`];
  lines.push(...emitStatements(statement.thenBlock.statements, indentLevel + 1, false));

  if (!statement.elseBlock) {
    lines.push(`${pad}}`);
    return lines;
  }

  lines.push(`${pad}} else {`);
  lines.push(...emitStatements(statement.elseBlock.statements, indentLevel + 1, false));
  lines.push(`${pad}}`);
  return lines;
}

function emitWhileStatement(statement: WhileStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const lines = [`${pad}while (${emitExpression(statement.condition)}) {`];
  lines.push(...emitStatements(statement.body.statements, indentLevel + 1, false));
  lines.push(`${pad}}`);
  return lines;
}

function emitBlockStatement(statement: BlockStatement, indentLevel: number): string[] {
  const pad = indent(indentLevel);
  const lines = [`${pad}{`];
  lines.push(...emitStatements(statement.statements, indentLevel + 1, false));
  lines.push(`${pad}}`);
  return lines;
}

function emitExpression(expression: Expression): string {
  switch (expression.kind) {
    case "IdentifierExpression":
      return emitIdentifierExpression(expression);
    case "IntegerLiteral":
      return emitIntegerLiteral(expression);
    case "BoolLiteral":
      return emitBoolLiteral(expression);
    case "UnaryExpression":
      return emitUnaryExpression(expression);
    case "BinaryExpression":
      return emitBinaryExpression(expression);
    case "CallExpression":
      return emitCallExpression(expression);
    case "FieldExpression":
      return emitFieldExpression(expression);
    case "IndexExpression":
      return emitIndexExpression(expression);
    case "ParenthesizedExpression":
      return emitParenthesizedExpression(expression);
    case "ErrorExpression":
      throw new Error("Cannot emit C for error expression.");
  }
}

function emitIdentifierExpression(expression: IdentifierExpression): string {
  return expression.name;
}

function emitIntegerLiteral(expression: IntegerLiteral): string {
  return expression.text;
}

function emitBoolLiteral(expression: BoolLiteral): string {
  return expression.value ? "true" : "false";
}

function emitUnaryExpression(expression: UnaryExpression): string {
  return `(${expression.operator}${emitExpression(expression.operand)})`;
}

function emitBinaryExpression(expression: BinaryExpression): string {
  return `(${emitExpression(expression.left)} ${expression.operator} ${emitExpression(expression.right)})`;
}

function emitCallExpression(expression: CallExpression): string {
  return `${emitExpression(expression.callee)}(${expression.args.map(emitExpression).join(", ")})`;
}

function emitFieldExpression(expression: FieldExpression): string {
  return `${emitExpression(expression.object)}.${expression.field.name}`;
}

function emitIndexExpression(expression: IndexExpression): string {
  return `${emitExpression(expression.object)}[${emitExpression(expression.index)}]`;
}

function emitParenthesizedExpression(expression: ParenthesizedExpression): string {
  return `(${emitExpression(expression.expression)})`;
}

function checkedExpressionType(context: CheckedEmitContext, expression: Expression): IntKernelType {
  const type = context.checked.typedAst.expressionTypes.get(expression);
  if (!type || type.kind === "unknown") {
    throw new Error(`Cannot emit checked C for expression with unresolved type '${expression.kind}'.`);
  }

  return type.kind === "integerLiteral" ? primitiveType("i32") : type;
}

function emitCheckedCType(type: IntKernelType): string {
  if (type.kind === "integerLiteral") {
    return "int32_t";
  }

  if (type.kind !== "primitive") {
    throw new Error(`Checked scalar codegen does not support non-scalar type '${type.kind}'.`);
  }

  return emitCPrimitiveType(type.name);
}

function signedMinConstant(type: IntKernelType): string {
  if (type.kind === "primitive" && type.name === "i64") {
    return "INT64_MIN";
  }

  if (type.kind === "integerLiteral" || (type.kind === "primitive" && type.name === "i32")) {
    return "INT32_MIN";
  }

  throw new Error("Checked unary minus and signed division overflow checks require a signed integer type.");
}

function isSignedIntegerType(type: IntKernelType): boolean {
  return type.kind === "integerLiteral" || (type.kind === "primitive" && (type.name === "i32" || type.name === "i64"));
}

function isUnsignedIntegerType(type: IntKernelType): boolean {
  return type.kind === "primitive" && (type.name === "u32" || type.name === "u64");
}

function nextCheckedTemp(context: CheckedEmitContext): string {
  const name = `ik_tmp${context.tempCounter}`;
  context.tempCounter += 1;
  return name;
}

function nextCheckedStatus(context: CheckedEmitContext): string {
  const name = `ik_status${context.statusCounter}`;
  context.statusCounter += 1;
  return name;
}

function indentCheckedLines(lines: string[], indentLevel: number): string[] {
  const pad = indent(indentLevel);
  return lines.map((line) => (line.length === 0 ? line : `${pad}${line}`));
}

function isCheckedArithmeticOperator(operator: string): boolean {
  return operator === "+" || operator === "-" || operator === "*" || operator === "/" || operator === "%";
}

function isCheckedComparisonOperator(operator: string): boolean {
  return operator === "==" || operator === "!=" || operator === "<" || operator === "<=" || operator === ">" || operator === ">=";
}

function unsupportedCheckedCodegen(what: string): Error {
  return new Error(`Checked scalar codegen does not support ${what} yet.`);
}

function indent(level: number): string {
  return "  ".repeat(level);
}
