use std::collections::BTreeSet;

use calckernel::{
    CalcKernelType, CheckedProgram, EditorOccurrenceKind, SourceFile, SourceSpan, Statement,
    SymbolKind, TokenKind, TypeNode, analyze_editor, check, get_compiler_builtin, lex,
};
use serde_json::{Value, json};

pub(super) fn handle(method: &str, params: &Value, text: &str) -> Option<Value> {
    Some(match method {
        "textDocument/completion" => completion(params, text),
        "textDocument/hover" => hover(params, text),
        "textDocument/signatureHelp" => signature_help(params, text),
        "textDocument/foldingRange" => folding_ranges(text),
        "textDocument/selectionRange" => selection_ranges(params, text),
        _ => return None,
    })
}

fn completion(params: &Value, text: &str) -> Value {
    let Some(offset) = position_offset(text, &params["position"]) else {
        return json!([]);
    };
    let source = SourceFile::new("editor.ck", text);
    let analysis = analyze_editor(&source);
    let tokens = lex(&source).tokens;
    if let Some((index, prefix)) = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            if token.kind != TokenKind::Dot {
                return None;
            }
            if token.end == offset {
                return Some((index, ""));
            }
            let next = tokens.get(index + 1)?;
            if next.kind != TokenKind::Identifier || next.start > offset || offset > next.end {
                return None;
            }
            let length = offset - next.start;
            next.text.get(..length).map(|prefix| (index, prefix))
        })
        .next_back()
    {
        return field_completion(&source, &analysis, &tokens, index, prefix);
    }
    let mut visible_scopes = BTreeSet::new();
    let mut scope = analysis
        .scopes
        .iter()
        .rev()
        .filter(|scope| scope.span.start.offset <= offset && offset <= scope.span.end.offset)
        .min_by_key(|scope| scope.span.end.offset - scope.span.start.offset);
    while let Some(current) = scope {
        visible_scopes.insert(current.id.0);
        scope = current
            .parent
            .and_then(|id| analysis.scopes.iter().find(|candidate| candidate.id == id));
    }

    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for symbol in &analysis.symbols {
        if symbol.kind == SymbolKind::Field || !visible_scopes.contains(&symbol.scope_id.0) {
            continue;
        }
        if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Struct)
            && symbol.declaration.start.offset > offset
        {
            continue;
        }
        if seen.insert(symbol.name.clone()) {
            let kind = match symbol.kind {
                SymbolKind::Function => 3,
                SymbolKind::Struct => 7,
                SymbolKind::Parameter | SymbolKind::Local => 6,
                SymbolKind::Field => 5,
            };
            items.push(json!({"label": symbol.name, "kind": kind}));
        }
    }
    for name in [
        "i32_to_f64",
        "u32_to_f64",
        "print_i32",
        "print_i64",
        "print_u32",
        "print_u64",
        "print_f64",
        "print_bool",
        "print_newline",
    ] {
        if seen.insert(name.to_owned()) {
            items.push(json!({"label": name, "kind": 3}));
        }
    }
    for name in [
        "fn", "struct", "export", "unsafe", "contract", "requires", "effects", "none", "let", "if",
        "else", "while", "break", "continue", "return", "true", "false",
    ] {
        if seen.insert(name.to_owned()) {
            items.push(json!({"label": name, "kind": 14}));
        }
    }
    for name in [
        "i32", "i64", "u32", "u64", "f64", "bool", "void", "ptr", "slice",
    ] {
        if seen.insert(name.to_owned()) {
            items.push(json!({"label": name, "kind": 7}));
        }
    }
    Value::Array(items)
}

fn field_completion(
    source: &SourceFile,
    analysis: &calckernel::EditorAnalysis,
    tokens: &[calckernel::Token],
    dot_index: usize,
    prefix: &str,
) -> Value {
    let receiver = dot_index.checked_sub(1).and_then(|index| tokens.get(index));
    let checked = check(source);
    let receiver_type = receiver
        .and_then(|token| {
            checked
                .checked_program
                .types
                .iter()
                .filter(|(span, _)| span.end.offset == token.end)
                .min_by_key(|(span, _)| span.start.offset)
                .map(|(_, ty)| ty)
        })
        .or_else(|| {
            receiver
                .filter(|token| token.kind == TokenKind::Identifier)
                .and_then(|token| analysis.definition_at(token.start))
                .and_then(|symbol| declaration_type(&checked.checked_program, symbol.declaration))
        })
        .or_else(|| call_result_type(tokens, dot_index, &checked.checked_program));
    let struct_filter = match receiver_type {
        Some(CalcKernelType::Struct(name)) => Some(name.as_str()),
        _ => None,
    };
    let is_slice = matches!(receiver_type, Some(CalcKernelType::Slice(_)));
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    if is_slice {
        for name in ["data", "len"] {
            if !name.starts_with(prefix) {
                continue;
            }
            seen.insert(name);
            items.push(json!({"label": name, "kind": 10, "detail": "slice projection"}));
        }
    }
    if let Some(struct_name) = struct_filter {
        for structure in &checked.checked_program.structs {
            if struct_name != structure.name {
                continue;
            }
            for field in &structure.fields {
                if field.name.starts_with(prefix) && seen.insert(field.name.as_str()) {
                    items.push(json!({"label": field.name, "kind": 5, "detail": structure.name}));
                }
            }
        }
    }
    Value::Array(items)
}

fn call_result_type<'a>(
    tokens: &[calckernel::Token],
    dot_index: usize,
    program: &'a CheckedProgram,
) -> Option<&'a CalcKernelType> {
    let right = dot_index.checked_sub(1)?;
    if tokens.get(right)?.kind != TokenKind::RightParen {
        return None;
    }
    let mut depth = 0;
    for index in (0..=right).rev() {
        match tokens[index].kind {
            TokenKind::RightParen => depth += 1,
            TokenKind::LeftParen => {
                depth -= 1;
                if depth == 0 {
                    let callee = index
                        .checked_sub(1)
                        .and_then(|previous| tokens.get(previous))?;
                    if callee.kind == TokenKind::Identifier {
                        return program
                            .function_map
                            .get(&callee.text)
                            .map(|function| &function.return_type);
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn declaration_type(program: &CheckedProgram, span: SourceSpan) -> Option<&CalcKernelType> {
    for function in &program.functions {
        for parameter in &function.params {
            if parameter.declaration.name.span == span {
                return Some(&parameter.type_node);
            }
        }
        if let Some(ty) =
            local_declaration_type(&function.declaration.body.statements, span, program)
        {
            return Some(ty);
        }
    }
    None
}

fn local_declaration_type<'a>(
    statements: &[Statement],
    span: SourceSpan,
    program: &'a CheckedProgram,
) -> Option<&'a CalcKernelType> {
    for statement in statements {
        match statement {
            Statement::Let(local) if local.name.span == span => {
                return program.local_types.get(&local.span);
            }
            Statement::Block(block) => {
                if let Some(ty) = local_declaration_type(&block.statements, span, program) {
                    return Some(ty);
                }
            }
            Statement::Unsafe(unsafe_statement) => {
                if let Some(ty) =
                    local_declaration_type(&unsafe_statement.block.statements, span, program)
                {
                    return Some(ty);
                }
            }
            Statement::If(branch) => {
                if let Some(ty) =
                    local_declaration_type(&branch.then_block.statements, span, program)
                {
                    return Some(ty);
                }
                if let Some(other) = &branch.else_block
                    && let Some(ty) = local_declaration_type(&other.statements, span, program)
                {
                    return Some(ty);
                }
            }
            Statement::While(loop_statement) => {
                if let Some(ty) =
                    local_declaration_type(&loop_statement.body.statements, span, program)
                {
                    return Some(ty);
                }
            }
            _ => {}
        }
    }
    None
}

fn hover(params: &Value, text: &str) -> Value {
    let Some(offset) = position_offset(text, &params["position"]) else {
        return Value::Null;
    };
    let source = SourceFile::new("editor.ck", text);
    let analysis = analyze_editor(&source);
    let Some(occurrence) = analysis.occurrences.iter().find(|occurrence| {
        occurrence.span.start.offset <= offset && offset < occurrence.span.end.offset
    }) else {
        return Value::Null;
    };
    let checked = check(&source);
    let label = if occurrence.kind == EditorOccurrenceKind::Builtin {
        get_compiler_builtin(&occurrence.name).map(|builtin| {
            let args = builtin
                .params
                .iter()
                .map(type_label)
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "fn {}({args}) -> {}",
                builtin.name,
                type_label(&builtin.return_type)
            )
        })
    } else {
        analysis
            .definition_at(offset)
            .map(|symbol| match symbol.kind {
                SymbolKind::Function => checked
                    .symbols
                    .functions
                    .get(&symbol.name)
                    .map(|function| {
                        let params = function
                            .declaration
                            .params
                            .iter()
                            .map(|parameter| {
                                format!(
                                    "{}: {}",
                                    parameter.name.name,
                                    type_node_label(&parameter.type_node)
                                )
                            })
                            .collect::<Vec<_>>()
                            .join(", ");
                        let prefix = if function.declaration.is_unsafe {
                            "unsafe "
                        } else {
                            ""
                        };
                        format!(
                            "{prefix}fn {}({params}) -> {}",
                            symbol.name,
                            type_label(&function.return_type)
                        )
                    })
                    .unwrap_or_else(|| format!("fn {}", symbol.name)),
                SymbolKind::Struct => format!("struct {}", symbol.name),
                SymbolKind::Field => format!("field {}", symbol.name),
                SymbolKind::Parameter => format!("parameter {}", symbol.name),
                SymbolKind::Local => format!("local {}", symbol.name),
            })
    };
    label.map_or(Value::Null, |label| {
        json!({"contents": {"kind": "markdown", "value": format!("```ck\n{label}\n```")}, "range": span_range(occurrence.span)})
    })
}

fn signature_help(params: &Value, text: &str) -> Value {
    let Some(offset) = position_offset(text, &params["position"]) else {
        return Value::Null;
    };
    let source = SourceFile::new("editor.ck", text);
    let tokens = lex(&source).tokens;
    let mut stack = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LeftParen => stack.push(index),
            TokenKind::RightParen => {
                stack.pop();
            }
            _ => {}
        }
    }
    let Some(&open) = stack.last() else {
        return Value::Null;
    };
    let Some(callee) = open.checked_sub(1).and_then(|index| tokens.get(index)) else {
        return Value::Null;
    };
    if callee.kind != TokenKind::Identifier {
        return Value::Null;
    }
    let checked = check(&source);
    let (parameters, result) = if let Some(function) = checked.symbols.functions.get(&callee.text) {
        (
            function
                .declaration
                .params
                .iter()
                .map(|param| format!("{}: {}", param.name.name, type_node_label(&param.type_node)))
                .collect::<Vec<_>>(),
            type_label(&function.return_type),
        )
    } else if let Some(builtin) = get_compiler_builtin(&callee.text) {
        (
            builtin.params.iter().map(type_label).collect::<Vec<_>>(),
            type_label(&builtin.return_type),
        )
    } else {
        return Value::Null;
    };

    let mut nested = 0usize;
    let mut active = 0usize;
    for token in tokens.iter().skip(open + 1) {
        if token.start >= offset {
            break;
        }
        match token.kind {
            TokenKind::LeftParen | TokenKind::LeftBracket | TokenKind::LeftBrace => nested += 1,
            TokenKind::RightParen | TokenKind::RightBracket | TokenKind::RightBrace => {
                nested = nested.saturating_sub(1);
            }
            TokenKind::Comma if nested == 0 => active += 1,
            _ => {}
        }
    }
    let label = format!("{}({}) -> {result}", callee.text, parameters.join(", "));
    json!({"signatures": [{"label": label, "parameters": parameters.iter().map(|p| json!({"label": p})).collect::<Vec<_>>()}], "activeSignature": 0, "activeParameter": active.min(parameters.len().saturating_sub(1))})
}

fn folding_ranges(text: &str) -> Value {
    let source = SourceFile::new("editor.ck", text);
    let mut stack = Vec::new();
    let mut ranges = Vec::new();
    for token in lex(&source).tokens {
        match token.kind {
            TokenKind::LeftBrace => stack.push(token.line),
            TokenKind::RightBrace => {
                if let Some(start) = stack.pop()
                    && token.line > start
                {
                    ranges.push(json!({"startLine": start - 1, "endLine": token.line - 1}));
                }
            }
            _ => {}
        }
    }
    Value::Array(ranges)
}

fn selection_ranges(params: &Value, text: &str) -> Value {
    let source = SourceFile::new("editor.ck", text);
    let analysis = analyze_editor(&source);
    let line_count = text.split('\n').count();
    let last_line_len = text
        .rsplit('\n')
        .next()
        .unwrap_or("")
        .encode_utf16()
        .count();
    let document_range = json!({"start": {"line": 0, "character": 0}, "end": {"line": line_count - 1, "character": last_line_len}});
    let Some(positions) = params["positions"].as_array() else {
        return json!([]);
    };
    Value::Array(positions.iter().map(|position| {
        let Some(offset) = position_offset(text, position) else {
            return json!({"range": document_range});
        };
        let line = position["line"].as_u64().unwrap_or(0) as usize;
        let line_text = text.split('\n').nth(line).unwrap_or("");
        let line_range = json!({"start": {"line": line, "character": 0}, "end": {"line": line, "character": line_text.trim_end_matches('\r').encode_utf16().count()}});
        let parent = if line_count > 1 { json!({"range": line_range, "parent": {"range": document_range}}) } else { json!({"range": line_range}) };
        analysis.occurrences.iter().find(|occurrence| {
            occurrence.span.start.offset <= offset && offset < occurrence.span.end.offset
        }).map_or(parent.clone(), |occurrence| json!({"range": span_range(occurrence.span), "parent": parent}))
    }).collect())
}

fn position_offset(text: &str, position: &Value) -> Option<usize> {
    let line = usize::try_from(position["line"].as_u64()?).ok()?;
    let character = usize::try_from(position["character"].as_u64()?).ok()?;
    let mut offset = 0;
    for (index, content) in text.split('\n').enumerate() {
        if index == line {
            if character > content.trim_end_matches('\r').encode_utf16().count() {
                return None;
            }
            return Some(offset + character);
        }
        offset += content.encode_utf16().count() + 1;
    }
    None
}

fn span_range(span: SourceSpan) -> Value {
    json!({"start": {"line": span.start.line - 1, "character": span.start.column - 1}, "end": {"line": span.end.line - 1, "character": span.end.column - 1}})
}

fn type_node_label(ty: &TypeNode) -> String {
    match ty {
        TypeNode::Primitive { name, .. } => name.clone(),
        TypeNode::Void { .. } => "void".to_owned(),
        TypeNode::Pointer { element_type, .. } => format!("ptr<{}>", type_node_label(element_type)),
        TypeNode::Slice { element_type, .. } => format!("slice<{}>", type_node_label(element_type)),
        TypeNode::Named { name, .. } => name.name.clone(),
        TypeNode::Error { .. } => "?".to_owned(),
    }
}

fn type_label(ty: &CalcKernelType) -> String {
    match ty {
        CalcKernelType::Primitive(name) => format!("{name:?}").to_lowercase(),
        CalcKernelType::Pointer(inner) => format!("ptr<{}>", type_label(inner)),
        CalcKernelType::Slice(inner) => format!("slice<{}>", type_label(inner)),
        CalcKernelType::Struct(name) => name.clone(),
        CalcKernelType::Void => "void".to_owned(),
        CalcKernelType::IntegerLiteral => "integer".to_owned(),
        CalcKernelType::Unknown => "?".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_includes_visible_symbols_and_ck_keywords() {
        let text = "fn outer(value: i32) -> i32 { let local: i32 = value; return local; }";
        let position =
            json!({"position": {"line": 0, "character": text.find("return").unwrap() + 7}});
        let items = handle("textDocument/completion", &position, text).unwrap();
        let labels: Vec<_> = items
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"local"));
        assert!(labels.contains(&"value"));
        assert!(labels.contains(&"unsafe"));
    }

    #[test]
    fn dot_completion_uses_the_receiver_struct_or_slice() {
        let text = "struct A { first: i32; } struct B { second: i32; } fn f(item: A) -> i32 { return item.first; }";
        let cursor = text.find("item.first").unwrap() + "item.".len();
        let result = handle(
            "textDocument/completion",
            &json!({"position": {"line": 0, "character": cursor}}),
            text,
        )
        .unwrap();
        let labels: Vec<_> = result
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert!(labels.contains(&"first"));
        assert!(!labels.contains(&"second"));
        assert!(!labels.contains(&"return"));

        let local_text = "struct A { first: i32; } struct B { second: i32; } fn f(item: A) -> i32 { let local: A = item; return local.first; }";
        let cursor = local_text.find("local.first").unwrap() + "local.".len();
        let local_result = handle(
            "textDocument/completion",
            &json!({"position": {"line": 0, "character": cursor}}),
            local_text,
        )
        .unwrap();
        let local_labels: Vec<_> = local_result
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert_eq!(local_labels, vec!["first"]);

        let prefix_result = handle(
            "textDocument/completion",
            &json!({"position": {"line": 0, "character": cursor + 2}}),
            local_text,
        )
        .unwrap();
        let prefix_labels: Vec<_> = prefix_result
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert_eq!(prefix_labels, vec!["first"]);

        let incomplete = text.replace("item.first", "item.");
        let cursor = incomplete.find("item.").unwrap() + "item.".len();
        let result = handle(
            "textDocument/completion",
            &json!({"position": {"line": 0, "character": cursor}}),
            &incomplete,
        )
        .unwrap();
        assert!(
            result
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["label"] == "first")
        );
    }

    #[test]
    fn dot_completion_follows_field_and_call_result_types() {
        let text = "struct Inner { value: i32; } struct Outer { inner: Inner; } fn make(input: Outer) -> Outer { return input; } fn f(input: Outer) -> i32 { return make(input).inner.value; }";
        let cursor = text.find(".inner.value").unwrap() + ".inner.".len();
        let result = handle(
            "textDocument/completion",
            &json!({"position": {"line": 0, "character": cursor}}),
            text,
        )
        .unwrap();
        let labels: Vec<_> = result
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, vec!["value"]);

        let direct = "struct Inner { value: i32; } fn make(item: Inner) -> Inner { return item; } fn f(item: Inner) -> i32 { return make(item).value; }";
        let cursor = direct.find("make(item).value").unwrap() + "make(item).".len();
        let result = handle(
            "textDocument/completion",
            &json!({"position": {"line": 0, "character": cursor}}),
            direct,
        )
        .unwrap();
        let labels: Vec<_> = result
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["label"].as_str().unwrap())
            .collect();
        assert_eq!(labels, vec!["value"]);
    }

    #[test]
    fn signature_help_counts_commas_outside_nested_calls() {
        let text =
            "fn f(a: i32, b: i32) -> i32 { return a + b; } fn g() -> i32 { return f(f(1, 2), 3); }";
        let cursor = text.find(", 3").unwrap() + 2;
        let result = handle(
            "textDocument/signatureHelp",
            &json!({"position": {"line": 0, "character": cursor}}),
            text,
        )
        .unwrap();
        assert_eq!(result["activeParameter"], 1);
        assert_eq!(result["signatures"][0]["label"], "f(a: i32, b: i32) -> i32");
    }

    #[test]
    fn hover_and_selection_use_utf16_positions() {
        let text = "// 😀\nfn f(value: i32) -> i32 { return value; }";
        let result = handle(
            "textDocument/hover",
            &json!({"position": {"line": 1, "character": 3}}),
            text,
        )
        .unwrap();
        assert!(result["contents"]["value"].as_str().unwrap().contains("f("));
        let selections = handle(
            "textDocument/selectionRange",
            &json!({"positions": [{"line": 1, "character": 3}]}),
            text,
        )
        .unwrap();
        assert_eq!(selections[0]["range"]["start"]["line"], 1);
    }
}
