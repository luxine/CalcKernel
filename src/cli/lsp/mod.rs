use std::{
    collections::HashMap,
    io::{self, BufRead, BufReader, BufWriter, Write},
};

use calckernel::{Diagnostic, SourceFile, check};
use serde_json::{Value, json};

const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_SOURCE_BYTES: usize = 4 * 1024 * 1024;
const MAX_HEADER_BYTES: usize = 8 * 1024;
const MAX_URI_BYTES: usize = 16 * 1024;

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

    if state.shutdown_requested && method != "exit" {
        if let Some(id) = id {
            write_response(output, id, -32600, "Server has shut down")?;
        }
        return Ok(None);
    }

    let params = object.get("params").unwrap_or(&Value::Null);
    match method {
        "initialize" => {
            if let Some(id) = id {
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
                )?;
            }
        }
        "initialized" => {}
        "shutdown" => {
            state.shutdown_requested = true;
            if let Some(id) = id {
                write_message(output, &json!({"jsonrpc": "2.0", "id": id, "result": null}))?;
            }
        }
        "exit" => return Ok(Some(i32::from(!state.shutdown_requested))),
        "textDocument/didOpen" => {
            if let Some((uri, version, text)) = opened_document(params) {
                analyze_and_publish(&uri, version, text, state, output)?;
            }
        }
        "textDocument/didChange" => {
            if let Some((uri, version, text)) = changed_document(params) {
                let is_newer = state
                    .documents
                    .get(&uri)
                    .is_some_and(|document| version > document.version);
                if is_newer {
                    analyze_and_publish(&uri, version, text, state, output)?;
                }
            }
        }
        "textDocument/didClose" => {
            if let Some(uri) = document_uri(params) {
                state.documents.remove(&uri);
                publish_diagnostics(output, &uri, None, &[])?;
            }
        }
        _ => {
            if let Some(id) = id {
                write_response(output, id, -32601, &format!("Method not found: {method}"))?;
            }
        }
    }

    Ok(None)
}

fn opened_document(params: &Value) -> Option<(String, i64, &str)> {
    let document = params.get("textDocument")?;
    Some((
        document_uri_value(document)?,
        document.get("version")?.as_i64()?,
        document.get("text")?.as_str()?,
    ))
}

fn changed_document(params: &Value) -> Option<(String, i64, &str)> {
    let document = params.get("textDocument")?;
    let change = params.get("contentChanges")?.as_array()?.last()?;
    Some((
        document_uri_value(document)?,
        document.get("version")?.as_i64()?,
        change.get("text")?.as_str()?,
    ))
}

fn document_uri(params: &Value) -> Option<String> {
    document_uri_value(params.get("textDocument")?)
}

fn document_uri_value(document: &Value) -> Option<String> {
    let uri = document.get("uri")?.as_str()?;
    (uri.len() <= MAX_URI_BYTES).then(|| uri.to_string())
}

fn analyze_and_publish(
    uri: &str,
    version: i64,
    text: &str,
    state: &mut ServerState,
    output: &mut impl Write,
) -> io::Result<()> {
    if text.len() > MAX_SOURCE_BYTES {
        state.documents.insert(
            uri.to_string(),
            DocumentSnapshot {
                version,
                _text: None,
            },
        );
        return publish_diagnostics(output, uri, Some(version), &[]);
    }

    let source = SourceFile::new(uri, text.to_string());
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
