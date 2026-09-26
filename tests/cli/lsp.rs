use std::{
    io::{self, BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

const FRAME_LIMIT: usize = 8 * 1024 * 1024;
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(3);

struct LspProcess {
    child: Child,
    stdin: Option<ChildStdin>,
    messages: Receiver<io::Result<Value>>,
}

impl LspProcess {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start ckc lsp");
        let stdout = child.stdout.take().expect("capture server stdout");
        let (sender, messages) = mpsc::channel();
        thread::spawn(move || {
            let mut stdout = BufReader::new(stdout);
            loop {
                match read_message(&mut stdout) {
                    Ok(Some(message)) => {
                        if sender.send(Ok(message)).is_err() {
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(error) => {
                        let _ = sender.send(Err(error));
                        break;
                    }
                }
            }
        });

        Self {
            stdin: child.stdin.take(),
            child,
            messages,
        }
    }

    fn initialize(&mut self) -> Value {
        self.send_initialize();
        let response = self.receive_matching(|message| message["id"] == 1);
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["error"], Value::Null, "{response}");
        assert_eq!(response["result"]["serverInfo"]["name"], "ckc");
        assert_eq!(
            response["result"]["serverInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(
            response["result"]["capabilities"]["textDocumentSync"]["change"],
            1
        );
        assert_eq!(
            response["result"]["capabilities"]["textDocumentSync"]["openClose"],
            true
        );
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "initialized",
            "params": {}
        }));
        response
    }

    fn send_initialize(&mut self) {
        self.send(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": null,
                "rootUri": null,
                "capabilities": {}
            }
        }));
    }

    fn send(&mut self, message: Value) {
        let body = serde_json::to_vec(&message).expect("serialize LSP request");
        self.send_body(&body);
    }

    fn send_invalid_json(&mut self, body: &[u8]) {
        self.send_body(body);
    }

    fn send_raw_input(&mut self, bytes: &[u8]) {
        self.stdin
            .as_mut()
            .expect("server stdin remains open")
            .write_all(bytes)
            .expect("write raw LSP input");
        self.stdin
            .as_mut()
            .expect("server stdin remains open")
            .flush()
            .expect("flush raw LSP input");
    }

    fn send_body(&mut self, body: &[u8]) {
        let stdin = self.stdin.as_mut().expect("server stdin remains open");
        write!(stdin, "Content-Length: {}\r\n\r\n", body.len()).expect("write LSP header");
        stdin.write_all(body).expect("write LSP body");
        stdin.flush().expect("flush LSP message");
    }

    fn receive_matching(&self, matches: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            let timeout = deadline.saturating_duration_since(Instant::now());
            assert!(!timeout.is_zero(), "timed out waiting for LSP message");
            let message = match self.messages.recv_timeout(timeout) {
                Ok(Ok(message)) => message,
                Ok(Err(error)) => panic!("stdout was not a valid LSP frame: {error}"),
                Err(RecvTimeoutError::Timeout) => panic!("timed out waiting for LSP message"),
                Err(RecvTimeoutError::Disconnected) => {
                    panic!("ckc lsp closed stdout before sending the expected message")
                }
            };
            if matches(&message) {
                return message;
            }
        }
    }

    fn receive_message(&self) -> Value {
        self.receive_matching(|_| true)
    }

    fn send_shutdown_and_exit(&mut self) -> ExitStatus {
        self.send(json!({"jsonrpc": "2.0", "id": 99, "method": "shutdown"}));
        let response = self.receive_matching(|message| message["id"] == 99);
        assert_eq!(response["result"], Value::Null, "{response}");
        self.send(json!({"jsonrpc": "2.0", "method": "exit"}));
        self.close_stdin();
        self.wait_for_exit()
    }

    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    fn wait_for_exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + RESPONSE_TIMEOUT;
        loop {
            if let Some(status) = self.child.try_wait().expect("wait for ckc lsp") {
                return status;
            }
            assert!(Instant::now() < deadline, "ckc lsp did not exit after EOF");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_stdout_closed(&self) {
        match self.messages.recv_timeout(RESPONSE_TIMEOUT) {
            Err(RecvTimeoutError::Disconnected) => {}
            Ok(Err(error)) => panic!("stdout contains non-LSP output: {error}"),
            Ok(Ok(message)) => panic!("unexpected extra stdout frame: {message}"),
            Err(RecvTimeoutError::Timeout) => panic!("stdout stayed open after process exit"),
        }
    }
}

impl Drop for LspProcess {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    let mut content_length = None;
    let mut header_bytes = 0;
    loop {
        let mut line = Vec::new();
        let count = reader.read_until(b'\n', &mut line)?;
        if count == 0 {
            if header_bytes == 0 {
                return Ok(None);
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "EOF inside LSP headers",
            ));
        }
        header_bytes += count;
        if header_bytes > 8 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "LSP headers exceed test limit",
            ));
        }
        if line == b"\r\n" || line == b"\n" {
            break;
        }
        let line = std::str::from_utf8(&line)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if let Some((name, value)) = line.trim().split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
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
        io::Error::new(
            io::ErrorKind::InvalidData,
            "LSP frame has no Content-Length",
        )
    })?;
    if content_length > FRAME_LIMIT {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "LSP frame exceeds test limit",
        ));
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    serde_json::from_slice(&body)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn uri() -> String {
    static NEXT_URI: AtomicUsize = AtomicUsize::new(0);
    let id = NEXT_URI.fetch_add(1, Ordering::Relaxed);
    format!("file:///ck-lsp-test-{id}.ck")
}

fn did_open(process: &mut LspProcess, uri: &str, version: i64, text: &str) {
    process.send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {
            "textDocument": {
                "uri": uri,
                "languageId": "ck",
                "version": version,
                "text": text
            }
        }
    }));
}

fn did_change(process: &mut LspProcess, uri: &str, version: i64, text: &str) {
    process.send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {"uri": uri, "version": version},
            "contentChanges": [{"text": text}]
        }
    }));
}

fn diagnostics_for(process: &LspProcess, uri: &str) -> Value {
    let message = process.receive_matching(|message| {
        message["method"] == "textDocument/publishDiagnostics" && message["params"]["uri"] == uri
    });
    message["params"].clone()
}

#[test]
fn lsp_should_publish_unsaved_ck_diagnostics_clear_them_after_change_and_shutdown() {
    let mut process = LspProcess::start();
    process.initialize();

    let uri = uri();
    let invalid_source = "fn main() -> i32 { return @; }";
    did_open(&mut process, &uri, 1, invalid_source);
    let first = diagnostics_for(&process, &uri);
    let diagnostic = first["diagnostics"]
        .as_array()
        .and_then(|diagnostics| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic["code"] == "CK0001")
        })
        .expect("invalid unsaved source should publish a CK0001 diagnostic");
    let at = invalid_source.find('@').expect("invalid character") as u32;
    assert_eq!(diagnostic["range"]["start"]["line"], 0);
    assert_eq!(diagnostic["range"]["start"]["character"], at);
    assert_eq!(diagnostic["range"]["end"]["line"], 0);
    assert_eq!(diagnostic["range"]["end"]["character"], at + 1);

    let valid_source = "fn main() -> i32 { return 1; }";
    did_change(&mut process, &uri, 2, valid_source);
    let cleared = diagnostics_for(&process, &uri);
    assert_eq!(cleared["version"], 2);
    assert_eq!(cleared["diagnostics"], json!([]));

    assert!(process.send_shutdown_and_exit().success());
    process.assert_stdout_closed();
}

#[test]
fn lsp_should_report_ranges_in_utf16_for_non_bmp_characters() {
    let mut process = LspProcess::start();
    process.initialize();

    let uri = uri();
    let source = "😀@";
    did_open(&mut process, &uri, 1, source);
    let diagnostics = diagnostics_for(&process, &uri);
    let diagnostic = diagnostics["diagnostics"]
        .as_array()
        .and_then(|diagnostics| {
            diagnostics
                .iter()
                .find(|diagnostic| diagnostic["message"] == "Unexpected character '@'.")
        })
        .expect("unexpected @ diagnostic");
    assert_eq!(
        diagnostic["range"]["start"],
        json!({"line": 0, "character": 2})
    );
    assert_eq!(
        diagnostic["range"]["end"],
        json!({"line": 0, "character": 3})
    );

    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_ignore_stale_document_versions() {
    let mut process = LspProcess::start();
    process.initialize();

    let uri = uri();
    did_open(&mut process, &uri, 4, "@");
    assert_eq!(diagnostics_for(&process, &uri)["version"], 4);
    did_change(&mut process, &uri, 6, "fn main() -> i32 { return 1; }");
    did_change(&mut process, &uri, 5, "@");

    let diagnostics = diagnostics_for(&process, &uri);
    assert_eq!(diagnostics["version"], 6);
    assert_eq!(diagnostics["diagnostics"], json!([]));
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_clear_closed_document_diagnostics_and_allow_reopen_with_new_version_epoch() {
    let mut process = LspProcess::start();
    process.initialize();

    let uri = uri();
    did_open(&mut process, &uri, 7, "@");
    assert!(
        !diagnostics_for(&process, &uri)["diagnostics"]
            .as_array()
            .expect("diagnostics array")
            .is_empty()
    );
    process.send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didClose",
        "params": {"textDocument": {"uri": uri}}
    }));
    assert_eq!(diagnostics_for(&process, &uri)["diagnostics"], json!([]));

    did_open(&mut process, &uri, 1, "@");
    let reopened = diagnostics_for(&process, &uri);
    assert_eq!(reopened["version"], 1);
    assert!(
        !reopened["diagnostics"]
            .as_array()
            .expect("diagnostics array")
            .is_empty()
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_return_json_rpc_errors_for_invalid_json_and_unknown_requests() {
    let mut process = LspProcess::start();
    process.initialize();

    process.send_invalid_json(b"{");
    let parse_error = process.receive_matching(|message| message["error"]["code"] == -32700);
    assert_eq!(parse_error["id"], Value::Null);

    process.send(json!({"jsonrpc": "2.0", "id": 9}));
    let invalid_request = process.receive_matching(|message| message["id"] == 9);
    assert_eq!(invalid_request["error"]["code"], -32600);

    process.send(json!({"jsonrpc": "2.0", "method": "not/known"}));
    process.send(json!({"jsonrpc": "2.0", "id": 8, "method": "not/known"}));
    let method_error = process.receive_matching(|message| message["id"] == 8);
    assert_eq!(method_error["error"]["code"], -32601);
    assert!(
        method_error["error"]["message"]
            .as_str()
            .expect("error message")
            .contains("not/known")
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_exit_cleanly_when_input_reaches_eof_before_initialization() {
    let mut process = LspProcess::start();
    process.close_stdin();
    let status = process.wait_for_exit();
    assert!(
        status.success(),
        "EOF should stop the server cleanly: {status}"
    );
    process.assert_stdout_closed();
}

#[test]
fn lsp_should_reject_oversized_header_lines_without_waiting_for_a_newline() {
    let mut process = LspProcess::start();
    process.send_raw_input(b"Content-Length: 0\r\n");
    process.send_raw_input(&vec![b'X'; 8 * 1024]);

    let status = process.wait_for_exit();
    assert!(!status.success(), "oversized LSP header should be rejected");
    process.assert_stdout_closed();
}

#[test]
fn lsp_should_skip_lexical_error_floods_and_continue_serving_documents() {
    let mut process = LspProcess::start();
    process.initialize();

    let large_uri = uri();
    did_open(&mut process, &large_uri, 1, &"@".repeat(60_000));
    let warning = process.receive_matching(|message| message["method"] == "window/logMessage");
    assert_eq!(warning["params"]["type"], 2);
    let message = warning["params"]["message"]
        .as_str()
        .expect("analysis limit warning message");
    assert!(message.contains("lexical errors"), "{message}");

    let diagnostics = diagnostics_for(&process, &large_uri);
    assert_eq!(diagnostics["diagnostics"], json!([]));

    let small_uri = uri();
    did_open(
        &mut process,
        &small_uri,
        1,
        "fn main() -> i32 { return 1; }",
    );
    assert_eq!(
        diagnostics_for(&process, &small_uri)["diagnostics"],
        json!([])
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_skip_unsafe_syntax_nesting_and_continue_serving_documents() {
    let mut process = LspProcess::start();
    process.initialize();

    let deep_uri = uri();
    let deeply_nested = format!(
        "fn main() -> i32 {{ return {}1{}; }}",
        "(".repeat(4_000),
        ")".repeat(4_000)
    );
    did_open(&mut process, &deep_uri, 1, &deeply_nested);

    let warning = process.receive_matching(|message| message["method"] == "window/logMessage");
    assert_eq!(warning["params"]["type"], 2);
    let warning = warning["params"]["message"]
        .as_str()
        .expect("syntax nesting warning");
    assert!(warning.contains("nesting"), "{warning}");
    assert!(warning.contains("256"), "{warning}");
    assert_eq!(
        diagnostics_for(&process, &deep_uri)["diagnostics"],
        json!([])
    );

    let small_uri = uri();
    did_open(
        &mut process,
        &small_uri,
        1,
        "fn main() -> i32 { return 1; }",
    );
    assert_eq!(
        diagnostics_for(&process, &small_uri)["diagnostics"],
        json!([])
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_bound_lexical_diagnostics_before_analysis_and_continue_serving() {
    let mut process = LspProcess::start();
    process.initialize();

    let noisy_uri = format!("file:///{}.ck", "n".repeat(4_000));
    did_open(&mut process, &noisy_uri, 1, &"@".repeat(5_000));
    let warning = process.receive_matching(|message| message["method"] == "window/logMessage");
    assert_eq!(warning["params"]["type"], 2);
    let warning = warning["params"]["message"]
        .as_str()
        .expect("lexical complexity warning");
    assert!(warning.contains("lexical"), "{warning}");
    assert!(warning.contains("256"), "{warning}");
    assert_eq!(
        diagnostics_for(&process, &noisy_uri)["diagnostics"],
        json!([])
    );

    let small_uri = uri();
    did_open(
        &mut process,
        &small_uri,
        1,
        "fn main() -> i32 { return 1; }",
    );
    assert_eq!(
        diagnostics_for(&process, &small_uri)["diagnostics"],
        json!([])
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_warn_and_skip_documents_with_oversized_uris() {
    let mut process = LspProcess::start();
    process.initialize();

    let long_uri = format!("file:///{}.ck", "u".repeat(4_100));
    did_open(&mut process, &long_uri, 1, "@");
    let warning = process.receive_matching(|message| message["method"] == "window/logMessage");
    assert_eq!(warning["params"]["type"], 2);
    let warning = warning["params"]["message"]
        .as_str()
        .expect("URI size warning");
    assert!(warning.contains("URI"), "{warning}");
    assert!(warning.contains("4 KiB"), "{warning}");

    let small_uri = uri();
    did_open(
        &mut process,
        &small_uri,
        1,
        "fn main() -> i32 { return 1; }",
    );
    assert_eq!(
        diagnostics_for(&process, &small_uri)["diagnostics"],
        json!([])
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_reject_preinitialization_requests_and_ignore_document_notifications() {
    let mut process = LspProcess::start();
    process.send(json!({
        "jsonrpc": "2.0",
        "id": 7,
        "method": "textDocument/hover",
        "params": {"textDocument": {"uri": "file:///early.ck"}, "position": {"line": 0, "character": 0}}
    }));
    let not_initialized = process.receive_message();
    assert_eq!(not_initialized["id"], 7);
    assert_eq!(not_initialized["error"]["code"], -32002);

    let early_uri = uri();
    did_open(&mut process, &early_uri, 1, "@");
    process.send_initialize();
    let initialize_response = process.receive_message();
    assert_eq!(initialize_response["id"], 1, "{initialize_response}");
    assert_eq!(initialize_response["error"], Value::Null);

    did_open(&mut process, &early_uri, 2, "@");
    process.send(json!({
        "jsonrpc": "2.0",
        "method": "initialized",
        "params": {}
    }));

    let active_uri = uri();
    did_open(&mut process, &active_uri, 1, "@");
    assert!(
        !diagnostics_for(&process, &active_uri)["diagnostics"]
            .as_array()
            .expect("diagnostics array")
            .is_empty()
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_skip_deep_binary_expression_trees_and_continue_serving() {
    let mut process = LspProcess::start();
    process.initialize();

    let deep_uri = uri();
    let expression = vec!["1"; 4_000].join(" + ");
    let source = format!("fn main() -> i32 {{ return {expression}; }}");
    did_open(&mut process, &deep_uri, 1, &source);
    let warning = process.receive_matching(|message| message["method"] == "window/logMessage");
    let warning = warning["params"]["message"]
        .as_str()
        .expect("expression complexity warning");
    assert!(warning.contains("operator"), "{warning}");
    assert_eq!(
        diagnostics_for(&process, &deep_uri)["diagnostics"],
        json!([])
    );

    let small_uri = uri();
    did_open(
        &mut process,
        &small_uri,
        1,
        "fn main() -> i32 { return 1; }",
    );
    assert_eq!(
        diagnostics_for(&process, &small_uri)["diagnostics"],
        json!([])
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_skip_deep_postfix_expression_trees_and_continue_serving() {
    let mut process = LspProcess::start();
    process.initialize();

    let deep_uri = uri();
    let access_chain = "n.x".repeat(4_000);
    let source = format!("fn main(n: i32) -> i32 {{ return {access_chain}; }}");
    did_open(&mut process, &deep_uri, 1, &source);
    let warning = process.receive_matching(|message| message["method"] == "window/logMessage");
    let warning = warning["params"]["message"]
        .as_str()
        .expect("postfix complexity warning");
    assert!(warning.contains("postfix"), "{warning}");
    assert_eq!(
        diagnostics_for(&process, &deep_uri)["diagnostics"],
        json!([])
    );

    let small_uri = uri();
    did_open(
        &mut process,
        &small_uri,
        1,
        "fn main() -> i32 { return 1; }",
    );
    assert_eq!(
        diagnostics_for(&process, &small_uri)["diagnostics"],
        json!([])
    );
    assert!(process.send_shutdown_and_exit().success());
}

#[test]
fn lsp_should_skip_deep_unary_expression_chains_and_continue_serving() {
    let mut process = LspProcess::start();
    process.initialize();

    let deep_uri = uri();
    let source = format!("fn main() -> i32 {{ return {}1; }}", "-".repeat(4_000));
    did_open(&mut process, &deep_uri, 1, &source);
    let warning = process.receive_matching(|message| message["method"] == "window/logMessage");
    let warning = warning["params"]["message"]
        .as_str()
        .expect("unary complexity warning");
    assert!(warning.contains("unary operator chain"), "{warning}");
    assert_eq!(
        diagnostics_for(&process, &deep_uri)["diagnostics"],
        json!([])
    );

    let small_uri = uri();
    did_open(
        &mut process,
        &small_uri,
        1,
        "fn main() -> i32 { return 1; }",
    );
    assert_eq!(
        diagnostics_for(&process, &small_uri)["diagnostics"],
        json!([])
    );
    assert!(process.send_shutdown_and_exit().success());
}
