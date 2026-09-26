use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, BufWriter, Write},
};

use calckernel::{Diagnostic, SourceFile, check};
use serde_json::{Value, json};

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_URI_BYTES: usize = 4 * 1024;
const MAX_SYNTAX_NESTING: usize = 256;
const MAX_SOURCE_TOKENS: usize = 50_000;
const MAX_LEXICAL_ERRORS: usize = 256;
const MAX_TOKEN_BYTES: usize = 64 * 1024;
const MAX_UNARY_OPERATOR_CHAIN: usize = 256;
const MAX_BINARY_OPERATOR_CHAIN: usize = 128;
const MAX_POSTFIX_CHAIN: usize = 256;

pub(super) fn run() -> i32 {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let input = BufReader::new(stdin.lock());
    let output = BufWriter::new(stdout.lock());

    match serve(input, output) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("ckc lsp: {error}");
            1
        }
    }
}

fn serve(mut input: impl BufRead, mut output: impl Write) -> io::Result<i32> {
    let mut state = ServerState::default();

    while let Some(body) = read_frame(&mut input)? {
        let message = match serde_json::from_slice::<Value>(&body) {
            Ok(message) => message,
            Err(_) => {
                write_response(&mut output, Value::Null, -32700, "Parse error")?;
                continue;
            }
        };

        if let Some(exit_code) = handle_message(message, &mut state, &mut output)? {
            return Ok(exit_code);
        }
    }

    Ok(0)
}

#[derive(Default)]
pub(super) struct ServerState {
    pub(super) documents: HashMap<String, DocumentSnapshot>,
    shutdown_requested: bool,
    initialize_received: bool,
    initialized: bool,
}

pub(super) struct DocumentSnapshot {
    pub(super) version: i64,
    pub(super) _text: Option<String>,
}

fn handle_message(
    message: Value,
    state: &mut ServerState,
    output: &mut impl Write,
) -> io::Result<Option<i32>> {
    let Some(object) = message.as_object() else {
        write_response(output, Value::Null, -32600, "Invalid Request")?;
        return Ok(None);
    };
    let id = object.get("id").cloned();
    if object.get("jsonrpc") != Some(&Value::String("2.0".to_string())) {
        write_response(output, id.unwrap_or(Value::Null), -32600, "Invalid Request")?;
        return Ok(None);
    }
    let Some(method) = object.get("method").and_then(Value::as_str) else {
        write_response(output, id.unwrap_or(Value::Null), -32600, "Invalid Request")?;
        return Ok(None);
    };

    if method == "exit" {
        return Ok(Some(i32::from(!state.shutdown_requested)));
    }
    if method == "initialize" {
        let Some(id) = id else {
            return Ok(None);
        };
        if state.initialize_received {
            write_response(output, id, -32600, "Initialize may only be requested once")?;
            return Ok(None);
        }
        state.initialize_received = true;
        write_initialize_response(output, id)?;
        return Ok(None);
    }
    if method == "initialized" {
        if state.initialize_received {
            state.initialized = true;
        }
        return Ok(None);
    }
    if !state.initialize_received {
        if let Some(id) = id {
            write_response(output, id, -32002, "Server not initialized")?;
        }
        return Ok(None);
    }
    if state.shutdown_requested && method != "exit" {
        if let Some(id) = id {
            write_response(output, id, -32600, "Server has shut down")?;
        }
        return Ok(None);
    }
    if !state.initialized {
        if let Some(id) = id {
            write_response(output, id, -32002, "Server not initialized")?;
        }
        return Ok(None);
    }

    let params = object.get("params").unwrap_or(&Value::Null);
    match method {
        "initialize" | "initialized" | "exit" => {}
        "shutdown" => {
            state.shutdown_requested = true;
            if let Some(id) = id {
                write_message(output, &json!({"jsonrpc": "2.0", "id": id, "result": null}))?;
            }
        }
        "textDocument/didOpen" => match opened_document(params) {
            Ok(Some((uri, version, text))) => {
                analyze_and_publish(&uri, version, text, state, output)?;
            }
            Err(DocumentInputError::UriTooLong) => {
                warn_document_skipped(output, "URI exceeds the 4 KiB limit")?;
            }
            Ok(None) => {}
        },
        "textDocument/didChange" => match changed_document(params) {
            Ok(Some((uri, version, text))) => {
                let is_newer = state
                    .documents
                    .get(&uri)
                    .is_some_and(|document| version > document.version);
                if is_newer {
                    analyze_and_publish(&uri, version, text, state, output)?;
                }
            }
            Err(DocumentInputError::UriTooLong) => {
                warn_document_skipped(output, "URI exceeds the 4 KiB limit")?;
            }
            Ok(None) => {}
        },
        "textDocument/didClose" => match document_uri(params) {
            Ok(Some(uri)) => {
                state.documents.remove(&uri);
                publish_diagnostics(output, &uri, None, &[])?;
            }
            Err(DocumentInputError::UriTooLong) => {
                warn_document_skipped(output, "URI exceeds the 4 KiB limit")?;
            }
            Ok(None) => {}
        },
        _ => {
            if let Some(id) = id {
                write_response(output, id, -32601, &format!("Method not found: {method}"))?;
            }
        }
    }

    Ok(None)
}

fn write_initialize_response(output: &mut impl Write, id: Value) -> io::Result<()> {
    write_message(
        output,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "capabilities": {
                    "textDocumentSync": {
                        "openClose": true,
                        "change": 1
                    }
                },
                "serverInfo": {
                    "name": "ckc",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DocumentInputError {
    UriTooLong,
}

fn opened_document(params: &Value) -> Result<Option<(String, i64, &str)>, DocumentInputError> {
    let Some(document) = params.get("textDocument") else {
        return Ok(None);
    };
    let Some(uri) = document_uri_value(document)? else {
        return Ok(None);
    };
    let (Some(version), Some(text)) = (
        document.get("version").and_then(Value::as_i64),
        document.get("text").and_then(Value::as_str),
    ) else {
        return Ok(None);
    };
    Ok(Some((uri, version, text)))
}

fn changed_document(params: &Value) -> Result<Option<(String, i64, &str)>, DocumentInputError> {
    let Some(document) = params.get("textDocument") else {
        return Ok(None);
    };
    let Some(uri) = document_uri_value(document)? else {
        return Ok(None);
    };
    let (Some(version), Some(change)) = (
        document.get("version").and_then(Value::as_i64),
        params
            .get("contentChanges")
            .and_then(Value::as_array)
            .and_then(|changes| changes.last()),
    ) else {
        return Ok(None);
    };
    let Some(text) = change.get("text").and_then(Value::as_str) else {
        return Ok(None);
    };
    Ok(Some((uri, version, text)))
}

fn document_uri(params: &Value) -> Result<Option<String>, DocumentInputError> {
    let Some(document) = params.get("textDocument") else {
        return Ok(None);
    };
    document_uri_value(document)
}

fn document_uri_value(document: &Value) -> Result<Option<String>, DocumentInputError> {
    let Some(uri) = document.get("uri").and_then(Value::as_str) else {
        return Ok(None);
    };
    if uri.len() > MAX_URI_BYTES {
        return Err(DocumentInputError::UriTooLong);
    }
    Ok(Some(uri.to_string()))
}

fn analyze_and_publish(
    uri: &str,
    version: i64,
    text: &str,
    state: &mut ServerState,
    output: &mut impl Write,
) -> io::Result<()> {
    if text.len() > MAX_SOURCE_BYTES {
        remember_unanalyzed_document(uri, version, state);
        warn_document_skipped(output, "source exceeds the 4 MiB limit")?;
        return publish_diagnostics(output, uri, Some(version), &[]);
    }

    if let Some(warning) = analysis_limit_warning(text) {
        remember_unanalyzed_document(uri, version, state);
        warn_document_skipped(output, warning)?;
        return publish_diagnostics(output, uri, Some(version), &[]);
    }

    // The LSP URI is already carried by publishDiagnostics. Keeping it out of
    // SourceFile prevents every compiler diagnostic from cloning a long URI.
    let source = SourceFile::new("editor.ck", text.to_string());
    let diagnostics = check(&source).diagnostics;
    state.documents.insert(
        uri.to_string(),
        DocumentSnapshot {
            version,
            _text: Some(source.text),
        },
    );
    publish_diagnostics(output, uri, Some(version), &diagnostics)
}

fn remember_unanalyzed_document(uri: &str, version: i64, state: &mut ServerState) {
    state.documents.insert(
        uri.to_string(),
        DocumentSnapshot {
            version,
            _text: None,
        },
    );
}

fn warn_document_skipped(output: &mut impl Write, reason: &str) -> io::Result<()> {
    write_message(
        output,
        &json!({
            "jsonrpc": "2.0",
            "method": "window/logMessage",
            "params": {
                "type": 2,
                "message": format!("ckc lsp skipped document analysis: {reason}.")
            }
        }),
    )
}

fn analysis_limit_warning(source: &str) -> Option<&'static str> {
    const ROUND: u8 = 1;
    const SQUARE: u8 = 2;
    const CURLY: u8 = 3;
    const TYPE_ANGLE: u8 = 4;

    let bytes = source.as_bytes();
    let mut offset = 0;
    let mut token_count = 0;
    let mut lexical_error_count = 0;
    let mut unary_chain = 0;
    let mut binary_operator_chain = 0;
    let mut postfix_chain = 0;
    let mut previous_expression_end = false;
    let mut pending_type_angle = false;
    let mut nesting = [0; MAX_SYNTAX_NESTING];
    let mut nesting_depth = 0;

    while offset < bytes.len() {
        let byte = bytes[offset];

        if matches!(byte, b' ' | b'\r' | b'\t' | b'\n') {
            offset += 1;
            continue;
        }
        if byte == b'/' && bytes.get(offset + 1) == Some(&b'/') {
            offset += 2;
            while offset < bytes.len() && bytes[offset] != b'\n' {
                offset += 1;
            }
            continue;
        }

        if byte >= 0x80 {
            let character = source[offset..].chars().next()?;
            lexical_error_count += character.len_utf16();
            if lexical_error_count > MAX_LEXICAL_ERRORS {
                return Some("more than 256 lexical errors");
            }
            offset += character.len_utf8();
            unary_chain = 0;
            postfix_chain = 0;
            previous_expression_end = false;
            pending_type_angle = false;
            continue;
        }

        let mut current_is_type_angle = false;
        if pending_type_angle {
            if byte == b'<' {
                if !push_syntax_nesting(&mut nesting, &mut nesting_depth, TYPE_ANGLE) {
                    return Some("syntax nesting exceeds the safe limit of 256");
                }
                current_is_type_angle = true;
            }
            pending_type_angle = false;
        }

        if is_ascii_identifier_start(byte) {
            let start = offset;
            offset += 1;
            while offset < bytes.len() && is_ascii_identifier_part(bytes[offset]) {
                offset += 1;
            }
            if offset - start > MAX_TOKEN_BYTES {
                return Some("a source token exceeds the 64 KiB limit");
            }
            if count_source_token(&mut token_count) {
                return Some("source exceeds the 50,000 token analysis budget");
            }
            let identifier = &source[start..offset];
            pending_type_angle = matches!(identifier, "ptr" | "slice");
            previous_expression_end = identifier_can_end_expression(identifier);
            if !previous_expression_end {
                postfix_chain = 0;
            }
            unary_chain = 0;
            continue;
        }

        if byte.is_ascii_digit() {
            let start = offset;
            let malformed = scan_numeric_token(bytes, &mut offset);
            if offset - start > MAX_TOKEN_BYTES {
                return Some("a source token exceeds the 64 KiB limit");
            }
            if malformed {
                lexical_error_count += 1;
                if lexical_error_count > MAX_LEXICAL_ERRORS {
                    return Some("more than 256 lexical errors");
                }
            }
            if count_source_token(&mut token_count) {
                return Some("source exceeds the 50,000 token analysis budget");
            }
            previous_expression_end = true;
            unary_chain = 0;
            continue;
        }

        pending_type_angle = false;
        match byte {
            b'(' | b'[' | b'{' => {
                if byte == b'(' || byte == b'[' {
                    if previous_expression_end {
                        if increment_postfix_chain(&mut postfix_chain) {
                            return Some("postfix chain exceeds the safe limit of 256");
                        }
                    } else {
                        postfix_chain = 0;
                    }
                } else {
                    postfix_chain = 0;
                }
                let kind = match byte {
                    b'(' => ROUND,
                    b'[' => SQUARE,
                    _ => CURLY,
                };
                if !push_syntax_nesting(&mut nesting, &mut nesting_depth, kind) {
                    return Some("syntax nesting exceeds the safe limit of 256");
                }
                offset += 1;
                binary_operator_chain = 0;
                previous_expression_end = false;
                unary_chain = 0;
            }
            b')' | b']' | b'}' => {
                let kind = match byte {
                    b')' => ROUND,
                    b']' => SQUARE,
                    _ => CURLY,
                };
                pop_syntax_nesting(&nesting, &mut nesting_depth, kind);
                offset += 1;
                binary_operator_chain = 0;
                previous_expression_end = byte != b'}';
                if byte == b'}' {
                    postfix_chain = 0;
                }
                unary_chain = 0;
            }
            b'>' => {
                if bytes.get(offset + 1) == Some(&b'=') {
                    offset += 2;
                    if increment_binary_operator_chain(&mut binary_operator_chain) {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                } else if nesting_depth > 0 && nesting[nesting_depth - 1] == TYPE_ANGLE {
                    pop_syntax_nesting(&nesting, &mut nesting_depth, TYPE_ANGLE);
                    offset += 1;
                } else {
                    offset += 1;
                    if increment_binary_operator_chain(&mut binary_operator_chain) {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                }
                postfix_chain = 0;
                previous_expression_end = false;
                unary_chain = 0;
            }
            b'!' => {
                if bytes.get(offset + 1) == Some(&b'=') {
                    offset += 2;
                    if increment_binary_operator_chain(&mut binary_operator_chain) {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                    postfix_chain = 0;
                    previous_expression_end = false;
                    unary_chain = 0;
                } else {
                    unary_chain += 1;
                    offset += 1;
                    postfix_chain = 0;
                    previous_expression_end = false;
                    if unary_chain > MAX_UNARY_OPERATOR_CHAIN {
                        return Some("unary operator chain exceeds the safe limit of 256");
                    }
                }
            }
            b'-' => {
                if bytes.get(offset + 1) == Some(&b'>') {
                    offset += 2;
                    binary_operator_chain = 0;
                    postfix_chain = 0;
                    previous_expression_end = false;
                    unary_chain = 0;
                } else {
                    if previous_expression_end
                        && increment_binary_operator_chain(&mut binary_operator_chain)
                    {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                    unary_chain += 1;
                    offset += 1;
                    postfix_chain = 0;
                    previous_expression_end = false;
                    if unary_chain > MAX_UNARY_OPERATOR_CHAIN {
                        return Some("unary operator chain exceeds the safe limit of 256");
                    }
                }
            }
            b'&' | b'|' => {
                let pair = if byte == b'&' { b'&' } else { b'|' };
                if bytes.get(offset + 1) == Some(&pair) {
                    offset += 2;
                    if increment_binary_operator_chain(&mut binary_operator_chain) {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                } else {
                    offset += 1;
                    lexical_error_count += 1;
                    if lexical_error_count > MAX_LEXICAL_ERRORS {
                        return Some("more than 256 lexical errors");
                    }
                }
                postfix_chain = 0;
                previous_expression_end = false;
                unary_chain = 0;
            }
            b'.' => {
                if bytes.get(offset + 1) == Some(&b'.') {
                    offset += 2;
                    postfix_chain = 0;
                    previous_expression_end = false;
                } else if bytes.get(offset + 1).is_some_and(u8::is_ascii_digit) {
                    offset += 1;
                    while offset < bytes.len() && bytes[offset].is_ascii_digit() {
                        offset += 1;
                    }
                    lexical_error_count += 1;
                    if lexical_error_count > MAX_LEXICAL_ERRORS {
                        return Some("more than 256 lexical errors");
                    }
                    postfix_chain = 0;
                    previous_expression_end = false;
                } else {
                    if previous_expression_end && increment_postfix_chain(&mut postfix_chain) {
                        return Some("postfix chain exceeds the safe limit of 256");
                    }
                    offset += 1;
                    previous_expression_end = false;
                }
                unary_chain = 0;
            }
            b'<' => {
                if bytes.get(offset + 1) == Some(&b'=') {
                    offset += 2;
                    if increment_binary_operator_chain(&mut binary_operator_chain) {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                } else {
                    offset += 1;
                    if !current_is_type_angle
                        && increment_binary_operator_chain(&mut binary_operator_chain)
                    {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                }
                postfix_chain = 0;
                previous_expression_end = false;
                unary_chain = 0;
            }
            byte if is_supported_punctuation(byte) => {
                let paired_equal = byte == b'=' && bytes.get(offset + 1) == Some(&b'=');
                offset += usize::from(paired_equal) + 1;
                if paired_equal {
                    if increment_binary_operator_chain(&mut binary_operator_chain) {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                } else if matches!(byte, b'+' | b'*' | b'/' | b'%') {
                    if increment_binary_operator_chain(&mut binary_operator_chain) {
                        return Some("binary operator chain exceeds the safe limit of 128");
                    }
                } else {
                    binary_operator_chain = 0;
                }
                postfix_chain = 0;
                previous_expression_end = false;
                unary_chain = 0;
            }
            _ => {
                offset += 1;
                lexical_error_count += 1;
                if lexical_error_count > MAX_LEXICAL_ERRORS {
                    return Some("more than 256 lexical errors");
                }
                binary_operator_chain = 0;
                postfix_chain = 0;
                previous_expression_end = false;
                unary_chain = 0;
            }
        }

        if count_source_token(&mut token_count) {
            return Some("source exceeds the 50,000 token analysis budget");
        }
    }

    None
}

fn is_ascii_identifier_start(byte: u8) -> bool {
    byte == b'_' || byte.is_ascii_alphabetic()
}

fn is_ascii_identifier_part(byte: u8) -> bool {
    is_ascii_identifier_start(byte) || byte.is_ascii_digit()
}

fn identifier_can_end_expression(identifier: &str) -> bool {
    !matches!(
        identifier,
        "struct"
            | "export"
            | "unsafe"
            | "fn"
            | "contract"
            | "requires"
            | "effects"
            | "let"
            | "return"
            | "break"
            | "continue"
            | "if"
            | "else"
            | "while"
            | "i32"
            | "i64"
            | "u32"
            | "u64"
            | "f64"
            | "bool"
            | "void"
            | "ptr"
    )
}

fn increment_binary_operator_chain(chain: &mut usize) -> bool {
    *chain += 1;
    *chain > MAX_BINARY_OPERATOR_CHAIN
}

fn increment_postfix_chain(chain: &mut usize) -> bool {
    *chain += 1;
    *chain > MAX_POSTFIX_CHAIN
}

fn is_supported_punctuation(byte: u8) -> bool {
    matches!(
        byte,
        b',' | b':' | b';' | b'+' | b'*' | b'/' | b'%' | b'=' | b'<' | b'>'
    )
}

fn scan_numeric_token(bytes: &[u8], offset: &mut usize) -> bool {
    while *offset < bytes.len() && bytes[*offset].is_ascii_digit() {
        *offset += 1;
    }

    if bytes.get(*offset) == Some(&b'.') && bytes.get(*offset + 1) != Some(&b'.') {
        *offset += 1;
        if !bytes.get(*offset).is_some_and(u8::is_ascii_digit) {
            return true;
        }
        while *offset < bytes.len() && bytes[*offset].is_ascii_digit() {
            *offset += 1;
        }
    }

    if bytes
        .get(*offset)
        .is_some_and(|byte| matches!(byte, b'e' | b'E'))
    {
        *offset += 1;
        if bytes
            .get(*offset)
            .is_some_and(|byte| matches!(byte, b'+' | b'-'))
        {
            *offset += 1;
        }
        if !bytes.get(*offset).is_some_and(u8::is_ascii_digit) {
            return true;
        }
        while *offset < bytes.len() && bytes[*offset].is_ascii_digit() {
            *offset += 1;
        }
    }

    false
}

fn count_source_token(token_count: &mut usize) -> bool {
    *token_count += 1;
    *token_count > MAX_SOURCE_TOKENS
}

fn push_syntax_nesting(
    nesting: &mut [u8; MAX_SYNTAX_NESTING],
    depth: &mut usize,
    kind: u8,
) -> bool {
    if *depth == nesting.len() {
        return false;
    }
    nesting[*depth] = kind;
    *depth += 1;
    true
}

fn pop_syntax_nesting(nesting: &[u8; MAX_SYNTAX_NESTING], depth: &mut usize, kind: u8) {
    if *depth > 0 && nesting[*depth - 1] == kind {
        *depth -= 1;
    }
}

fn publish_diagnostics(
    output: &mut impl Write,
    uri: &str,
    version: Option<i64>,
    diagnostics: &[Diagnostic],
) -> io::Result<()> {
    let empty_message = diagnostics_message(uri, version, &[]);
    let base_size = serde_json::to_vec(&empty_message)
        .map_err(io::Error::other)?
        .len();
    let mut diagnostics_json = Vec::new();
    let mut diagnostics_bytes = 0usize;
    let mut truncated = false;

    for diagnostic in diagnostics {
        let diagnostic = diagnostic_json(diagnostic);
        let diagnostic_size = serde_json::to_vec(&diagnostic)
            .map_err(io::Error::other)?
            .len();
        let added_size = diagnostic_size + usize::from(!diagnostics_json.is_empty());
        if base_size
            .saturating_add(diagnostics_bytes)
            .saturating_add(added_size)
            > MAX_FRAME_BYTES
        {
            truncated = true;
            break;
        }
        diagnostics_bytes += added_size;
        diagnostics_json.push(diagnostic);
    }

    if truncated {
        write_message(
            output,
            &json!({
                "jsonrpc": "2.0",
                "method": "window/logMessage",
                "params": {
                    "type": 2,
                    "message": "ckc lsp truncated diagnostics for a document because its publishDiagnostics response exceeded the 8 MiB limit."
                }
            }),
        )?;
    }

    write_message(
        output,
        &diagnostics_message(uri, version, &diagnostics_json),
    )
}

fn diagnostics_message(uri: &str, version: Option<i64>, diagnostics: &[Value]) -> Value {
    let mut params = json!({"uri": uri, "diagnostics": diagnostics});
    if let Some(version) = version {
        params["version"] = json!(version);
    }
    json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": params
    })
}

fn diagnostic_json(diagnostic: &Diagnostic) -> Value {
    json!({
        "range": {
            "start": {
                "line": diagnostic.span.start.line.saturating_sub(1),
                "character": diagnostic.span.start.column.saturating_sub(1)
            },
            "end": {
                "line": diagnostic.span.end.line.saturating_sub(1),
                "character": diagnostic.span.end.column.saturating_sub(1)
            }
        },
        "severity": 1,
        "code": diagnostic.code.to_string(),
        "source": "ckc",
        "message": diagnostic.message
    })
}

fn write_response(output: &mut impl Write, id: Value, code: i64, message: &str) -> io::Result<()> {
    write_message(
        output,
        &json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": code, "message": message}
        }),
    )
}

fn write_message(output: &mut impl Write, message: &Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing LSP message exceeds size limit",
        ));
    }
    write!(output, "Content-Length: {}\r\n\r\n", body.len())?;
    output.write_all(&body)?;
    output.flush()
}

fn read_frame(input: &mut impl BufRead) -> io::Result<Option<Vec<u8>>> {
    let mut header_bytes = 0;
    let mut content_length = None;
    loop {
        let Some(line) = read_header_line(input, MAX_HEADER_BYTES - header_bytes)? else {
            if header_bytes == 0 {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "EOF inside LSP headers",
            ));
        };
        header_bytes += line.len();
        if line == b"\r\n" || line == b"\n" {
            break;
        }
        let line = std::str::from_utf8(&line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some((name, value)) = line.trim().split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                if content_length.is_some() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "duplicate Content-Length header",
                    ));
                }
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
                );
            }
        }
    }

    let content_length = content_length.ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length header")
    })?;
    if content_length > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "incoming LSP message exceeds size limit",
        ));
    }
    let mut body = vec![0; content_length];
    input.read_exact(&mut body)?;
    Ok(Some(body))
}

fn read_header_line(input: &mut impl BufRead, byte_limit: usize) -> io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let available = input.fill_buf()?;
        if available.is_empty() {
            return Ok((!line.is_empty()).then_some(line));
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let visible_bytes = newline.map_or(available.len(), |index| index + 1);
        let remaining = byte_limit.saturating_sub(line.len());
        if visible_bytes > remaining {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LSP headers exceed size limit",
            ));
        }
        line.extend_from_slice(&available[..visible_bytes]);
        input.consume(visible_bytes);

        if newline.is_some() {
            return Ok(Some(line));
        }
        if line.len() == byte_limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LSP headers exceed size limit",
            ));
        }
    }
}
