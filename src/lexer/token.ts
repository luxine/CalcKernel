export enum TokenKind {
  Eof = "Eof",

  Identifier = "Identifier",
  Integer = "Integer",

  Struct = "Struct",
  Export = "Export",
  Fn = "Fn",
  Let = "Let",
  Return = "Return",
  If = "If",
  Else = "Else",
  While = "While",
  True = "True",
  False = "False",

  I32 = "I32",
  I64 = "I64",
  U32 = "U32",
  U64 = "U64",
  Bool = "Bool",
  Ptr = "Ptr",

  LeftParen = "LeftParen",
  RightParen = "RightParen",
  LeftBrace = "LeftBrace",
  RightBrace = "RightBrace",
  LeftBracket = "LeftBracket",
  RightBracket = "RightBracket",
  Comma = "Comma",
  Colon = "Colon",
  Semicolon = "Semicolon",
  Dot = "Dot",
  Arrow = "Arrow",

  Plus = "Plus",
  Minus = "Minus",
  Star = "Star",
  Slash = "Slash",
  Percent = "Percent",
  Equal = "Equal",
  EqualEqual = "EqualEqual",
  Bang = "Bang",
  BangEqual = "BangEqual",
  Less = "Less",
  LessEqual = "LessEqual",
  Greater = "Greater",
  GreaterEqual = "GreaterEqual",
  AmpAmp = "AmpAmp",
  PipePipe = "PipePipe"
}

export interface Token {
  kind: TokenKind;
  text: string;
  line: number;
  column: number;
  start: number;
  end: number;
}

export const keywords: ReadonlyMap<string, TokenKind> = new Map([
  ["struct", TokenKind.Struct],
  ["export", TokenKind.Export],
  ["fn", TokenKind.Fn],
  ["let", TokenKind.Let],
  ["return", TokenKind.Return],
  ["if", TokenKind.If],
  ["else", TokenKind.Else],
  ["while", TokenKind.While],
  ["true", TokenKind.True],
  ["false", TokenKind.False],
  ["i32", TokenKind.I32],
  ["i64", TokenKind.I64],
  ["u32", TokenKind.U32],
  ["u64", TokenKind.U64],
  ["bool", TokenKind.Bool],
  ["ptr", TokenKind.Ptr]
]);
