import { errorAt, type Diagnostic } from "../source/diagnostics.js";
import { SourceFile, type SourcePosition } from "../source/source-file.js";
import { keywords, type Token, TokenKind } from "./token.js";

export interface LexResult {
  tokens: Token[];
  diagnostics: Diagnostic[];
}

export function lex(source: SourceFile): LexResult {
  return new Lexer(source).lex();
}

class Lexer {
  private readonly tokens: Token[] = [];
  private readonly diagnostics: Diagnostic[] = [];
  private offset = 0;
  private line = 1;
  private column = 1;

  constructor(private readonly source: SourceFile) {}

  lex(): LexResult {
    while (!this.isAtEnd()) {
      this.scanToken();
    }

    const position = this.position();
    this.tokens.push({
      kind: TokenKind.Eof,
      text: "",
      line: position.line,
      column: position.column,
      start: position.offset,
      end: position.offset
    });

    return {
      tokens: this.tokens,
      diagnostics: this.diagnostics
    };
  }

  private scanToken(): void {
    const char = this.peek();

    if (isWhitespace(char)) {
      this.advance();
      return;
    }

    if (char === "/" && this.peekNext() === "/") {
      this.skipLineComment();
      return;
    }

    if (isIdentifierStart(char)) {
      this.scanIdentifierOrKeyword();
      return;
    }

    if (isDigit(char)) {
      this.scanNumber();
      return;
    }

    const start = this.position();

    switch (char) {
      case "(":
        this.advance();
        this.addToken(TokenKind.LeftParen, start);
        return;
      case ")":
        this.advance();
        this.addToken(TokenKind.RightParen, start);
        return;
      case "{":
        this.advance();
        this.addToken(TokenKind.LeftBrace, start);
        return;
      case "}":
        this.advance();
        this.addToken(TokenKind.RightBrace, start);
        return;
      case "[":
        this.advance();
        this.addToken(TokenKind.LeftBracket, start);
        return;
      case "]":
        this.advance();
        this.addToken(TokenKind.RightBracket, start);
        return;
      case ",":
        this.advance();
        this.addToken(TokenKind.Comma, start);
        return;
      case ":":
        this.advance();
        this.addToken(TokenKind.Colon, start);
        return;
      case ";":
        this.advance();
        this.addToken(TokenKind.Semicolon, start);
        return;
      case ".":
        if (isDigit(this.peekNext())) {
          this.scanMalformedFloatStartingWithDot();
          return;
        }
        this.advance();
        this.addToken(TokenKind.Dot, start);
        return;
      case "+":
        this.advance();
        this.addToken(TokenKind.Plus, start);
        return;
      case "-":
        this.advance();
        this.addToken(this.match(">") ? TokenKind.Arrow : TokenKind.Minus, start);
        return;
      case "*":
        this.advance();
        this.addToken(TokenKind.Star, start);
        return;
      case "/":
        this.advance();
        this.addToken(TokenKind.Slash, start);
        return;
      case "%":
        this.advance();
        this.addToken(TokenKind.Percent, start);
        return;
      case "=":
        this.advance();
        this.addToken(this.match("=") ? TokenKind.EqualEqual : TokenKind.Equal, start);
        return;
      case "!":
        this.advance();
        this.addToken(this.match("=") ? TokenKind.BangEqual : TokenKind.Bang, start);
        return;
      case "<":
        this.advance();
        this.addToken(this.match("=") ? TokenKind.LessEqual : TokenKind.Less, start);
        return;
      case ">":
        this.advance();
        this.addToken(this.match("=") ? TokenKind.GreaterEqual : TokenKind.Greater, start);
        return;
      case "&":
        this.advance();
        if (this.match("&")) {
          this.addToken(TokenKind.AmpAmp, start);
        } else {
          this.reportUnexpected(start, char);
        }
        return;
      case "|":
        this.advance();
        if (this.match("|")) {
          this.addToken(TokenKind.PipePipe, start);
        } else {
          this.reportUnexpected(start, char);
        }
        return;
      default:
        this.advance();
        this.reportUnexpected(start, char);
    }
  }

  private scanIdentifierOrKeyword(): void {
    const start = this.position();
    this.advance();

    while (!this.isAtEnd() && isIdentifierPart(this.peek())) {
      this.advance();
    }

    const text = this.source.text.slice(start.offset, this.offset);
    this.addToken(keywords.get(text) ?? TokenKind.Identifier, start);
  }

  private scanNumber(): void {
    const start = this.position();
    this.advance();

    while (!this.isAtEnd() && isDigit(this.peek())) {
      this.advance();
    }

    let isFloat = false;

    if (this.peek() === ".") {
      isFloat = true;
      this.advance();
      if (!isDigit(this.peek())) {
        this.reportMalformedFloat(start);
        return;
      }

      while (!this.isAtEnd() && isDigit(this.peek())) {
        this.advance();
      }
    }

    if (isExponentStart(this.peek())) {
      isFloat = true;
      this.advance();
      if (this.peek() === "+" || this.peek() === "-") {
        this.advance();
      }

      if (!isDigit(this.peek())) {
        this.reportMalformedFloat(start);
        return;
      }

      while (!this.isAtEnd() && isDigit(this.peek())) {
        this.advance();
      }
    }

    this.addToken(isFloat ? TokenKind.Float : TokenKind.Integer, start);
  }

  private scanMalformedFloatStartingWithDot(): void {
    const start = this.position();
    this.advance();

    while (!this.isAtEnd() && isDigit(this.peek())) {
      this.advance();
    }

    this.reportMalformedFloat(start);
  }

  private skipLineComment(): void {
    while (!this.isAtEnd() && this.peek() !== "\n") {
      this.advance();
    }
  }

  private addToken(kind: TokenKind, start: SourcePosition): void {
    this.tokens.push({
      kind,
      text: this.source.text.slice(start.offset, this.offset),
      line: start.line,
      column: start.column,
      start: start.offset,
      end: this.offset
    });
  }

  private reportUnexpected(start: SourcePosition, char: string): void {
    this.diagnostics.push(errorAt(this.source, { start, end: this.position() }, "CK0001", `Unexpected character '${char}'.`));
  }

  private reportMalformedFloat(start: SourcePosition): void {
    const text = this.source.text.slice(start.offset, this.offset);
    this.diagnostics.push(errorAt(this.source, { start, end: this.position() }, "CK0001", `Malformed float literal '${text}'.`));
  }

  private match(expected: string): boolean {
    if (this.isAtEnd() || this.peek() !== expected) {
      return false;
    }

    this.advance();
    return true;
  }

  private advance(): string {
    const char = this.source.text[this.offset] ?? "";
    this.offset += 1;

    if (char === "\n") {
      this.line += 1;
      this.column = 1;
    } else {
      this.column += 1;
    }

    return char;
  }

  private peek(): string {
    return this.source.text[this.offset] ?? "";
  }

  private peekNext(): string {
    return this.source.text[this.offset + 1] ?? "";
  }

  private isAtEnd(): boolean {
    return this.offset >= this.source.text.length;
  }

  private position(): SourcePosition {
    return {
      offset: this.offset,
      line: this.line,
      column: this.column
    };
  }
}

function isWhitespace(char: string): boolean {
  return char === " " || char === "\r" || char === "\t" || char === "\n";
}

function isDigit(char: string): boolean {
  return char >= "0" && char <= "9";
}

function isExponentStart(char: string): boolean {
  return char === "e" || char === "E";
}

function isIdentifierStart(char: string): boolean {
  return (char >= "A" && char <= "Z") || (char >= "a" && char <= "z") || char === "_";
}

function isIdentifierPart(char: string): boolean {
  return isIdentifierStart(char) || isDigit(char);
}
