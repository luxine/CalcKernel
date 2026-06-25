import type {
  AssignmentStatement,
  BinaryExpression,
  BlockStatement,
  BoolLiteral,
  CallExpression,
  Declaration,
  Expression,
  FieldExpression,
  FloatLiteral,
  FunctionParam,
  FunctionDeclaration,
  IdentifierExpression,
  IfStatement,
  IndexExpression,
  IntegerLiteral,
  LetStatement,
  ParenthesizedExpression,
  Program,
  ReturnStatement,
  Statement,
  StructField,
  StructDeclaration,
  TypeNode,
  UnaryExpression,
  WhileStatement
} from "../parser/ast.js";
import { parse } from "../parser/parser.js";
import { errorAt, type Diagnostic, type DiagnosticCode } from "../source/diagnostics.js";
import { SourceFile, type SourceSpan } from "../source/source-file.js";
import { Scope, SymbolTable, type FunctionSymbol, type StructSymbol } from "./symbols.js";
import {
  canAssign,
  integerLiteralType,
  isBool,
  isFloatType,
  isIndexInteger,
  isInteger,
  isNumericType,
  isUnknown,
  materializeIntegerLiteral,
  pointerType,
  primitiveType,
  sameType,
  structType,
  typeToString,
  unknownType,
  type IntKernelType,
  type PrimitiveTypeName
} from "./types.js";

export interface TypedAst {
  program: Program;
  expressionTypes: Map<Expression, IntKernelType>;
}

export type TypeMap = Map<Expression, IntKernelType>;
export type LetTypeMap = Map<LetStatement, IntKernelType>;

export interface StructFieldInfo {
  name: string;
  type: IntKernelType;
  declaration: StructField;
}

export interface StructInfo {
  name: string;
  declaration: StructDeclaration;
  fields: StructFieldInfo[];
  fieldMap: Map<string, StructFieldInfo>;
}

export interface FunctionParamInfo {
  name: string;
  type: IntKernelType;
  declaration: FunctionParam;
}

export interface FunctionInfo {
  name: string;
  exported: boolean;
  declaration: FunctionDeclaration;
  params: FunctionParamInfo[];
  returnType: IntKernelType;
}

export interface CheckedProgram {
  ast: Program;
  symbols: SymbolTable;
  types: TypeMap;
  localTypes: LetTypeMap;
  structs: StructInfo[];
  functions: FunctionInfo[];
  structMap: Map<string, StructInfo>;
  functionMap: Map<string, FunctionInfo>;
}

export interface CheckResult {
  ast: Program;
  typedAst: TypedAst;
  checkedProgram: CheckedProgram;
  diagnostics: Diagnostic[];
  symbols: SymbolTable;
}

interface CompilerBuiltin {
  name: string;
  params: IntKernelType[];
  returnType: IntKernelType;
}

const compilerBuiltins = new Map<string, CompilerBuiltin>(
  [
    {
      name: "i32_to_f64",
      params: [primitiveType("i32")],
      returnType: primitiveType("f64")
    },
    {
      name: "u32_to_f64",
      params: [primitiveType("u32")],
      returnType: primitiveType("f64")
    }
  ].map((builtin) => [builtin.name, builtin])
);

export function check(source: SourceFile): CheckResult {
  const parseResult = parse(source);
  const checker = new Checker(source, parseResult.ast, [...parseResult.diagnostics]);
  return checker.check();
}

class Checker {
  private readonly symbols = new SymbolTable();
  private readonly expressionTypes = new Map<Expression, IntKernelType>();
  private readonly localTypes = new Map<LetStatement, IntKernelType>();

  constructor(
    private readonly source: SourceFile,
    private readonly program: Program,
    private readonly diagnostics: Diagnostic[]
  ) {}

  check(): CheckResult {
    this.collectStructNames();
    this.collectStructFields();
    this.collectFunctionSignatures();
    this.checkFunctionBodies();

    const typedAst = {
      program: this.program,
      expressionTypes: this.expressionTypes
    };

    return {
      ast: this.program,
      typedAst,
      checkedProgram: createCheckedProgram(this.program, this.symbols, this.expressionTypes, this.localTypes),
      diagnostics: this.diagnostics,
      symbols: this.symbols
    };
  }

  private collectStructNames(): void {
    for (const declaration of this.program.declarations) {
      if (declaration.kind !== "StructDeclaration") {
        continue;
      }

      const name = declaration.name.name;
      if (this.symbols.structs.has(name)) {
        this.error(declaration.name.span, `Duplicate struct '${name}'.`);
        continue;
      }

      this.symbols.structs.set(name, {
        name,
        declaration,
        fields: new Map()
      });
    }
  }

  private collectStructFields(): void {
    for (const declaration of this.program.declarations) {
      if (declaration.kind !== "StructDeclaration") {
        continue;
      }

      const symbol = this.symbols.structs.get(declaration.name.name);
      if (!symbol || symbol.declaration !== declaration) {
        continue;
      }

      for (const field of declaration.fields) {
        if (symbol.fields.has(field.name.name)) {
          this.error(field.name.span, `Duplicate field '${field.name.name}' in struct '${symbol.name}'.`);
          continue;
        }

        symbol.fields.set(field.name.name, this.resolveType(field.type));
      }
    }
  }

  private collectFunctionSignatures(): void {
    for (const declaration of this.program.declarations) {
      if (declaration.kind !== "FunctionDeclaration") {
        continue;
      }

      const name = declaration.name.name;
      if (compilerBuiltins.has(name)) {
        this.error(declaration.name.span, `Cannot define reserved compiler builtin '${name}'.`);
        continue;
      }

      if (this.symbols.functions.has(name)) {
        this.error(declaration.name.span, `Duplicate function '${name}'.`);
        continue;
      }

      this.symbols.functions.set(name, {
        name,
        declaration,
        params: declaration.params.map((param) => this.resolveType(param.type)),
        returnType: this.resolveType(declaration.returnType)
      });
    }
  }

  private checkFunctionBodies(): void {
    for (const declaration of this.program.declarations) {
      if (declaration.kind !== "FunctionDeclaration") {
        continue;
      }

      const functionSymbol = this.symbols.functions.get(declaration.name.name);
      if (!functionSymbol || functionSymbol.declaration !== declaration) {
        continue;
      }

      this.checkFunctionBody(declaration, functionSymbol);
    }
  }

  private checkFunctionBody(declaration: FunctionDeclaration, functionSymbol: FunctionSymbol): void {
    const scope = new Scope();

    declaration.params.forEach((param, index) => {
      const name = param.name.name;
      const type = functionSymbol.params[index] ?? unknownType;
      if (!scope.declare({ name, type })) {
        this.error(param.name.span, `Duplicate variable '${name}'.`);
      }
    });

    this.checkBlock(declaration.body, scope, functionSymbol.returnType, false);
    if (!this.blockDefinitelyReturns(declaration.body)) {
      this.error(declaration.body.span, `Missing return in function '${declaration.name.name}'.`);
    }
  }

  private checkBlock(block: BlockStatement, parentScope: Scope, returnType: IntKernelType, createScope: boolean): void {
    const scope = createScope ? new Scope(parentScope) : parentScope;

    for (const statement of block.statements) {
      this.checkStatement(statement, scope, returnType);
    }
  }

  private checkStatement(statement: Statement, scope: Scope, returnType: IntKernelType): void {
    switch (statement.kind) {
      case "BlockStatement":
        this.checkBlock(statement, scope, returnType, true);
        return;
      case "LetStatement":
        this.checkLetStatement(statement, scope);
        return;
      case "AssignmentStatement":
        this.checkAssignmentStatement(statement, scope);
        return;
      case "ReturnStatement":
        this.checkReturnStatement(statement, scope, returnType);
        return;
      case "IfStatement":
        this.checkIfStatement(statement, scope, returnType);
        return;
      case "WhileStatement":
        this.checkWhileStatement(statement, scope, returnType);
        return;
      case "ErrorStatement":
        return;
    }
  }

  private checkLetStatement(statement: LetStatement, scope: Scope): void {
    const declaredType = this.resolveType(statement.type);
    this.localTypes.set(statement, declaredType);

    if (!scope.declare({ name: statement.name.name, type: declaredType })) {
      this.error(statement.name.span, `Duplicate variable '${statement.name.name}'.`);
    }

    const initializerType = this.checkExpression(statement.initializer, scope, declaredType);
    if (!isUnknown(declaredType) && !isUnknown(initializerType) && !canAssign(declaredType, initializerType)) {
      this.error(
        statement.initializer.span,
        `Cannot initialize '${statement.name.name}': expected ${typeToString(declaredType)} but got ${typeToString(initializerType)}.`
      );
    }
  }

  private checkAssignmentStatement(statement: AssignmentStatement, scope: Scope): void {
    if (!this.isAssignableExpression(statement.target)) {
      this.error(statement.target.span, "Invalid assignment target.");
    }

    const targetType = this.checkExpression(statement.target, scope);
    const valueType = this.checkExpression(statement.value, scope, targetType);

    if (!isUnknown(targetType) && !isUnknown(valueType) && !canAssign(targetType, valueType)) {
      this.error(statement.value.span, `Cannot assign ${typeToString(valueType)} to ${typeToString(targetType)}.`);
    }
  }

  private checkReturnStatement(statement: ReturnStatement, scope: Scope, returnType: IntKernelType): void {
    const valueType = this.checkExpression(statement.value, scope, returnType);
    if (!isUnknown(returnType) && !isUnknown(valueType) && !canAssign(returnType, valueType)) {
      this.error(statement.value.span, `Return type mismatch: expected ${typeToString(returnType)} but got ${typeToString(valueType)}.`);
    }
  }

  private checkIfStatement(statement: IfStatement, scope: Scope, returnType: IntKernelType): void {
    const conditionType = materializeIntegerLiteral(this.checkExpression(statement.condition, scope));
    if (!isUnknown(conditionType) && !isBool(conditionType)) {
      this.error(statement.condition.span, `If condition must be bool, got ${typeToString(conditionType)}.`);
    }

    this.checkBlock(statement.thenBlock, scope, returnType, true);
    if (statement.elseBlock) {
      this.checkBlock(statement.elseBlock, scope, returnType, true);
    }
  }

  private checkWhileStatement(statement: WhileStatement, scope: Scope, returnType: IntKernelType): void {
    const conditionType = materializeIntegerLiteral(this.checkExpression(statement.condition, scope));
    if (!isUnknown(conditionType) && !isBool(conditionType)) {
      this.error(statement.condition.span, `While condition must be bool, got ${typeToString(conditionType)}.`);
    }

    this.checkBlock(statement.body, scope, returnType, true);
  }

  private checkExpression(expression: Expression, scope: Scope, expectedType?: IntKernelType): IntKernelType {
    switch (expression.kind) {
      case "IdentifierExpression":
        return this.checkIdentifierExpression(expression, scope);
      case "IntegerLiteral":
        return this.checkIntegerLiteral(expression, expectedType);
      case "FloatLiteral":
        return this.checkFloatLiteral(expression);
      case "BoolLiteral":
        return this.recordExpressionType(expression, { kind: "primitive", name: "bool" });
      case "UnaryExpression":
        return this.checkUnaryExpression(expression, scope, expectedType);
      case "BinaryExpression":
        return this.checkBinaryExpression(expression, scope, expectedType);
      case "CallExpression":
        return this.checkCallExpression(expression, scope);
      case "FieldExpression":
        return this.checkFieldExpression(expression, scope);
      case "IndexExpression":
        return this.checkIndexExpression(expression, scope);
      case "ParenthesizedExpression":
        return this.checkParenthesizedExpression(expression, scope, expectedType);
      case "ErrorExpression":
        return this.recordExpressionType(expression, unknownType);
    }
  }

  private checkIdentifierExpression(expression: IdentifierExpression, scope: Scope): IntKernelType {
    const symbol = scope.lookup(expression.name);
    if (!symbol) {
      this.error(expression.span, `Unknown variable '${expression.name}'.`);
      return this.recordExpressionType(expression, unknownType);
    }

    return this.recordExpressionType(expression, symbol.type);
  }

  private checkIntegerLiteral(expression: IntegerLiteral, expectedType?: IntKernelType): IntKernelType {
    const type = expectedType && isInteger(expectedType) ? expectedType : integerLiteralType;
    return this.recordExpressionType(expression, type);
  }

  private checkFloatLiteral(expression: FloatLiteral): IntKernelType {
    return this.recordExpressionType(expression, primitiveType("f64"));
  }

  private checkUnaryExpression(expression: UnaryExpression, scope: Scope, expectedType?: IntKernelType): IntKernelType {
    if (expression.operator === "!") {
      const operandType = materializeIntegerLiteral(this.checkExpression(expression.operand, scope));
      if (!isUnknown(operandType) && !isBool(operandType)) {
        this.error(expression.operand.span, `Unary operator '!' requires bool operand, got ${typeToString(operandType)}.`);
      }

      return this.recordExpressionType(expression, primitiveType("bool"));
    }

    const fallback = integerLiteralFallback(expectedType);
    const operandType = materializeIntegerLiteral(this.checkExpression(expression.operand, scope, fallback), fallback);
    if (!isUnknown(operandType) && !isNumericType(operandType)) {
      this.error(expression.operand.span, `Unary operator '-' requires integer operand, got ${typeToString(operandType)}.`);
      return this.recordExpressionType(expression, unknownType);
    }

    return this.recordExpressionType(expression, materializeIntegerLiteral(operandType, fallback));
  }

  private checkBinaryExpression(expression: BinaryExpression, scope: Scope, expectedType?: IntKernelType): IntKernelType {
    if (isArithmeticOperator(expression.operator)) {
      return this.checkArithmeticExpression(expression, scope, expectedType);
    }

    if (isComparisonOperator(expression.operator)) {
      return this.checkComparisonExpression(expression, scope);
    }

    if (expression.operator === "&&" || expression.operator === "||") {
      const leftType = materializeIntegerLiteral(this.checkExpression(expression.left, scope));
      const rightType = materializeIntegerLiteral(this.checkExpression(expression.right, scope));
      if (!isUnknown(leftType) && !isBool(leftType)) {
        this.error(expression.left.span, `Logical operator '${expression.operator}' requires bool operands.`);
      }
      if (!isUnknown(rightType) && !isBool(rightType)) {
        this.error(expression.right.span, `Logical operator '${expression.operator}' requires bool operands.`);
      }
      return this.recordExpressionType(expression, primitiveType("bool"));
    }

    return this.recordExpressionType(expression, unknownType);
  }

  private checkArithmeticExpression(expression: BinaryExpression, scope: Scope, expectedType?: IntKernelType): IntKernelType {
    const leftRaw = this.checkExpression(expression.left, scope);
    const rightRaw = this.checkExpression(expression.right, scope);
    const fallback = integerLiteralFallback(expectedType);
    const leftType = materializeIntegerLiteral(leftRaw, rightRaw.kind === "integerLiteral" ? fallback : integerLiteralFallback(rightRaw));
    const rightType = materializeIntegerLiteral(rightRaw, integerLiteralFallback(leftType));

    if (expression.operator === "%" && (isFloatType(leftType) || isFloatType(rightType))) {
      this.error(expression.span, "Arithmetic operator '%' does not support f64 operands.");
      return this.recordExpressionType(expression, unknownType);
    }

    if (!isUnknown(leftType) && !isUnknown(rightType) && (!isNumericType(leftType) || !isNumericType(rightType) || !sameType(leftType, rightType))) {
      this.error(expression.span, `Arithmetic operator '${expression.operator}' requires integer operands of the same type.`);
      return this.recordExpressionType(expression, unknownType);
    }

    this.expressionTypes.set(expression.left, leftType);
    this.expressionTypes.set(expression.right, rightType);
    return this.recordExpressionType(expression, materializeIntegerLiteral(leftType, fallback));
  }

  private checkComparisonExpression(expression: BinaryExpression, scope: Scope): IntKernelType {
    const leftRaw = this.checkExpression(expression.left, scope);
    const rightRaw = this.checkExpression(expression.right, scope);
    const leftType = materializeIntegerLiteral(leftRaw, rightRaw.kind === "integerLiteral" ? primitiveType("i32") : integerLiteralFallback(rightRaw));
    const rightType = materializeIntegerLiteral(rightRaw, integerLiteralFallback(leftType));

    const valid =
      expression.operator === "==" || expression.operator === "!="
        ? sameType(leftType, rightType)
        : isNumericType(leftType) && isNumericType(rightType) && sameType(leftType, rightType);

    if (!isUnknown(leftType) && !isUnknown(rightType) && !valid) {
      this.error(expression.span, `Comparison operator '${expression.operator}' requires compatible operands.`);
    }

    this.expressionTypes.set(expression.left, leftType);
    this.expressionTypes.set(expression.right, rightType);
    return this.recordExpressionType(expression, primitiveType("bool"));
  }

  private checkCallExpression(expression: CallExpression, scope: Scope): IntKernelType {
    if (expression.callee.kind !== "IdentifierExpression") {
      this.error(expression.callee.span, "Can only call functions by name.");
      for (const arg of expression.args) {
        this.checkExpression(arg, scope);
      }
      return this.recordExpressionType(expression, unknownType);
    }

    const builtin = compilerBuiltins.get(expression.callee.name);
    if (builtin) {
      return this.checkCompilerBuiltinCall(expression, scope, builtin);
    }

    const functionSymbol = this.symbols.functions.get(expression.callee.name);
    if (!functionSymbol) {
      this.error(expression.callee.span, `Unknown function '${expression.callee.name}'.`);
      for (const arg of expression.args) {
        this.checkExpression(arg, scope);
      }
      return this.recordExpressionType(expression, unknownType);
    }

    this.recordExpressionType(expression.callee, functionSymbol.returnType);

    if (expression.args.length !== functionSymbol.params.length) {
      this.error(
        expression.span,
        `Function '${functionSymbol.name}' expects ${functionSymbol.params.length} argument${functionSymbol.params.length === 1 ? "" : "s"} but got ${expression.args.length}.`
      );
    }

    expression.args.forEach((arg, index) => {
      const expected = functionSymbol.params[index];
      const argType = this.checkExpression(arg, scope, expected);
      if (expected && !isUnknown(expected) && !isUnknown(argType) && !canAssign(expected, argType)) {
        this.error(arg.span, `Argument ${index + 1} of function '${functionSymbol.name}' expects ${typeToString(expected)} but got ${typeToString(argType)}.`);
      }
    });

    return this.recordExpressionType(expression, functionSymbol.returnType);
  }

  private checkCompilerBuiltinCall(expression: CallExpression, scope: Scope, builtin: CompilerBuiltin): IntKernelType {
    this.recordExpressionType(expression.callee, builtin.returnType);

    if (expression.args.length !== builtin.params.length) {
      this.error(
        expression.span,
        `Compiler builtin '${builtin.name}' expects ${builtin.params.length} argument${builtin.params.length === 1 ? "" : "s"} but got ${expression.args.length}.`
      );
    }

    expression.args.forEach((arg, index) => {
      const expected = builtin.params[index];
      const argType = this.checkExpression(arg, scope, expected);
      if (expected && !isUnknown(expected) && !isUnknown(argType) && !canAssign(expected, argType)) {
        this.error(arg.span, `Argument ${index + 1} of compiler builtin '${builtin.name}' expects ${typeToString(expected)} but got ${typeToString(argType)}.`);
      }
    });

    return this.recordExpressionType(expression, builtin.returnType);
  }

  private checkFieldExpression(expression: FieldExpression, scope: Scope): IntKernelType {
    const objectType = this.checkExpression(expression.object, scope);
    if (objectType.kind !== "struct") {
      if (!isUnknown(objectType)) {
        this.error(expression.object.span, `Field access requires struct value, got ${typeToString(objectType)}.`);
      }
      return this.recordExpressionType(expression, unknownType);
    }

    const structSymbol = this.symbols.structs.get(objectType.name);
    const fieldType = structSymbol?.fields.get(expression.field.name);
    if (!fieldType) {
      this.error(expression.field.span, `Struct '${objectType.name}' has no field '${expression.field.name}'.`);
      return this.recordExpressionType(expression, unknownType);
    }

    return this.recordExpressionType(expression, fieldType);
  }

  private checkIndexExpression(expression: IndexExpression, scope: Scope): IntKernelType {
    const objectType = this.checkExpression(expression.object, scope);
    const indexType = materializeIntegerLiteral(this.checkExpression(expression.index, scope), primitiveType("i32"));

    if (!isUnknown(indexType) && !isIndexInteger(indexType)) {
      this.error(expression.index.span, `Index expression requires i32 or u32 index, got ${typeToString(indexType)}.`);
    }

    if (objectType.kind !== "pointer") {
      if (!isUnknown(objectType)) {
        this.error(expression.object.span, `Index access requires pointer value, got ${typeToString(objectType)}.`);
      }
      return this.recordExpressionType(expression, unknownType);
    }

    return this.recordExpressionType(expression, objectType.elementType);
  }

  private checkParenthesizedExpression(expression: ParenthesizedExpression, scope: Scope, expectedType?: IntKernelType): IntKernelType {
    const type = this.checkExpression(expression.expression, scope, expectedType);
    return this.recordExpressionType(expression, type);
  }

  private resolveType(typeNode: TypeNode): IntKernelType {
    switch (typeNode.kind) {
      case "PrimitiveType":
        return primitiveType(typeNode.name as PrimitiveTypeName);
      case "PointerType":
        return pointerType(this.resolveType(typeNode.elementType));
      case "NamedType": {
        const name = typeNode.name.name;
        if (!this.symbols.structs.has(name)) {
          this.error(typeNode.name.span, `Unknown type '${name}'.`);
          return unknownType;
        }
        return structType(name);
      }
      case "ErrorType":
        return unknownType;
    }
  }

  private recordExpressionType<T extends Expression>(expression: T, type: IntKernelType): IntKernelType {
    this.expressionTypes.set(expression, type);
    return type;
  }

  private blockDefinitelyReturns(block: BlockStatement): boolean {
    const lastStatement = block.statements.at(-1);
    if (!lastStatement) {
      return false;
    }

    return this.statementDefinitelyReturns(lastStatement);
  }

  private statementDefinitelyReturns(statement: Statement): boolean {
    switch (statement.kind) {
      case "ReturnStatement":
        return true;
      case "BlockStatement":
        return this.blockDefinitelyReturns(statement);
      case "IfStatement":
        return Boolean(
          statement.elseBlock &&
            this.blockDefinitelyReturns(statement.thenBlock) &&
            this.blockDefinitelyReturns(statement.elseBlock)
        );
      case "LetStatement":
      case "AssignmentStatement":
      case "WhileStatement":
      case "ErrorStatement":
        return false;
    }
  }

  private isAssignableExpression(expression: Expression): boolean {
    return expression.kind === "IdentifierExpression" || expression.kind === "FieldExpression" || expression.kind === "IndexExpression";
  }

  private error(span: SourceSpan, message: string): void {
    this.diagnostics.push(errorAt(this.source, span, checkerDiagnosticCode(message), message));
  }
}

export function getExprType(checkedProgram: CheckedProgram, expression: Expression): IntKernelType | undefined {
  return checkedProgram.types.get(expression);
}

export function getLetType(checkedProgram: CheckedProgram, statement: LetStatement): IntKernelType | undefined {
  return checkedProgram.localTypes.get(statement);
}

export function getStructInfo(checkedProgram: CheckedProgram, name: string): StructInfo | undefined {
  return checkedProgram.structMap.get(name);
}

export function getFieldInfo(checkedProgram: CheckedProgram, structName: string, fieldName: string): StructFieldInfo | undefined {
  return checkedProgram.structMap.get(structName)?.fieldMap.get(fieldName);
}

export function getFunctionInfo(checkedProgram: CheckedProgram, name: string): FunctionInfo | undefined {
  return checkedProgram.functionMap.get(name);
}

function createCheckedProgram(
  ast: Program,
  symbols: SymbolTable,
  expressionTypes: TypeMap,
  localTypes: LetTypeMap
): CheckedProgram {
  const structs = [...symbols.structs.values()].map(toStructInfo);
  const functions = [...symbols.functions.values()].map(toFunctionInfo);

  return {
    ast,
    symbols,
    types: new Map(expressionTypes),
    localTypes: new Map(localTypes),
    structs,
    functions,
    structMap: new Map(structs.map((struct) => [struct.name, struct])),
    functionMap: new Map(functions.map((func) => [func.name, func]))
  };
}

function toStructInfo(symbol: StructSymbol): StructInfo {
  const fields = [...symbol.fields.entries()].map(([name, type]) => {
    const declaration = symbol.declaration.fields.find((field) => field.name.name === name);
    if (!declaration) {
      throw new Error(`Checker invariant violation: missing declaration for field '${symbol.name}.${name}'.`);
    }
    return { name, type, declaration };
  });

  return {
    name: symbol.name,
    declaration: symbol.declaration,
    fields,
    fieldMap: new Map(fields.map((field) => [field.name, field]))
  };
}

function toFunctionInfo(symbol: FunctionSymbol): FunctionInfo {
  const params = symbol.declaration.params.map((param, index) => ({
    name: param.name.name,
    type: symbol.params[index] ?? unknownType,
    declaration: param
  }));

  return {
    name: symbol.name,
    exported: symbol.declaration.exported,
    declaration: symbol.declaration,
    params,
    returnType: symbol.returnType
  };
}

function checkerDiagnosticCode(message: string): DiagnosticCode {
  if (message.startsWith("Unknown variable")) {
    return "IK2001";
  }
  if (message.startsWith("Unknown function")) {
    return "IK2002";
  }
  if (message.startsWith("Unknown type")) {
    return "IK2003";
  }
  if (message.startsWith("Duplicate")) {
    return "IK2005";
  }
  if (message.startsWith("If condition") || message.startsWith("While condition")) {
    return "IK2006";
  }
  if (message.startsWith("Invalid assignment target")) {
    return "IK2007";
  }
  if (message.startsWith("Missing return")) {
    return "IK2008";
  }

  return "IK2004";
}

function integerLiteralFallback(type?: IntKernelType): IntKernelType {
  return type?.kind === "primitive" && isInteger(type) ? type : primitiveType("i32");
}

function isArithmeticOperator(operator: string): boolean {
  return operator === "+" || operator === "-" || operator === "*" || operator === "/" || operator === "%";
}

function isComparisonOperator(operator: string): boolean {
  return operator === "==" || operator === "!=" || operator === "<" || operator === "<=" || operator === ">" || operator === ">=";
}
