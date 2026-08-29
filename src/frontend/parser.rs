use super::*;

#[must_use]
pub fn parse(source: &SourceFile) -> ParseResult {
    let lex_result = lex(source);
    Parser::new(source, lex_result.tokens, lex_result.diagnostics).parse()
}

struct Parser<'source> {
    source: &'source SourceFile,
    tokens: Vec<Token>,
    diagnostics: Vec<Diagnostic>,
    index: usize,
}

impl<'source> Parser<'source> {
    fn new(source: &'source SourceFile, tokens: Vec<Token>, diagnostics: Vec<Diagnostic>) -> Self {
        Self {
            source,
            tokens,
            diagnostics,
            index: 0,
        }
    }

    fn parse(mut self) -> ParseResult {
        let start = self.position_from_token(self.current());
        let mut declarations = Vec::new();

        while !self.check(TokenKind::Eof) {
            if let Some(declaration) = self.parse_declaration() {
                declarations.push(declaration);
            }
        }

        let end = self.position_from_token(self.current());
        ParseResult {
            ast: Program {
                declarations,
                span: SourceSpan { start, end },
            },
            diagnostics: self.diagnostics,
        }
    }

    fn parse_declaration(&mut self) -> Option<Declaration> {
        if self.check(TokenKind::Struct) {
            return Some(Declaration::Struct(self.parse_struct_declaration()));
        }

        if self.check(TokenKind::Contract) {
            let token = self.advance();
            self.error_with_code(
                &token,
                DiagnosticCode::Ck2014,
                "A contract may appear only after an unsafe function signature.",
            );
            self.skip_braced_declaration();
            return None;
        }

        if self.check(TokenKind::Unsafe) && self.next_is(TokenKind::LeftBrace) {
            let token = self.advance();
            self.error_with_code(
                &token,
                DiagnosticCode::Ck2014,
                "An unsafe block may appear only inside a function body.",
            );
            self.skip_braced_declaration();
            return None;
        }

        if self.check(TokenKind::Export)
            || self.check(TokenKind::Unsafe)
            || self.check(TokenKind::Fn)
        {
            return Some(Declaration::Function(Box::new(
                self.parse_function_declaration(),
            )));
        }

        let token = self.current().clone();
        self.error(&token, "Expected declaration.");
        self.advance();
        None
    }

    fn parse_struct_declaration(&mut self) -> StructDeclaration {
        let struct_token = self.consume(TokenKind::Struct, "Expected 'struct'.");
        let name = self.parse_identifier("Expected struct name.");
        self.consume(TokenKind::LeftBrace, "Expected '{' after struct name.");

        let mut fields = Vec::new();
        while !self.check(TokenKind::RightBrace) && !self.check(TokenKind::Eof) {
            let field_start = self.current().clone();
            let field_name = self.parse_identifier("Expected field name.");
            self.consume(TokenKind::Colon, "Expected ':' after field name.");
            let field_type = self.parse_type();
            let semicolon = self.consume(TokenKind::Semicolon, "Expected ';' after struct field.");
            fields.push(StructField {
                name: field_name,
                type_node: field_type,
                span: self.span_between_tokens(&field_start, &semicolon),
            });
        }

        let end = self.consume(TokenKind::RightBrace, "Expected '}' after struct fields.");
        StructDeclaration {
            name,
            fields,
            span: self.span_between_tokens(&struct_token, &end),
        }
    }

    fn parse_function_declaration(&mut self) -> FunctionDeclaration {
        let start_token = self.current().clone();
        let mut exported = self.match_token(TokenKind::Export);
        let is_unsafe = self.match_token(TokenKind::Unsafe);
        if is_unsafe && self.match_token(TokenKind::Export) {
            exported = true;
            let export_token = self.previous().clone();
            self.error_with_code(
                &export_token,
                DiagnosticCode::Ck2014,
                "Function modifiers must appear in '[export] [unsafe] fn' order.",
            );
        }
        self.consume(TokenKind::Fn, "Expected 'fn' after function modifiers.");
        let name = self.parse_identifier("Expected function name.");
        self.consume(TokenKind::LeftParen, "Expected '(' after function name.");

        let mut params = Vec::new();
        if !self.check(TokenKind::RightParen) {
            loop {
                params.push(self.parse_function_param());
                if !self.match_token(TokenKind::Comma) {
                    break;
                }
            }
        }

        self.consume(TokenKind::RightParen, "Expected ')' after parameters.");
        self.consume(TokenKind::Arrow, "Expected '->' before return type.");
        let return_type = self.parse_type();
        let contract = self
            .match_token(TokenKind::Contract)
            .then(|| Box::new(self.parse_contract_declaration()));
        let body = self.parse_block_statement();

        FunctionDeclaration {
            exported,
            is_unsafe,
            name,
            params,
            return_type,
            contract,
            span: self.span_from_positions(self.position_from_token(&start_token), body.span.end),
            body,
        }
    }

    fn parse_contract_declaration(&mut self) -> ContractDeclaration {
        let contract_token = self.previous().clone();
        self.consume(TokenKind::LeftBrace, "Expected '{' after 'contract'.");
        let mut requirements = Vec::new();
        let mut effects = None;
        while !self.check(TokenKind::RightBrace) && !self.check(TokenKind::Eof) {
            if self.match_token(TokenKind::Requires) {
                let requires_token = self.previous().clone();
                let expression = self.parse_expression(1);
                let semicolon = self.consume(
                    TokenKind::Semicolon,
                    "Expected ';' after contract requirement.",
                );
                requirements.push(ContractRequirement {
                    expression,
                    span: self.span_between_tokens(&requires_token, &semicolon),
                });
                continue;
            }
            if self.match_token(TokenKind::Effects) {
                let effects_token = self.previous().clone();
                let clause = self.parse_contract_effect_clause(effects_token);
                if effects.replace(clause).is_some() {
                    let token = self.previous().clone();
                    self.error(&token, "A contract can contain only one effects clause.");
                }
                continue;
            }
            let token = self.current().clone();
            self.error(&token, "Expected 'requires' or 'effects' in contract.");
            self.synchronize_contract_clause();
        }
        let right_brace = self.consume(TokenKind::RightBrace, "Expected '}' after contract.");
        ContractDeclaration {
            requirements,
            effects,
            span: self.span_between_tokens(&contract_token, &right_brace),
        }
    }

    fn parse_contract_effect_clause(&mut self, effects_token: Token) -> ContractEffectClause {
        if self.match_token(TokenKind::None) {
            let semicolon =
                self.consume(TokenKind::Semicolon, "Expected ';' after effects clause.");
            return ContractEffectClause {
                is_none: true,
                items: Vec::new(),
                span: self.span_between_tokens(&effects_token, &semicolon),
            };
        }

        let mut items = Vec::new();
        loop {
            let kind_token = self.consume(
                TokenKind::Identifier,
                "Expected read, write, or readwrite in effects clause.",
            );
            let kind = match kind_token.text.as_str() {
                "read" => ContractEffectKind::Read,
                "write" => ContractEffectKind::Write,
                "readwrite" => ContractEffectKind::ReadWrite,
                _ => {
                    self.error(
                        &kind_token,
                        "Expected read, write, or readwrite in effects clause.",
                    );
                    ContractEffectKind::ReadWrite
                }
            };
            self.consume(TokenKind::LeftParen, "Expected '(' after effect kind.");
            let target = self.parse_identifier("Expected slice parameter in effect.");
            let right_paren =
                self.consume(TokenKind::RightParen, "Expected ')' after effect target.");
            items.push(ContractEffectItem {
                kind,
                span: self.span_from_positions(
                    self.position_from_token(&kind_token),
                    self.end_position_from_token(&right_paren),
                ),
                target,
            });
            if !self.match_token(TokenKind::Comma) {
                break;
            }
        }
        let semicolon = self.consume(TokenKind::Semicolon, "Expected ';' after effects clause.");
        ContractEffectClause {
            is_none: false,
            items,
            span: self.span_between_tokens(&effects_token, &semicolon),
        }
    }

    fn parse_function_param(&mut self) -> FunctionParam {
        let start = self.current().clone();
        let name = self.parse_identifier("Expected parameter name.");
        self.consume(TokenKind::Colon, "Expected ':' after parameter name.");
        let type_node = self.parse_type();
        FunctionParam {
            name,
            span: self.span_from_positions(self.position_from_token(&start), type_node.span().end),
            type_node,
        }
    }

    fn parse_type(&mut self) -> TypeNode {
        let token = self.current().clone();
        match token.kind {
            TokenKind::I32
            | TokenKind::I64
            | TokenKind::U32
            | TokenKind::U64
            | TokenKind::F64
            | TokenKind::Bool => {
                self.advance();
                let span = self.span_from_token(&token);
                TypeNode::Primitive {
                    name: token.text,
                    span,
                }
            }
            TokenKind::Void => {
                self.advance();
                TypeNode::Void {
                    span: self.span_from_token(&token),
                }
            }
            TokenKind::Identifier => {
                let name = self.parse_identifier("Expected type name.");
                TypeNode::Named {
                    span: name.span,
                    name,
                }
            }
            TokenKind::Ptr => {
                let ptr_token = self.advance();
                self.consume(TokenKind::Less, "Expected '<' after 'ptr'.");
                let element_type = self.parse_type();
                let greater = self.consume(TokenKind::Greater, "Expected '>' after pointer type.");
                TypeNode::Pointer {
                    element_type: Box::new(element_type),
                    span: self.span_between_tokens(&ptr_token, &greater),
                }
            }
            TokenKind::Slice => {
                let slice_token = self.advance();
                self.consume(TokenKind::Less, "Expected '<' after 'slice'.");
                let element_type = self.parse_type();
                let greater = self.consume(TokenKind::Greater, "Expected '>' after slice type.");
                TypeNode::Slice {
                    element_type: Box::new(element_type),
                    span: self.span_between_tokens(&slice_token, &greater),
                }
            }
            _ => {
                self.error(&token, "Expected type.");
                self.advance();
                TypeNode::Error {
                    span: self.span_from_token(&token),
                }
            }
        }
    }

    fn parse_block_statement(&mut self) -> BlockStatement {
        let left_brace = self.consume(TokenKind::LeftBrace, "Expected '{' before block.");
        let mut statements = Vec::new();

        while !self.check(TokenKind::RightBrace) && !self.check(TokenKind::Eof) {
            statements.push(self.parse_statement());
        }

        let right_brace = self.consume(TokenKind::RightBrace, "Expected '}' after block.");
        BlockStatement {
            statements,
            span: self.span_between_tokens(&left_brace, &right_brace),
        }
    }

    fn parse_statement(&mut self) -> Statement {
        if self.check(TokenKind::LeftBrace) {
            return Statement::Block(self.parse_block_statement());
        }
        if self.match_token(TokenKind::Unsafe) {
            let unsafe_token = self.previous().clone();
            let block = self.parse_block_statement();
            return Statement::Unsafe(UnsafeStatement {
                span: self
                    .span_from_positions(self.position_from_token(&unsafe_token), block.span.end),
                block,
            });
        }
        if self.check(TokenKind::Let) {
            return Statement::Let(self.parse_let_statement());
        }
        if self.check(TokenKind::Return) {
            return Statement::Return(self.parse_return_statement());
        }
        if self.check(TokenKind::Break) {
            return Statement::Break(self.parse_break_statement());
        }
        if self.check(TokenKind::Continue) {
            return Statement::Continue(self.parse_continue_statement());
        }
        if self.check(TokenKind::If) {
            return Statement::If(self.parse_if_statement());
        }
        if self.check(TokenKind::While) {
            return Statement::While(self.parse_while_statement());
        }

        self.parse_assignment_or_call_statement()
    }

    fn parse_let_statement(&mut self) -> LetStatement {
        let let_token = self.consume(TokenKind::Let, "Expected 'let'.");
        let name = self.parse_identifier("Expected local name.");
        self.consume(TokenKind::Colon, "Expected ':' after local name.");
        let type_node = self.parse_type();
        self.consume(TokenKind::Equal, "Expected '=' after local type.");
        let initializer = self.parse_expression(1);
        let semicolon = self.consume(TokenKind::Semicolon, "Expected ';' after let statement.");

        LetStatement {
            name,
            type_node,
            initializer,
            span: self.span_between_tokens(&let_token, &semicolon),
        }
    }

    fn parse_assignment_or_call_statement(&mut self) -> Statement {
        let start = self.current().clone();
        let target = self.parse_expression(1);

        if self.match_token(TokenKind::Equal) {
            let value = self.parse_expression(1);
            let semicolon = self.consume(
                TokenKind::Semicolon,
                "Expected ';' after assignment statement.",
            );
            return Statement::Assignment(AssignmentStatement {
                span: self.span_from_positions(
                    target.span().start,
                    self.end_position_from_token(&semicolon),
                ),
                target,
                value,
            });
        }

        if matches!(target, Expression::Call { .. }) {
            let semicolon = self.consume(
                TokenKind::Semicolon,
                "Expected ';' after function call statement.",
            );
            return Statement::Call(CallStatement {
                span: self.span_from_positions(
                    target.span().start,
                    self.end_position_from_token(&semicolon),
                ),
                call: target,
            });
        }

        let token = self.current().clone();
        self.error(
            &token,
            "Only a function call may be used as a standalone statement.",
        );
        self.synchronize_statement();
        Statement::Error {
            span: self.span_from_token(&start),
        }
    }

    fn parse_return_statement(&mut self) -> ReturnStatement {
        let return_token = self.consume(TokenKind::Return, "Expected 'return'.");
        let lex_error_before_semicolon = self.check(TokenKind::Semicolon)
            && self.diagnostics.iter().any(|diagnostic| {
                diagnostic.code == DiagnosticCode::Ck0001
                    && diagnostic.span.start.offset >= return_token.end
                    && diagnostic.span.start.offset < self.current().start
            });
        let value = if self.check(TokenKind::Semicolon) && !lex_error_before_semicolon {
            None
        } else {
            Some(self.parse_expression(1))
        };
        let semicolon = self.consume(TokenKind::Semicolon, "Expected ';' after return statement.");
        ReturnStatement {
            value,
            span: self.span_between_tokens(&return_token, &semicolon),
        }
    }

    fn parse_break_statement(&mut self) -> BreakStatement {
        let token = self.consume(TokenKind::Break, "Expected 'break'.");
        let semicolon = self.consume(TokenKind::Semicolon, "Expected ';' after 'break'.");
        BreakStatement {
            span: self.span_between_tokens(&token, &semicolon),
        }
    }

    fn parse_continue_statement(&mut self) -> ContinueStatement {
        let token = self.consume(TokenKind::Continue, "Expected 'continue'.");
        let semicolon = self.consume(TokenKind::Semicolon, "Expected ';' after 'continue'.");
        ContinueStatement {
            span: self.span_between_tokens(&token, &semicolon),
        }
    }

    fn parse_if_statement(&mut self) -> IfStatement {
        let if_token = self.consume(TokenKind::If, "Expected 'if'.");
        let condition = self.parse_expression(1);
        let then_block = self.parse_block_statement();
        let else_block = if self.match_token(TokenKind::Else) {
            Some(self.parse_block_statement())
        } else {
            None
        };
        let end = else_block
            .as_ref()
            .map_or(then_block.span.end, |block| block.span.end);
        IfStatement {
            condition,
            then_block,
            else_block,
            span: self.span_from_positions(self.position_from_token(&if_token), end),
        }
    }

    fn parse_while_statement(&mut self) -> WhileStatement {
        let while_token = self.consume(TokenKind::While, "Expected 'while'.");
        let condition = self.parse_expression(1);
        let body = self.parse_block_statement();
        WhileStatement {
            condition,
            span: self.span_from_positions(self.position_from_token(&while_token), body.span.end),
            body,
        }
    }

    fn parse_expression(&mut self, min_precedence: u8) -> Expression {
        let mut left = self.parse_unary_expression();

        loop {
            let operator = self.current().clone();
            let precedence = binary_precedence(operator.kind);
            if precedence < min_precedence {
                break;
            }

            self.advance();
            let right = self.parse_expression(precedence + 1);
            left = Expression::Binary {
                operator: operator.text,
                span: self.span_from_positions(left.span().start, right.span().end),
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        left
    }

    fn parse_unary_expression(&mut self) -> Expression {
        if self.check(TokenKind::Bang) || self.check(TokenKind::Minus) {
            let operator = self.advance();
            let operand = self.parse_expression(7);
            let span =
                self.span_from_positions(self.position_from_token(&operator), operand.span().end);
            return Expression::Unary {
                operator: operator.text,
                span,
                operand: Box::new(operand),
            };
        }

        let primary = self.parse_primary_expression();
        self.parse_postfix_expression(primary)
    }

    fn parse_postfix_expression(&mut self, base: Expression) -> Expression {
        let mut expression = base;

        loop {
            if self.match_token(TokenKind::LeftParen) {
                let mut args = Vec::new();
                if !self.check(TokenKind::RightParen) {
                    loop {
                        args.push(self.parse_expression(1));
                        if !self.match_token(TokenKind::Comma) {
                            break;
                        }
                    }
                }
                let right_paren =
                    self.consume(TokenKind::RightParen, "Expected ')' after arguments.");
                expression = Expression::Call {
                    span: self.span_from_positions(
                        expression.span().start,
                        self.end_position_from_token(&right_paren),
                    ),
                    callee: Box::new(expression),
                    args,
                };
                continue;
            }

            if self.match_token(TokenKind::Dot) {
                let field = self.parse_identifier("Expected field name after '.'.");
                expression = Expression::Field {
                    span: self.span_from_positions(expression.span().start, field.span.end),
                    object: Box::new(expression),
                    field,
                };
                continue;
            }

            if self.match_token(TokenKind::LeftBracket) {
                if self.check(TokenKind::DotDot) {
                    let token = self.current().clone();
                    self.error(&token, "Sub-slice start expression is required.");
                    let right_bracket = self.recover_to_right_bracket();
                    expression = Expression::Error {
                        span: self.span_from_positions(
                            expression.span().start,
                            self.end_position_from_token(&right_bracket),
                        ),
                    };
                    continue;
                }
                let index = self.parse_expression(1);
                if self.match_token(TokenKind::DotDot) {
                    if self.check(TokenKind::RightBracket) {
                        let token = self.current().clone();
                        self.error(&token, "Sub-slice end expression is required.");
                        let right_bracket = self.advance();
                        expression = Expression::Error {
                            span: self.span_from_positions(
                                expression.span().start,
                                self.end_position_from_token(&right_bracket),
                            ),
                        };
                        continue;
                    }
                    let end = self.parse_expression(1);
                    if self.check(TokenKind::DotDot) {
                        let token = self.current().clone();
                        self.error(&token, "Only one '..' is allowed in a sub-slice.");
                        let right_bracket = self.recover_to_right_bracket();
                        expression = Expression::Error {
                            span: self.span_from_positions(
                                expression.span().start,
                                self.end_position_from_token(&right_bracket),
                            ),
                        };
                        continue;
                    }
                    let right_bracket = self.consume(
                        TokenKind::RightBracket,
                        "Expected ']' after sub-slice expression.",
                    );
                    expression = Expression::Subslice {
                        span: self.span_from_positions(
                            expression.span().start,
                            self.end_position_from_token(&right_bracket),
                        ),
                        slice: Box::new(expression),
                        start: Box::new(index),
                        end: Box::new(end),
                    };
                    continue;
                }
                let right_bracket = self.consume(
                    TokenKind::RightBracket,
                    "Expected ']' after index expression.",
                );
                expression = Expression::Index {
                    span: self.span_from_positions(
                        expression.span().start,
                        self.end_position_from_token(&right_bracket),
                    ),
                    object: Box::new(expression),
                    index: Box::new(index),
                };
                continue;
            }

            return expression;
        }
    }

    fn parse_primary_expression(&mut self) -> Expression {
        let token = self.current().clone();

        if self.match_token(TokenKind::Integer) {
            let span = self.span_from_token(&token);
            return Expression::IntegerLiteral {
                text: token.text,
                span,
            };
        }

        if self.match_token(TokenKind::Float) {
            let span = self.span_from_token(&token);
            return Expression::FloatLiteral {
                text: token.text,
                span,
            };
        }

        if self.match_token(TokenKind::True) || self.match_token(TokenKind::False) {
            return Expression::BoolLiteral {
                value: token.kind == TokenKind::True,
                span: self.span_from_token(&token),
            };
        }

        if self.match_token(TokenKind::Identifier) {
            let span = self.span_from_token(&token);
            return Expression::Identifier {
                name: token.text,
                span,
            };
        }

        if self.match_token(TokenKind::Slice) {
            self.consume(TokenKind::LeftParen, "Expected '(' after 'slice'.");
            let data = self.parse_expression(1);
            self.consume(
                TokenKind::Comma,
                "Expected ',' after slice data expression.",
            );
            let len = self.parse_expression(1);
            let right_paren = self.consume(
                TokenKind::RightParen,
                "Expected ')' after slice constructor.",
            );
            return Expression::SliceConstructor {
                data: Box::new(data),
                len: Box::new(len),
                span: self.span_from_positions(
                    self.position_from_token(&token),
                    self.end_position_from_token(&right_paren),
                ),
            };
        }

        if self.match_token(TokenKind::LeftParen) {
            let expression = self.parse_expression(1);
            let right_paren = self.consume(TokenKind::RightParen, "Expected ')' after expression.");
            return Expression::Parenthesized {
                span: self.span_from_positions(
                    self.position_from_token(&token),
                    self.end_position_from_token(&right_paren),
                ),
                expression: Box::new(expression),
            };
        }

        self.error(&token, "Expected expression.");
        self.advance();
        Expression::Error {
            span: self.span_from_token(&token),
        }
    }

    fn parse_identifier(&mut self, message: &str) -> IdentifierNode {
        let token = self.consume(TokenKind::Identifier, message);
        IdentifierNode {
            name: token.text.clone(),
            span: self.span_from_token(&token),
        }
    }

    fn recover_to_right_bracket(&mut self) -> Token {
        while !self.check(TokenKind::RightBracket) && !self.check(TokenKind::Eof) {
            self.advance();
        }
        self.consume(
            TokenKind::RightBracket,
            "Expected ']' after sub-slice expression.",
        )
    }

    fn match_token(&mut self, kind: TokenKind) -> bool {
        if !self.check(kind) {
            return false;
        }
        self.advance();
        true
    }

    fn consume(&mut self, kind: TokenKind, message: &str) -> Token {
        if self.check(kind) {
            return self.advance();
        }

        let token = self.current().clone();
        self.error(&token, message);
        Token {
            kind,
            text: String::new(),
            line: token.line,
            column: token.column,
            start: token.start,
            end: token.start,
        }
    }

    fn check(&self, kind: TokenKind) -> bool {
        self.current().kind == kind
    }

    fn next_is(&self, kind: TokenKind) -> bool {
        self.tokens
            .get(self.index + 1)
            .is_some_and(|token| token.kind == kind)
    }

    fn advance(&mut self) -> Token {
        let token = self.current().clone();
        if !self.check(TokenKind::Eof) {
            self.index += 1;
        }
        token
    }

    fn previous(&self) -> &Token {
        self.tokens
            .get(self.index.saturating_sub(1))
            .unwrap_or_else(|| self.current())
    }

    fn current(&self) -> &Token {
        self.tokens
            .get(self.index)
            .unwrap_or_else(|| self.tokens.last().expect("lexer always emits EOF"))
    }

    fn error(&mut self, token: &Token, message: &str) {
        self.error_with_code(token, DiagnosticCode::Ck1001, message);
    }

    fn error_with_code(&mut self, token: &Token, code: DiagnosticCode, message: &str) {
        self.diagnostics.push(Diagnostic::error(
            code,
            message,
            self.source.file_name.clone(),
            self.span_from_token(token),
        ));
    }

    fn skip_braced_declaration(&mut self) {
        if !self.match_token(TokenKind::LeftBrace) {
            return;
        }
        let mut depth = 1_usize;
        while depth > 0 && !self.check(TokenKind::Eof) {
            let token = self.advance();
            match token.kind {
                TokenKind::LeftBrace => depth += 1,
                TokenKind::RightBrace => depth -= 1,
                _ => {}
            }
        }
    }

    fn synchronize_statement(&mut self) {
        while !self.check(TokenKind::Eof) {
            if self.match_token(TokenKind::Semicolon) || self.check(TokenKind::RightBrace) {
                return;
            }
            self.advance();
        }
    }

    fn synchronize_contract_clause(&mut self) {
        while !self.check(TokenKind::Eof) && !self.check(TokenKind::RightBrace) {
            if self.match_token(TokenKind::Semicolon) {
                return;
            }
            self.advance();
        }
    }

    fn span_from_token(&self, token: &Token) -> SourceSpan {
        SourceSpan {
            start: self.position_from_token(token),
            end: self.end_position_from_token(token),
        }
    }

    fn span_between_tokens(&self, start: &Token, end: &Token) -> SourceSpan {
        SourceSpan {
            start: self.position_from_token(start),
            end: self.end_position_from_token(end),
        }
    }

    fn span_from_positions(&self, start: SourcePosition, end: SourcePosition) -> SourceSpan {
        SourceSpan { start, end }
    }

    fn position_from_token(&self, token: &Token) -> SourcePosition {
        SourcePosition {
            offset: token.start,
            line: token.line,
            column: token.column,
        }
    }

    fn end_position_from_token(&self, token: &Token) -> SourcePosition {
        SourcePosition {
            offset: token.end,
            line: token.line,
            column: token.column + token.text.chars().count(),
        }
    }
}

fn binary_precedence(kind: TokenKind) -> u8 {
    match kind {
        TokenKind::PipePipe => 1,
        TokenKind::AmpAmp => 2,
        TokenKind::EqualEqual | TokenKind::BangEqual => 3,
        TokenKind::Less | TokenKind::LessEqual | TokenKind::Greater | TokenKind::GreaterEqual => 4,
        TokenKind::Plus | TokenKind::Minus => 5,
        TokenKind::Star | TokenKind::Slash | TokenKind::Percent => 6,
        _ => 0,
    }
}
