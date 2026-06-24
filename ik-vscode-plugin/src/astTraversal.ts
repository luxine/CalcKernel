import type {
  BinaryExpression,
  BlockStatement,
  CallExpression,
  Declaration,
  Expression,
  FunctionDeclaration,
  IfStatement,
  IndexExpression,
  Program,
  Statement,
  UnaryExpression,
  WhileStatement
} from "intkernel";
import type { SourcePosition, SourceSpan } from "intkernel";

export interface ZeroBasedPosition {
  line: number;
  character: number;
}

export interface AstVisitor {
  declaration?: (node: Declaration) => void;
  statement?: (node: Statement) => void;
  expression?: (node: Expression) => void;
}

export function walkProgram(program: Program, visitor: AstVisitor): void {
  for (const declaration of program.declarations) {
    visitor.declaration?.(declaration);
    if (declaration.kind === "FunctionDeclaration") {
      walkFunction(declaration, visitor);
    }
  }
}

export function containsPosition(span: SourceSpan, position: ZeroBasedPosition): boolean {
  const start = toZeroBased(span.start);
  const end = toZeroBased(span.end);
  if (position.line < start.line || position.line > end.line) {
    return false;
  }
  if (position.line === start.line && position.character < start.character) {
    return false;
  }
  if (position.line === end.line && position.character >= end.character) {
    return false;
  }
  return true;
}

export function toZeroBased(position: SourcePosition): ZeroBasedPosition {
  return {
    line: Math.max(0, position.line - 1),
    character: Math.max(0, position.column - 1)
  };
}

function walkFunction(declaration: FunctionDeclaration, visitor: AstVisitor): void {
  walkBlock(declaration.body, visitor);
}

function walkBlock(block: BlockStatement, visitor: AstVisitor): void {
  for (const statement of block.statements) {
    visitor.statement?.(statement);
    walkStatement(statement, visitor);
  }
}

function walkStatement(statement: Statement, visitor: AstVisitor): void {
  switch (statement.kind) {
    case "BlockStatement":
      walkBlock(statement, visitor);
      return;
    case "LetStatement":
      walkExpression(statement.initializer, visitor);
      return;
    case "AssignmentStatement":
      walkExpression(statement.target, visitor);
      walkExpression(statement.value, visitor);
      return;
    case "ReturnStatement":
      walkExpression(statement.value, visitor);
      return;
    case "IfStatement":
      walkIfStatement(statement, visitor);
      return;
    case "WhileStatement":
      walkWhileStatement(statement, visitor);
      return;
    case "ErrorStatement":
      return;
  }
}

function walkIfStatement(statement: IfStatement, visitor: AstVisitor): void {
  walkExpression(statement.condition, visitor);
  walkBlock(statement.thenBlock, visitor);
  if (statement.elseBlock) {
    walkBlock(statement.elseBlock, visitor);
  }
}

function walkWhileStatement(statement: WhileStatement, visitor: AstVisitor): void {
  walkExpression(statement.condition, visitor);
  walkBlock(statement.body, visitor);
}

function walkExpression(expression: Expression, visitor: AstVisitor): void {
  visitor.expression?.(expression);
  switch (expression.kind) {
    case "UnaryExpression":
      walkUnaryExpression(expression, visitor);
      return;
    case "BinaryExpression":
      walkBinaryExpression(expression, visitor);
      return;
    case "CallExpression":
      walkCallExpression(expression, visitor);
      return;
    case "FieldExpression":
      walkExpression(expression.object, visitor);
      return;
    case "IndexExpression":
      walkIndexExpression(expression, visitor);
      return;
    case "ParenthesizedExpression":
      walkExpression(expression.expression, visitor);
      return;
    case "IdentifierExpression":
    case "IntegerLiteral":
    case "BoolLiteral":
    case "ErrorExpression":
      return;
  }
}

function walkUnaryExpression(expression: UnaryExpression, visitor: AstVisitor): void {
  walkExpression(expression.operand, visitor);
}

function walkBinaryExpression(expression: BinaryExpression, visitor: AstVisitor): void {
  walkExpression(expression.left, visitor);
  walkExpression(expression.right, visitor);
}

function walkCallExpression(expression: CallExpression, visitor: AstVisitor): void {
  walkExpression(expression.callee, visitor);
  for (const arg of expression.args) {
    walkExpression(arg, visitor);
  }
}

function walkIndexExpression(expression: IndexExpression, visitor: AstVisitor): void {
  walkExpression(expression.object, visitor);
  walkExpression(expression.index, visitor);
}
