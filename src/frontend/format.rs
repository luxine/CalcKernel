use super::{SourceFile, Token, TokenKind, lex, parse};

#[derive(Debug)]
struct LineComment {
    start: usize,
    text: String,
    inline: bool,
}

#[derive(Debug)]
enum Event {
    Token(TokenKind, String),
    Comment { text: String, inline: bool },
}

/// Formats syntactically valid CK source with deterministic whitespace.
///
/// Token spellings, token order, and line-comment contents are preserved. If
/// parsing fails, the original source is returned unchanged.
pub fn format_source(source: &str) -> String {
    let source_file = SourceFile::new("<format>.ck", source);
    if !parse(&source_file).diagnostics.is_empty() {
        return source.to_owned();
    }

    let lexed = lex(&source_file);
    if !lexed.diagnostics.is_empty() {
        return source.to_owned();
    }

    let comments = scan_line_comments(source);
    let events = merge_events(source, lexed.tokens, comments);
    let formatted = Formatter::new().format(&events);

    if token_signature(source) == token_signature(&formatted)
        && comment_signature(source) == comment_signature(&formatted)
        && parse(&SourceFile::new("<formatted>.ck", &formatted))
            .diagnostics
            .is_empty()
    {
        formatted
    } else {
        source.to_owned()
    }
}

fn token_signature(source: &str) -> Vec<(TokenKind, String)> {
    lex(&SourceFile::new("<format-check>.ck", source))
        .tokens
        .into_iter()
        .filter(|token| token.kind != TokenKind::Eof)
        .map(|token| (token.kind, token.text))
        .collect()
}

fn comment_signature(source: &str) -> Vec<String> {
    scan_line_comments(source)
        .into_iter()
        .map(|comment| comment.text)
        .collect()
}

fn scan_line_comments(source: &str) -> Vec<LineComment> {
    let bytes = source.as_bytes();
    let mut comments = Vec::new();
    let mut line_start = 0;

    while line_start < bytes.len() {
        let line_end = bytes[line_start..]
            .iter()
            .position(|byte| matches!(byte, b'\r' | b'\n'))
            .map_or(bytes.len(), |offset| line_start + offset);
        let line = &source[line_start..line_end];
        if let Some(comment_offset) = line.find("//") {
            let prefix = &line[..comment_offset];
            comments.push(LineComment {
                start: line_start + comment_offset,
                text: line[comment_offset..].to_owned(),
                inline: prefix
                    .chars()
                    .any(|character| !matches!(character, ' ' | '\t')),
            });
        }

        if line_end == bytes.len() {
            break;
        }
        line_start = if bytes[line_end] == b'\r'
            && bytes.get(line_end + 1).is_some_and(|byte| *byte == b'\n')
        {
            line_end + 2
        } else {
            line_end + 1
        };
    }

    comments
}

fn merge_events(source: &str, tokens: Vec<Token>, comments: Vec<LineComment>) -> Vec<Event> {
    let byte_offsets = utf16_byte_offsets(source);
    let mut positioned = Vec::with_capacity(tokens.len() + comments.len());

    for token in tokens {
        if token.kind == TokenKind::Eof {
            continue;
        }
        let start = byte_offsets
            .get(token.start)
            .copied()
            .unwrap_or(source.len());
        positioned.push((start, Event::Token(token.kind, token.text)));
    }
    for comment in comments {
        positioned.push((
            comment.start,
            Event::Comment {
                text: comment.text,
                inline: comment.inline,
            },
        ));
    }

    positioned.sort_by_key(|(start, _)| *start);
    positioned.into_iter().map(|(_, event)| event).collect()
}

fn utf16_byte_offsets(source: &str) -> Vec<usize> {
    let utf16_length = source.encode_utf16().count();
    let mut offsets = vec![source.len(); utf16_length + 1];
    let mut unit_offset = 0;

    for (byte_offset, character) in source.char_indices() {
        offsets[unit_offset] = byte_offset;
        let width = character.len_utf16();
        if width == 2 {
            offsets[unit_offset + 1] = byte_offset;
        }
        unit_offset += width;
        offsets[unit_offset] = byte_offset + character.len_utf8();
    }

    offsets
}

struct Formatter {
    output: String,
    line: String,
    line_has_content: bool,
    indent: usize,
    generic_depth: usize,
    previous: Option<TokenKind>,
    pending_top_level_gap: bool,
}

impl Formatter {
    fn new() -> Self {
        Self {
            output: String::new(),
            line: String::new(),
            line_has_content: false,
            indent: 0,
            generic_depth: 0,
            previous: None,
            pending_top_level_gap: false,
        }
    }

    fn format(mut self, events: &[Event]) -> String {
        for (index, event) in events.iter().enumerate() {
            match event {
                Event::Token(kind, text) => {
                    self.prepare_top_level_token(*kind);
                    let inline_comment_follows = matches!(
                        events.get(index + 1),
                        Some(Event::Comment { inline: true, .. })
                    );
                    let else_follows = matches!(
                        events.get(index + 1),
                        Some(Event::Token(TokenKind::Else, _))
                    );
                    self.write_token(*kind, text, inline_comment_follows, else_follows);
                    self.previous = Some(*kind);
                }
                Event::Comment { text, inline } => self.write_comment(text, *inline),
            }
        }
        self.flush_line();
        self.output
    }

    fn prepare_top_level_token(&mut self, kind: TokenKind) {
        if !self.pending_top_level_gap {
            return;
        }
        if self.indent == 0 && is_declaration_start(kind) {
            self.ensure_blank_line();
        }
        self.pending_top_level_gap = false;
    }

    fn write_comment(&mut self, text: &str, inline: bool) {
        if inline {
            if self.line_has_content {
                self.space();
            }
            self.push(text);
            self.flush_line();
            return;
        }

        self.flush_line();
        if self.pending_top_level_gap && self.indent == 0 {
            self.ensure_blank_line();
            self.pending_top_level_gap = false;
        }
        self.push(text);
        self.flush_line();
    }

    fn write_token(
        &mut self,
        kind: TokenKind,
        text: &str,
        inline_comment_follows: bool,
        else_follows: bool,
    ) {
        match kind {
            TokenKind::LeftBrace => {
                if self.line_has_content {
                    self.space();
                }
                self.push(text);
                self.indent += 1;
                if !inline_comment_follows {
                    self.flush_line();
                }
            }
            TokenKind::RightBrace => {
                self.flush_line();
                self.indent = self.indent.saturating_sub(1);
                self.push(text);
                if self.indent == 0 {
                    self.pending_top_level_gap = true;
                }
                if !inline_comment_follows && !else_follows {
                    self.flush_line();
                }
            }
            TokenKind::Contract => {
                self.flush_line();
                self.push(text);
            }
            TokenKind::Semicolon => {
                self.push(text);
                if !inline_comment_follows {
                    self.flush_line();
                }
            }
            TokenKind::Comma => {
                self.push(text);
                self.space();
            }
            TokenKind::Colon => {
                self.trim_line_end_space();
                self.push(text);
                self.space();
            }
            TokenKind::Arrow => self.write_spaced_operator(text),
            TokenKind::Dot | TokenKind::DotDot => self.push(text),
            TokenKind::LeftParen => {
                if matches!(
                    self.previous,
                    Some(TokenKind::If | TokenKind::While | TokenKind::Return)
                ) {
                    self.space();
                }
                self.push(text);
            }
            TokenKind::RightParen | TokenKind::LeftBracket | TokenKind::RightBracket => {
                self.trim_line_end_space();
                self.push(text);
            }
            TokenKind::Less if self.is_generic_open() => {
                self.trim_line_end_space();
                self.push(text);
                self.generic_depth += 1;
            }
            TokenKind::Greater if self.generic_depth > 0 => {
                self.trim_line_end_space();
                self.push(text);
                self.generic_depth -= 1;
            }
            TokenKind::Equal
            | TokenKind::EqualEqual
            | TokenKind::BangEqual
            | TokenKind::Less
            | TokenKind::LessEqual
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::AmpAmp
            | TokenKind::PipePipe
            | TokenKind::Plus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent => self.write_spaced_operator(text),
            TokenKind::Bang | TokenKind::Minus => self.write_bang_or_minus(kind, text),
            _ => self.write_word(kind, text),
        }
    }

    fn write_word(&mut self, kind: TokenKind, text: &str) {
        if self.previous.is_some_and(is_word_kind)
            || (kind == TokenKind::Else && self.previous == Some(TokenKind::RightBrace))
        {
            self.space();
        }
        self.push(text);
        if matches!(kind, TokenKind::Ptr | TokenKind::Slice) {
            self.trim_line_end_space();
        }
    }

    fn write_bang_or_minus(&mut self, kind: TokenKind, text: &str) {
        let is_prefix = self
            .previous
            .is_none_or(|previous| !can_end_expression(previous));
        if is_prefix {
            if self.previous.is_some_and(is_word_kind) {
                self.space();
            }
            self.push(text);
        } else {
            self.write_spaced_operator(text);
        }
        if kind == TokenKind::Bang {
            self.trim_line_end_space();
        }
    }

    fn write_spaced_operator(&mut self, text: &str) {
        self.trim_line_end_space();
        if self.line_has_content {
            self.space();
        }
        self.push(text);
        self.space();
    }

    fn is_generic_open(&self) -> bool {
        self.generic_depth > 0 || matches!(self.previous, Some(TokenKind::Ptr | TokenKind::Slice))
    }

    fn push(&mut self, text: &str) {
        self.start_line_if_needed();
        self.line.push_str(text);
        self.line_has_content = true;
    }

    fn space(&mut self) {
        if self.line_has_content && !self.line.ends_with(' ') {
            self.line.push(' ');
        }
    }

    fn trim_line_end_space(&mut self) {
        while self.line.ends_with(' ') {
            self.line.pop();
        }
    }

    fn start_line_if_needed(&mut self) {
        if self.line_has_content {
            return;
        }
        self.line.clear();
        self.line
            .extend(std::iter::repeat_n(' ', self.indent.saturating_mul(2)));
    }

    fn flush_line(&mut self) {
        if !self.line_has_content {
            self.line.clear();
            return;
        }
        self.output.push_str(&self.line);
        self.output.push('\n');
        self.line.clear();
        self.line_has_content = false;
    }

    fn ensure_blank_line(&mut self) {
        self.flush_line();
        if !self.output.is_empty() && !self.output.ends_with("\n\n") {
            self.output.push('\n');
        }
    }
}

fn is_declaration_start(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Struct | TokenKind::Export | TokenKind::Unsafe | TokenKind::Fn
    )
}

fn is_word_kind(kind: TokenKind) -> bool {
    !matches!(
        kind,
        TokenKind::Eof
            | TokenKind::LeftParen
            | TokenKind::RightParen
            | TokenKind::LeftBrace
            | TokenKind::RightBrace
            | TokenKind::LeftBracket
            | TokenKind::RightBracket
            | TokenKind::Comma
            | TokenKind::Colon
            | TokenKind::Semicolon
            | TokenKind::Dot
            | TokenKind::DotDot
            | TokenKind::Arrow
            | TokenKind::Plus
            | TokenKind::Minus
            | TokenKind::Star
            | TokenKind::Slash
            | TokenKind::Percent
            | TokenKind::Equal
            | TokenKind::EqualEqual
            | TokenKind::Bang
            | TokenKind::BangEqual
            | TokenKind::Less
            | TokenKind::LessEqual
            | TokenKind::Greater
            | TokenKind::GreaterEqual
            | TokenKind::AmpAmp
            | TokenKind::PipePipe
    )
}

fn can_end_expression(kind: TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier
            | TokenKind::Integer
            | TokenKind::Float
            | TokenKind::True
            | TokenKind::False
            | TokenKind::RightParen
            | TokenKind::RightBracket
    )
}

#[cfg(test)]
mod tests {
    use super::format_source;
    use crate::frontend::{SourceFile, lex, parse};

    fn comment_signature(source: &str) -> Vec<String> {
        super::scan_line_comments(source)
            .into_iter()
            .map(|comment| comment.text)
            .collect()
    }

    fn lexemes(source: &str) -> Vec<(crate::frontend::TokenKind, String)> {
        lex(&SourceFile::new("formatter-test.ck", source))
            .tokens
            .into_iter()
            .filter(|token| token.kind != crate::frontend::TokenKind::Eof)
            .map(|token| (token.kind, token.text))
            .collect()
    }

    fn assert_idempotent(source: &str) -> String {
        let formatted = format_source(source);
        assert_eq!(format_source(&formatted), formatted);
        assert_eq!(lexemes(source), lexemes(&formatted));
        assert_eq!(comment_signature(source), comment_signature(&formatted));
        let diagnostics = parse(&SourceFile::new("formatted.ck", &formatted)).diagnostics;
        assert!(
            diagnostics.is_empty(),
            "unexpected formatted diagnostics: {diagnostics:?}\n{formatted}"
        );
        formatted
    }

    #[test]
    fn formats_struct_fields_and_function_signatures() {
        let source = "struct Item{price:i64;qty:slice<f64>; }\nfn sum(items:slice<i32>,n:i32)->i32{return items[0]+n;}";
        let expected = "struct Item {\n  price: i64;\n  qty: slice<f64>;\n}\n\nfn sum(items: slice<i32>, n: i32) -> i32 {\n  return items[0] + n;\n}\n";
        assert_eq!(assert_idempotent(source), expected);
    }

    #[test]
    fn formats_nested_control_flow_and_unsafe_blocks() {
        let source = "fn walk(out:ptr<i32>,n:i32)->void{let i:i32=0;while i<n{unsafe{out[i]=i;if i==1{continue;}if i>2{break;}}i=i+1;}return;}";
        let expected = "fn walk(out: ptr<i32>, n: i32) -> void {\n  let i: i32 = 0;\n  while i < n {\n    unsafe {\n      out[i] = i;\n      if i == 1 {\n        continue;\n      }\n      if i > 2 {\n        break;\n      }\n    }\n    i = i + 1;\n  }\n  return;\n}\n";
        assert_eq!(assert_idempotent(source), expected);
    }

    #[test]
    fn keeps_else_attached_to_the_preceding_block() {
        let source = "fn choose(x:i32)->i32{if x>0{return x;}else{return 0;}}";
        let expected = "fn choose(x: i32) -> i32 {\n  if x > 0 {\n    return x;\n  } else {\n    return 0;\n  }\n}\n";
        assert_eq!(assert_idempotent(source), expected);
    }

    #[test]
    fn formats_contracts_and_slice_construction_and_ranges() {
        let source = "export unsafe fn select(data:ptr<i32>,len:u32,start:u32,end:u32)->slice<i32> contract{requires end<=len;effects read(data);} {let values:slice<i32> =slice(data,len);return values[start..end];}";
        let expected = "export unsafe fn select(data: ptr<i32>, len: u32, start: u32, end: u32) -> slice<i32>\ncontract {\n  requires end <= len;\n  effects read(data);\n}\n{\n  let values: slice<i32> = slice(data, len);\n  return values[start..end];\n}\n";
        assert_eq!(assert_idempotent(source), expected);
    }

    #[test]
    fn preserves_inline_and_standalone_line_comments() {
        let source = "fn value()->i32{// opening\nlet n:i32=1;// trailing π\n// standalone 𐐀\nreturn n;// final\n}";
        let expected = "fn value() -> i32 { // opening\n  let n: i32 = 1; // trailing π\n  // standalone 𐐀\n  return n; // final\n}\n";
        let formatted = assert_idempotent(source);
        assert_eq!(formatted, expected);
        for comment in ["// opening", "// trailing π", "// standalone 𐐀", "// final"] {
            assert_eq!(formatted.matches(comment).count(), 1);
        }
    }

    #[test]
    fn normalizes_crlf_without_corrupting_unicode_comment_offsets() {
        let source = "fn value()->i32{\r\n// 🧮 CK\r\nreturn 1;\r\n}\r\n";
        let expected = "fn value() -> i32 {\n  // 🧮 CK\n  return 1;\n}\n";
        assert_eq!(assert_idempotent(source), expected);
    }

    #[test]
    fn leaves_unparseable_source_unchanged() {
        let source = "fn broken( { // keep exactly\r\n";
        assert!(
            !parse(&SourceFile::new("invalid.ck", source))
                .diagnostics
                .is_empty()
        );
        assert_eq!(format_source(source), source);
    }

    #[test]
    fn empty_source_stays_empty() {
        assert_eq!(format_source(""), "");
    }
}
