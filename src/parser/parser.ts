import { lex } from "../lexer/lexer.js";
import { type Token, TokenKind } from "../lexer/token.js";
import { errorAt, type Diagnostic } from "../source/diagnostics.js";
import { SourceFile, type SourcePosition, type SourceSpan } from "../source/source-file.js";
import type {
  AssignmentStatement,
  BinaryExpression,
  BlockStatement,
  BoolLiteral,
  CallExpression,
  Declaration,
  ErrorExpression,
  ErrorStatement,
  ErrorTypeNode,
  Expression,
  FieldExpression,
  FunctionDeclaration,
  FunctionParam,
  IdentifierExpression,
  IdentifierNode,
  IfStatement,
  IndexExpression,
  IntegerLiteral,
  LetStatement,
  NamedTypeNode,
  ParenthesizedExpression,
  PointerTypeNode,
  PrimitiveTypeNode,
  Program,
  ReturnStatement,
  Statement,
  StructDeclaration,
  StructField,
  TypeNode,
  UnaryExpression,
  WhileStatement
} from "./ast.js";

export interface ParseResult {
  ast: Program;
  diagnostics: Diagnostic[];
}

export function parse(source: SourceFile): ParseResult {
  const lexResult = lex(source);
  const parser = new Parser(source, lexResult.tokens, [...lexResult.diagnostics]);
  return parser.parse();
}

class Parser {
  private index = 0;

  constructor(
    private readonly source: SourceFile,
    private readonly tokens: Token[],
    private readonly diagnostics: Diagnostic[]
  ) {}

  parse(): ParseResult {
    const start = this.positionFromToken(this.current());
    const declarations: Declaration[] = [];

    while (!this.check(TokenKind.Eof)) {
      const declaration = this.parseDeclaration();
      if (declaration) {
        declarations.push(declaration);
      }
    }

    const end = this.positionFromToken(this.current());
    return {
      ast: {
        kind: "Program",
        declarations,
        span: { start, end }
      },
      diagnostics: this.diagnostics
    };
  }

  private parseDeclaration(): Declaration | null {
    if (this.check(TokenKind.Struct)) {
      return this.parseStructDeclaration();
    }

    if (this.check(TokenKind.Export) || this.check(TokenKind.Fn)) {
      return this.parseFunctionDeclaration();
    }

    this.error(this.current(), "Expected declaration.");
    this.advance();
    return null;
  }

  private parseStructDeclaration(): StructDeclaration {
    const structToken = this.consume(TokenKind.Struct, "Expected 'struct'.");
    const name = this.parseIdentifier("Expected struct name.");
    this.consume(TokenKind.LeftBrace, "Expected '{' after struct name.");

    const fields: StructField[] = [];
    while (!this.check(TokenKind.RightBrace) && !this.check(TokenKind.Eof)) {
      const fieldStart = this.current();
      const fieldName = this.parseIdentifier("Expected field name.");
      this.consume(TokenKind.Colon, "Expected ':' after field name.");
      const fieldType = this.parseType();
      const semicolon = this.consume(TokenKind.Semicolon, "Expected ';' after struct field.");
      fields.push({
        kind: "StructField",
        name: fieldName,
        type: fieldType,
        span: this.spanBetweenTokens(fieldStart, semicolon)
      });
    }

    const end = this.consume(TokenKind.RightBrace, "Expected '}' after struct fields.");
    return {
      kind: "StructDeclaration",
      name,
      fields,
      span: this.spanBetweenTokens(structToken, end)
    };
  }

  private parseFunctionDeclaration(): FunctionDeclaration {
    const startToken = this.match(TokenKind.Export) ? this.previous() : this.current();
    const exported = startToken.kind === TokenKind.Export;
    this.consume(TokenKind.Fn, "Expected 'fn' after 'export'.");
    const name = this.parseIdentifier("Expected function name.");
    this.consume(TokenKind.LeftParen, "Expected '(' after function name.");

    const params: FunctionParam[] = [];
    if (!this.check(TokenKind.RightParen)) {
      do {
        params.push(this.parseFunctionParam());
      } while (this.match(TokenKind.Comma));
    }

    this.consume(TokenKind.RightParen, "Expected ')' after parameters.");
    this.consume(TokenKind.Arrow, "Expected '->' before return type.");
    const returnType = this.parseType();
    const body = this.parseBlockStatement();

    return {
      kind: "FunctionDeclaration",
      exported,
      name,
      params,
      returnType,
      body,
      span: this.spanFromPositions(this.positionFromToken(startToken), body.span.end)
    };
  }

  private parseFunctionParam(): FunctionParam {
    const start = this.current();
    const name = this.parseIdentifier("Expected parameter name.");
    this.consume(TokenKind.Colon, "Expected ':' after parameter name.");
    const type = this.parseType();
    return {
      kind: "FunctionParam",
      name,
      type,
      span: this.spanFromPositions(this.positionFromToken(start), type.span.end)
    };
  }

  private parseType(): TypeNode {
    const token = this.current();
    switch (token.kind) {
      case TokenKind.I32:
      case TokenKind.I64:
      case TokenKind.U32:
      case TokenKind.U64:
      case TokenKind.Bool:
        this.advance();
        return {
          kind: "PrimitiveType",
          name: token.text as PrimitiveTypeNode["name"],
          span: this.spanFromToken(token)
        };
      case TokenKind.Identifier: {
        const name = this.parseIdentifier("Expected type name.");
        return {
          kind: "NamedType",
          name,
          span: name.span
        } satisfies NamedTypeNode;
      }
      case TokenKind.Ptr: {
        const ptrToken = this.advance();
        this.consume(TokenKind.Less, "Expected '<' after 'ptr'.");
        const elementType = this.parseType();
        const greater = this.consume(TokenKind.Greater, "Expected '>' after pointer type.");
        return {
          kind: "PointerType",
          elementType,
          span: this.spanBetweenTokens(ptrToken, greater)
        } satisfies PointerTypeNode;
      }
      default: {
        this.error(token, "Expected type.");
        this.advance();
        return {
          kind: "ErrorType",
          span: this.spanFromToken(token)
        } satisfies ErrorTypeNode;
      }
    }
  }

  private parseBlockStatement(): BlockStatement {
    const leftBrace = this.consume(TokenKind.LeftBrace, "Expected '{' before block.");
    const statements: Statement[] = [];

    while (!this.check(TokenKind.RightBrace) && !this.check(TokenKind.Eof)) {
      statements.push(this.parseStatement());
    }

    const rightBrace = this.consume(TokenKind.RightBrace, "Expected '}' after block.");
    return {
      kind: "BlockStatement",
      statements,
      span: this.spanBetweenTokens(leftBrace, rightBrace)
    };
  }

  private parseStatement(): Statement {
    if (this.check(TokenKind.LeftBrace)) {
      return this.parseBlockStatement();
    }
    if (this.check(TokenKind.Let)) {
      return this.parseLetStatement();
    }
    if (this.check(TokenKind.Return)) {
      return this.parseReturnStatement();
    }
    if (this.check(TokenKind.If)) {
      return this.parseIfStatement();
    }
    if (this.check(TokenKind.While)) {
      return this.parseWhileStatement();
    }

    return this.parseAssignmentStatement();
  }

  private parseLetStatement(): LetStatement {
    const letToken = this.consume(TokenKind.Let, "Expected 'let'.");
    const name = this.parseIdentifier("Expected local name.");
    this.consume(TokenKind.Colon, "Expected ':' after local name.");
    const type = this.parseType();
    this.consume(TokenKind.Equal, "Expected '=' after local type.");
    const initializer = this.parseExpression();
    const semicolon = this.consume(TokenKind.Semicolon, "Expected ';' after let statement.");

    return {
      kind: "LetStatement",
      name,
      type,
      initializer,
      span: this.spanBetweenTokens(letToken, semicolon)
    };
  }

  private parseAssignmentStatement(): Statement {
    const start = this.current();
    const target = this.parseExpression();

    if (!this.match(TokenKind.Equal)) {
      this.error(this.current(), "Expected '=' in assignment statement.");
      this.synchronizeStatement();
      return {
        kind: "ErrorStatement",
        span: this.spanFromToken(start)
      } satisfies ErrorStatement;
    }

    const value = this.parseExpression();
    const semicolon = this.consume(TokenKind.Semicolon, "Expected ';' after assignment statement.");
    return {
      kind: "AssignmentStatement",
      target,
      value,
      span: this.spanFromPositions(target.span.start, this.endPositionFromToken(semicolon))
    } satisfies AssignmentStatement;
  }

  private parseReturnStatement(): ReturnStatement {
    const returnToken = this.consume(TokenKind.Return, "Expected 'return'.");
    const value = this.parseExpression();
    const semicolon = this.consume(TokenKind.Semicolon, "Expected ';' after return statement.");
    return {
      kind: "ReturnStatement",
      value,
      span: this.spanBetweenTokens(returnToken, semicolon)
    };
  }

  private parseIfStatement(): IfStatement {
    const ifToken = this.consume(TokenKind.If, "Expected 'if'.");
    const condition = this.parseExpression();
    const thenBlock = this.parseBlockStatement();
    const elseBlock = this.match(TokenKind.Else) ? this.parseBlockStatement() : null;
    return {
      kind: "IfStatement",
      condition,
      thenBlock,
      elseBlock,
      span: this.spanFromPositions(this.positionFromToken(ifToken), (elseBlock ?? thenBlock).span.end)
    };
  }

  private parseWhileStatement(): WhileStatement {
    const whileToken = this.consume(TokenKind.While, "Expected 'while'.");
    const condition = this.parseExpression();
    const body = this.parseBlockStatement();
    return {
      kind: "WhileStatement",
      condition,
      body,
      span: this.spanFromPositions(this.positionFromToken(whileToken), body.span.end)
    };
  }

  private parseExpression(minPrecedence = 1): Expression {
    let left = this.parseUnaryExpression();

    while (true) {
      const operator = this.current();
      const precedence = binaryPrecedence(operator.kind);
      if (precedence < minPrecedence) {
        break;
      }

      this.advance();
      const right = this.parseExpression(precedence + 1);
      left = {
        kind: "BinaryExpression",
        operator: operator.text,
        left,
        right,
        span: this.spanFromPositions(left.span.start, right.span.end)
      } satisfies BinaryExpression;
    }

    return left;
  }

  private parseUnaryExpression(): Expression {
    if (this.check(TokenKind.Bang) || this.check(TokenKind.Minus)) {
      const operator = this.advance();
      const operand = this.parseExpression(7);
      return {
        kind: "UnaryExpression",
        operator: operator.text as UnaryExpression["operator"],
        operand,
        span: this.spanFromPositions(this.positionFromToken(operator), operand.span.end)
      } satisfies UnaryExpression;
    }

    return this.parsePostfixExpression(this.parsePrimaryExpression());
  }

  private parsePostfixExpression(base: Expression): Expression {
    let expression = base;

    while (true) {
      if (this.match(TokenKind.LeftParen)) {
        const args: Expression[] = [];
        if (!this.check(TokenKind.RightParen)) {
          do {
            args.push(this.parseExpression());
          } while (this.match(TokenKind.Comma));
        }
        const rightParen = this.consume(TokenKind.RightParen, "Expected ')' after arguments.");
        expression = {
          kind: "CallExpression",
          callee: expression,
          args,
          span: this.spanFromPositions(expression.span.start, this.endPositionFromToken(rightParen))
        } satisfies CallExpression;
        continue;
      }

      if (this.match(TokenKind.Dot)) {
        const field = this.parseIdentifier("Expected field name after '.'.");
        expression = {
          kind: "FieldExpression",
          object: expression,
          field,
          span: this.spanFromPositions(expression.span.start, field.span.end)
        } satisfies FieldExpression;
        continue;
      }

      if (this.match(TokenKind.LeftBracket)) {
        const index = this.parseExpression();
        const rightBracket = this.consume(TokenKind.RightBracket, "Expected ']' after index expression.");
        expression = {
          kind: "IndexExpression",
          object: expression,
          index,
          span: this.spanFromPositions(expression.span.start, this.endPositionFromToken(rightBracket))
        } satisfies IndexExpression;
        continue;
      }

      return expression;
    }
  }

  private parsePrimaryExpression(): Expression {
    const token = this.current();

    if (this.match(TokenKind.Integer)) {
      return {
        kind: "IntegerLiteral",
        text: token.text,
        span: this.spanFromToken(token)
      } satisfies IntegerLiteral;
    }

    if (this.match(TokenKind.True) || this.match(TokenKind.False)) {
      return {
        kind: "BoolLiteral",
        value: token.kind === TokenKind.True,
        span: this.spanFromToken(token)
      } satisfies BoolLiteral;
    }

    if (this.match(TokenKind.Identifier)) {
      return {
        kind: "IdentifierExpression",
        name: token.text,
        span: this.spanFromToken(token)
      } satisfies IdentifierExpression;
    }

    if (this.match(TokenKind.LeftParen)) {
      const expression = this.parseExpression();
      const rightParen = this.consume(TokenKind.RightParen, "Expected ')' after expression.");
      return {
        kind: "ParenthesizedExpression",
        expression,
        span: this.spanFromPositions(this.positionFromToken(token), this.endPositionFromToken(rightParen))
      } satisfies ParenthesizedExpression;
    }

    this.error(token, "Expected expression.");
    this.advance();
    return {
      kind: "ErrorExpression",
      span: this.spanFromToken(token)
    } satisfies ErrorExpression;
  }

  private parseIdentifier(message: string): IdentifierNode {
    const token = this.consume(TokenKind.Identifier, message);
    return {
      kind: "Identifier",
      name: token.text,
      span: this.spanFromToken(token)
    };
  }

  private match(kind: TokenKind): boolean {
    if (!this.check(kind)) {
      return false;
    }
    this.advance();
    return true;
  }

  private consume(kind: TokenKind, message: string): Token {
    if (this.check(kind)) {
      return this.advance();
    }

    const token = this.current();
    this.error(token, message);
    return {
      kind,
      text: "",
      line: token.line,
      column: token.column,
      start: token.start,
      end: token.start
    };
  }

  private check(kind: TokenKind): boolean {
    return this.current().kind === kind;
  }

  private advance(): Token {
    const token = this.current();
    if (!this.check(TokenKind.Eof)) {
      this.index += 1;
    }
    return token;
  }

  private previous(): Token {
    return this.tokens[Math.max(0, this.index - 1)] ?? this.current();
  }

  private current(): Token {
    return this.tokens[this.index] ?? this.tokens[this.tokens.length - 1]!;
  }

  private error(token: Token, message: string): void {
    this.diagnostics.push(errorAt(this.source, this.spanFromToken(token), "IK1001", message));
  }

  private synchronizeStatement(): void {
    while (!this.check(TokenKind.Eof)) {
      if (this.match(TokenKind.Semicolon)) {
        return;
      }
      if (this.check(TokenKind.RightBrace)) {
        return;
      }
      this.advance();
    }
  }

  private spanFromToken(token: Token): SourceSpan {
    return {
      start: this.positionFromToken(token),
      end: this.endPositionFromToken(token)
    };
  }

  private spanBetweenTokens(start: Token, end: Token): SourceSpan {
    return {
      start: this.positionFromToken(start),
      end: this.endPositionFromToken(end)
    };
  }

  private spanFromPositions(start: SourcePosition, end: SourcePosition): SourceSpan {
    return { start, end };
  }

  private positionFromToken(token: Token): SourcePosition {
    return {
      offset: token.start,
      line: token.line,
      column: token.column
    };
  }

  private endPositionFromToken(token: Token): SourcePosition {
    return {
      offset: token.end,
      line: token.line,
      column: token.column + token.text.length
    };
  }
}

function binaryPrecedence(kind: TokenKind): number {
  switch (kind) {
    case TokenKind.PipePipe:
      return 1;
    case TokenKind.AmpAmp:
      return 2;
    case TokenKind.EqualEqual:
    case TokenKind.BangEqual:
      return 3;
    case TokenKind.Less:
    case TokenKind.LessEqual:
    case TokenKind.Greater:
    case TokenKind.GreaterEqual:
      return 4;
    case TokenKind.Plus:
    case TokenKind.Minus:
      return 5;
    case TokenKind.Star:
    case TokenKind.Slash:
    case TokenKind.Percent:
      return 6;
    default:
      return 0;
  }
}
