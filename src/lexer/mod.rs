use crate::{Diagnostic, DiagnosticCode, SourceFile, SourcePosition, SourceSpan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Eof,

    Identifier,
    Integer,
    Float,

    Struct,
    Export,
    Fn,
    Let,
    Return,
    Break,
    Continue,
    If,
    Else,
    While,
    True,
    False,

    I32,
    I64,
    U32,
    U64,
    F64,
    Bool,
    Ptr,

    LeftParen,
    RightParen,
    LeftBrace,
    RightBrace,
    LeftBracket,
    RightBracket,
    Comma,
    Colon,
    Semicolon,
    Dot,
    Arrow,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Equal,
    EqualEqual,
    Bang,
    BangEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    AmpAmp,
    PipePipe,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
    pub line: usize,
    pub column: usize,
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LexResult {
    pub tokens: Vec<Token>,
    pub diagnostics: Vec<Diagnostic>,
}

#[must_use]
pub fn lex(source: &SourceFile) -> LexResult {
    Lexer::new(source).lex()
}

struct Lexer<'source> {
    source: &'source SourceFile,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    byte_offset: usize,
    utf16_offset: usize,
    line: usize,
    column: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LexerPosition {
    byte_offset: usize,
    utf16_offset: usize,
    line: usize,
    column: usize,
}

impl LexerPosition {
    fn source_position(self) -> SourcePosition {
        SourcePosition {
            offset: self.utf16_offset,
            line: self.line,
            column: self.column,
        }
    }
}

impl<'source> Lexer<'source> {
    fn new(source: &'source SourceFile) -> Self {
        Self {
            source,
            tokens: Vec::new(),
            diagnostics: Vec::new(),
            byte_offset: 0,
            utf16_offset: 0,
            line: 1,
            column: 1,
        }
    }

    fn lex(mut self) -> LexResult {
        while !self.is_at_end() {
            self.scan_token();
        }

        let position = self.position();
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            text: String::new(),
            line: position.line,
            column: position.column,
            start: position.utf16_offset,
            end: position.utf16_offset,
        });

        LexResult {
            tokens: self.tokens,
            diagnostics: self.diagnostics,
        }
    }

    fn scan_token(&mut self) {
        let char = self.peek();

        if is_whitespace(char) {
            self.advance();
            return;
        }

        if char == '/' && self.peek_next() == '/' {
            self.skip_line_comment();
            return;
        }

        if is_identifier_start(char) {
            self.scan_identifier_or_keyword();
            return;
        }

        if char.is_ascii_digit() {
            self.scan_number();
            return;
        }

        let start = self.position();

        match char {
            '(' => {
                self.advance();
                self.add_token(TokenKind::LeftParen, start);
            }
            ')' => {
                self.advance();
                self.add_token(TokenKind::RightParen, start);
            }
            '{' => {
                self.advance();
                self.add_token(TokenKind::LeftBrace, start);
            }
            '}' => {
                self.advance();
                self.add_token(TokenKind::RightBrace, start);
            }
            '[' => {
                self.advance();
                self.add_token(TokenKind::LeftBracket, start);
            }
            ']' => {
                self.advance();
                self.add_token(TokenKind::RightBracket, start);
            }
            ',' => {
                self.advance();
                self.add_token(TokenKind::Comma, start);
            }
            ':' => {
                self.advance();
                self.add_token(TokenKind::Colon, start);
            }
            ';' => {
                self.advance();
                self.add_token(TokenKind::Semicolon, start);
            }
            '.' => {
                if self.peek_next().is_ascii_digit() {
                    self.scan_malformed_float_starting_with_dot();
                } else {
                    self.advance();
                    self.add_token(TokenKind::Dot, start);
                }
            }
            '+' => {
                self.advance();
                self.add_token(TokenKind::Plus, start);
            }
            '-' => {
                self.advance();
                if self.match_char('>') {
                    self.add_token(TokenKind::Arrow, start);
                } else {
                    self.add_token(TokenKind::Minus, start);
                }
            }
            '*' => {
                self.advance();
                self.add_token(TokenKind::Star, start);
            }
            '/' => {
                self.advance();
                self.add_token(TokenKind::Slash, start);
            }
            '%' => {
                self.advance();
                self.add_token(TokenKind::Percent, start);
            }
            '=' => {
                self.advance();
                if self.match_char('=') {
                    self.add_token(TokenKind::EqualEqual, start);
                } else {
                    self.add_token(TokenKind::Equal, start);
                }
            }
            '!' => {
                self.advance();
                if self.match_char('=') {
                    self.add_token(TokenKind::BangEqual, start);
                } else {
                    self.add_token(TokenKind::Bang, start);
                }
            }
            '<' => {
                self.advance();
                if self.match_char('=') {
                    self.add_token(TokenKind::LessEqual, start);
                } else {
                    self.add_token(TokenKind::Less, start);
                }
            }
            '>' => {
                self.advance();
                if self.match_char('=') {
                    self.add_token(TokenKind::GreaterEqual, start);
                } else {
                    self.add_token(TokenKind::Greater, start);
                }
            }
            '&' => {
                self.advance();
                if self.match_char('&') {
                    self.add_token(TokenKind::AmpAmp, start);
                } else {
                    self.report_unexpected(start, char);
                }
            }
            '|' => {
                self.advance();
                if self.match_char('|') {
                    self.add_token(TokenKind::PipePipe, start);
                } else {
                    self.report_unexpected(start, char);
                }
            }
            _ => {
                self.scan_unexpected_character(start, char);
            }
        }
    }

    fn scan_identifier_or_keyword(&mut self) {
        let start = self.position();
        self.advance();

        while !self.is_at_end() && is_identifier_part(self.peek()) {
            self.advance();
        }

        let text = &self.source.text[start.byte_offset..self.byte_offset];
        self.add_token(keyword_kind(text).unwrap_or(TokenKind::Identifier), start);
    }

    fn scan_number(&mut self) {
        let start = self.position();
        self.advance();

        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }

        let mut is_float = false;

        if self.peek() == '.' {
            is_float = true;
            self.advance();
            if !self.peek().is_ascii_digit() {
                self.report_malformed_float(start);
                return;
            }

            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }

        if is_exponent_start(self.peek()) {
            is_float = true;
            self.advance();
            if self.peek() == '+' || self.peek() == '-' {
                self.advance();
            }

            if !self.peek().is_ascii_digit() {
                self.report_malformed_float(start);
                return;
            }

            while !self.is_at_end() && self.peek().is_ascii_digit() {
                self.advance();
            }
        }

        self.add_token(
            if is_float {
                TokenKind::Float
            } else {
                TokenKind::Integer
            },
            start,
        );
    }

    fn scan_malformed_float_starting_with_dot(&mut self) {
        let start = self.position();
        self.advance();

        while !self.is_at_end() && self.peek().is_ascii_digit() {
            self.advance();
        }

        self.report_malformed_float(start);
    }

    fn skip_line_comment(&mut self) {
        while !self.is_at_end() && self.peek() != '\n' {
            self.advance();
        }
    }

    fn add_token(&mut self, kind: TokenKind, start: LexerPosition) {
        self.tokens.push(Token {
            kind,
            text: self.source.text[start.byte_offset..self.byte_offset].to_string(),
            line: start.line,
            column: start.column,
            start: start.utf16_offset,
            end: self.utf16_offset,
        });
    }

    fn report_unexpected(&mut self, start: LexerPosition, char: char) {
        self.diagnostics.push(Diagnostic::error(
            DiagnosticCode::Ck0001,
            format!("Unexpected character '{char}'."),
            self.source.file_name.clone(),
            SourceSpan {
                start: start.source_position(),
                end: self.position().source_position(),
            },
        ));
    }

    fn scan_unexpected_character(&mut self, start: LexerPosition, char: char) {
        let utf16_width = char.len_utf16();
        self.advance();

        if utf16_width == 1 {
            self.report_unexpected(start, char);
            return;
        }

        for index in 0..utf16_width {
            let unit_start = SourcePosition {
                offset: start.utf16_offset + index,
                line: start.line,
                column: start.column + index,
            };
            let unit_end = SourcePosition {
                offset: unit_start.offset + 1,
                line: unit_start.line,
                column: unit_start.column + 1,
            };
            self.diagnostics.push(Diagnostic::error(
                DiagnosticCode::Ck0001,
                "Unexpected character '�'.",
                self.source.file_name.clone(),
                SourceSpan {
                    start: unit_start,
                    end: unit_end,
                },
            ));
        }
    }

    fn report_malformed_float(&mut self, start: LexerPosition) {
        let text = &self.source.text[start.byte_offset..self.byte_offset];
        self.diagnostics.push(Diagnostic::error(
            DiagnosticCode::Ck0001,
            format!("Malformed float literal '{text}'."),
            self.source.file_name.clone(),
            SourceSpan {
                start: start.source_position(),
                end: self.position().source_position(),
            },
        ));
    }

    fn match_char(&mut self, expected: char) -> bool {
        if self.is_at_end() || self.peek() != expected {
            return false;
        }

        self.advance();
        true
    }

    fn advance(&mut self) -> char {
        let char = self.peek();
        if char == '\0' {
            return char;
        }

        self.byte_offset += char.len_utf8();
        self.utf16_offset += char.len_utf16();
        if char == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += char.len_utf16();
        }
        char
    }

    fn peek(&self) -> char {
        self.source.text[self.byte_offset..]
            .chars()
            .next()
            .unwrap_or('\0')
    }

    fn peek_next(&self) -> char {
        let mut chars = self.source.text[self.byte_offset..].chars();
        chars.next();
        chars.next().unwrap_or('\0')
    }

    fn is_at_end(&self) -> bool {
        self.byte_offset >= self.source.text.len()
    }

    fn position(&self) -> LexerPosition {
        LexerPosition {
            byte_offset: self.byte_offset,
            utf16_offset: self.utf16_offset,
            line: self.line,
            column: self.column,
        }
    }
}

fn keyword_kind(text: &str) -> Option<TokenKind> {
    match text {
        "struct" => Some(TokenKind::Struct),
        "export" => Some(TokenKind::Export),
        "fn" => Some(TokenKind::Fn),
        "let" => Some(TokenKind::Let),
        "return" => Some(TokenKind::Return),
        "break" => Some(TokenKind::Break),
        "continue" => Some(TokenKind::Continue),
        "if" => Some(TokenKind::If),
        "else" => Some(TokenKind::Else),
        "while" => Some(TokenKind::While),
        "true" => Some(TokenKind::True),
        "false" => Some(TokenKind::False),
        "i32" => Some(TokenKind::I32),
        "i64" => Some(TokenKind::I64),
        "u32" => Some(TokenKind::U32),
        "u64" => Some(TokenKind::U64),
        "f64" => Some(TokenKind::F64),
        "bool" => Some(TokenKind::Bool),
        "ptr" => Some(TokenKind::Ptr),
        _ => None,
    }
}

fn is_whitespace(char: char) -> bool {
    matches!(char, ' ' | '\r' | '\t' | '\n')
}

fn is_exponent_start(char: char) -> bool {
    matches!(char, 'e' | 'E')
}

fn is_identifier_start(char: char) -> bool {
    char == '_' || char.is_ascii_alphabetic()
}

fn is_identifier_part(char: char) -> bool {
    is_identifier_start(char) || char.is_ascii_digit()
}
