import type { SourceSpan } from "../source/source-file.js";

export interface AstNode {
  kind: string;
  span: SourceSpan;
}

export interface IdentifierNode extends AstNode {
  kind: "Identifier";
  name: string;
}

export interface Program extends AstNode {
  kind: "Program";
  declarations: Declaration[];
}

export type Declaration = StructDeclaration | FunctionDeclaration;

export interface StructDeclaration extends AstNode {
  kind: "StructDeclaration";
  name: IdentifierNode;
  fields: StructField[];
}

export interface StructField extends AstNode {
  kind: "StructField";
  name: IdentifierNode;
  type: TypeNode;
}

export interface FunctionDeclaration extends AstNode {
  kind: "FunctionDeclaration";
  exported: boolean;
  name: IdentifierNode;
  params: FunctionParam[];
  returnType: TypeNode;
  body: BlockStatement;
}

export interface FunctionParam extends AstNode {
  kind: "FunctionParam";
  name: IdentifierNode;
  type: TypeNode;
}

export type TypeNode = PrimitiveTypeNode | PointerTypeNode | NamedTypeNode | ErrorTypeNode;

export interface PrimitiveTypeNode extends AstNode {
  kind: "PrimitiveType";
  name: "i32" | "i64" | "u32" | "u64" | "f64" | "bool";
}

export interface PointerTypeNode extends AstNode {
  kind: "PointerType";
  elementType: TypeNode;
}

export interface NamedTypeNode extends AstNode {
  kind: "NamedType";
  name: IdentifierNode;
}

export interface ErrorTypeNode extends AstNode {
  kind: "ErrorType";
}

export type Statement =
  | BlockStatement
  | LetStatement
  | AssignmentStatement
  | ReturnStatement
  | IfStatement
  | WhileStatement
  | ErrorStatement;

export interface BlockStatement extends AstNode {
  kind: "BlockStatement";
  statements: Statement[];
}

export interface LetStatement extends AstNode {
  kind: "LetStatement";
  name: IdentifierNode;
  type: TypeNode;
  initializer: Expression;
}

export interface AssignmentStatement extends AstNode {
  kind: "AssignmentStatement";
  target: Expression;
  value: Expression;
}

export interface ReturnStatement extends AstNode {
  kind: "ReturnStatement";
  value: Expression;
}

export interface IfStatement extends AstNode {
  kind: "IfStatement";
  condition: Expression;
  thenBlock: BlockStatement;
  elseBlock: BlockStatement | null;
}

export interface WhileStatement extends AstNode {
  kind: "WhileStatement";
  condition: Expression;
  body: BlockStatement;
}

export interface ErrorStatement extends AstNode {
  kind: "ErrorStatement";
}

export type Expression =
  | IdentifierExpression
  | IntegerLiteral
  | FloatLiteral
  | BoolLiteral
  | UnaryExpression
  | BinaryExpression
  | CallExpression
  | FieldExpression
  | IndexExpression
  | ParenthesizedExpression
  | ErrorExpression;

export interface IdentifierExpression extends AstNode {
  kind: "IdentifierExpression";
  name: string;
}

export interface IntegerLiteral extends AstNode {
  kind: "IntegerLiteral";
  text: string;
}

export interface FloatLiteral extends AstNode {
  kind: "FloatLiteral";
  text: string;
}

export interface BoolLiteral extends AstNode {
  kind: "BoolLiteral";
  value: boolean;
}

export interface UnaryExpression extends AstNode {
  kind: "UnaryExpression";
  operator: "!" | "-";
  operand: Expression;
}

export interface BinaryExpression extends AstNode {
  kind: "BinaryExpression";
  operator: string;
  left: Expression;
  right: Expression;
}

export interface CallExpression extends AstNode {
  kind: "CallExpression";
  callee: Expression;
  args: Expression[];
}

export interface FieldExpression extends AstNode {
  kind: "FieldExpression";
  object: Expression;
  field: IdentifierNode;
}

export interface IndexExpression extends AstNode {
  kind: "IndexExpression";
  object: Expression;
  index: Expression;
}

export interface ParenthesizedExpression extends AstNode {
  kind: "ParenthesizedExpression";
  expression: Expression;
}

export interface ErrorExpression extends AstNode {
  kind: "ErrorExpression";
}
