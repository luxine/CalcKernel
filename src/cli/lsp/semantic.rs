#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::{SEMANTIC_TOKEN_TYPES, handle};

    const URI: &str = "file:///tmp/editor.ck";

    fn request(method: &str, params: Value, text: &str) -> Value {
        handle(method, &params, URI, text)
            .expect("supported provider method")
            .expect("successful request")
    }

    fn location_of(text: &str, needle: &str, last: bool) -> (usize, usize) {
        let byte = if last {
            text.rfind(needle).expect("needle exists")
        } else {
            text.find(needle).expect("needle exists")
        };
        let prefix = &text[..byte];
        let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
        let column = prefix
            .rsplit('\n')
            .next()
            .expect("last line")
            .encode_utf16()
            .count();
        (line, column)
    }

    fn position_params(text: &str, needle: &str, last: bool) -> Value {
        let (line, character) = location_of(text, needle, last);
        json!({
            "textDocument": {"uri": URI},
            "position": {"line": line, "character": character}
        })
    }

    #[test]
    fn definition_and_references_use_the_global_call_binding() {
        let text = "fn target() -> i32 { return 1; }\nfn caller() -> i32 { let target: i32 = 0; return target(); }";
        let params = position_params(text, "target()", true);
        let definition = request("textDocument/definition", params.clone(), text);
        assert_eq!(definition["uri"], URI);
        assert_eq!(definition["range"]["start"]["line"], 0);

        let mut params = params;
        params["context"] = json!({"includeDeclaration": true});
        let references = request("textDocument/references", params, text);
        assert_eq!(references.as_array().expect("locations").len(), 2);
        assert_eq!(references[0]["range"]["start"]["line"], 0);
        assert_eq!(references[1]["range"]["start"]["line"], 1);
    }

    #[test]
    fn rename_emits_utf16_ranges_for_every_bound_occurrence() {
        let text = "// 😀\nfn f(value: i32) -> i32 { return value; }";
        let params = position_params(text, "value", false);
        let result = request(
            "textDocument/rename",
            json!({
                "textDocument": {"uri": URI},
                "position": params["position"],
                "newName": "amount"
            }),
            text,
        );
        let edits = result["changes"][URI].as_array().expect("text edits");
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0]["range"]["start"]["line"], 1);
        assert_eq!(edits[0]["range"]["start"]["character"], 5);
        assert_eq!(edits[0]["newText"], "amount");
    }

    #[test]
    fn document_and_workspace_symbols_include_struct_members_and_filter_queries() {
        let text =
            "struct Item { price: i32; }\nfn calculate(item: Item) -> i32 { return item.price; }";
        let document = request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": URI}}),
            text,
        );
        assert_eq!(document.as_array().expect("document symbols").len(), 2);
        assert_eq!(document[0]["name"], "Item");
        assert_eq!(document[0]["children"][0]["name"], "price");
        assert_eq!(document[1]["name"], "calculate");
        assert_eq!(document[1]["children"][0]["name"], "item");

        let workspace = request("workspace/symbol", json!({"query": "calc"}), text);
        assert_eq!(workspace.as_array().expect("workspace symbols").len(), 1);
        assert_eq!(workspace[0]["name"], "calculate");
    }

    #[test]
    fn semantic_tokens_use_delta_encoded_utf16_lengths_and_the_advertised_legend() {
        let text = "// 😀\nfn f(value: i32) -> i32 { return value; }";
        let result = request(
            "textDocument/semanticTokens/full",
            json!({"textDocument": {"uri": URI}}),
            text,
        );
        let data = result["data"].as_array().expect("token data");
        assert_eq!(data.len() % 5, 0);
        assert!(!SEMANTIC_TOKEN_TYPES.is_empty());
        assert_eq!(data[0], 1);
        assert_eq!(data[1], 3);
        assert_eq!(data[2], 1);
        assert_eq!(data[3], 3);
        assert_eq!(data[4], 1);
    }

    #[test]
    fn prepare_rename_returns_null_for_builtin_projection_and_main() {
        let text =
            "fn main() -> i32 { return 0; }\nfn f(items: slice<i32>) -> u32 { return items.len; }";
        let params = position_params(text, "main", false);
        assert_eq!(
            request("textDocument/prepareRename", params, text),
            Value::Null
        );

        let params = position_params(text, "len", false);
        assert_eq!(
            request("textDocument/prepareRename", params, text),
            Value::Null
        );
    }
}
use serde_json::{Value, json};

use calckernel::{
    EditorAnalysis, EditorOccurrence, EditorOccurrenceKind, EditorRenameError, EditorScope,
    EditorSymbol, ScopeId, SourceFile, SourceSpan, SymbolId, SymbolKind, analyze_editor,
    get_compiler_builtin,
};

pub(super) const SEMANTIC_TOKEN_TYPES: &[&str] = &[
    "namespace",
    "type",
    "struct",
    "function",
    "parameter",
    "variable",
    "property",
];

pub(super) const SEMANTIC_TOKEN_MODIFIERS: &[&str] = &[
    "declaration",
    "definition",
    "readonly",
    "static",
    "deprecated",
    "abstract",
    "async",
    "modification",
    "documentation",
    "defaultLibrary",
];

const TOKEN_TYPE_STRUCT: u32 = 2;
const TOKEN_TYPE_FUNCTION: u32 = 3;
const TOKEN_TYPE_PARAMETER: u32 = 4;
const TOKEN_TYPE_VARIABLE: u32 = 5;
const TOKEN_TYPE_PROPERTY: u32 = 6;
const TOKEN_MODIFIER_DECLARATION: u32 = 1 << 0;
const TOKEN_MODIFIER_DEFAULT_LIBRARY: u32 = 1 << 9;

type ProviderResult = Result<Value, (i64, String)>;

/// Handles semantic LSP methods for one immutable document snapshot.
/// The transport owns URI selection, document versions, and lifecycle.
pub(super) fn handle(
    method: &str,
    params: &Value,
    uri: &str,
    text: &str,
) -> Option<ProviderResult> {
    let method_result = match method {
        "textDocument/definition" => definition(params, uri, text),
        "textDocument/references" => references(params, uri, text),
        "textDocument/prepareRename" => prepare_rename(params, text),
        "textDocument/rename" => rename(params, uri, text),
        "textDocument/documentSymbol" => Ok(Value::Array(document_symbols(
            &analyze(text),
            &LineIndex::new(text),
        ))),
        "workspace/symbol" => workspace_symbols(params, uri, text),
        "textDocument/semanticTokens/full" => {
            Ok(json!({"data": semantic_token_data(&analyze(text), text)}))
        }
        _ => return None,
    };
    Some(method_result)
}

fn analyze(text: &str) -> EditorAnalysis {
    analyze_editor(&SourceFile::new("editor.ck", text))
}

fn definition(params: &Value, uri: &str, text: &str) -> ProviderResult {
    let offset = position_offset(params, text)?;
    let analysis = analyze(text);
    let Some(symbol) = analysis.definition_at(offset) else {
        return Ok(Value::Null);
    };
    location(uri, symbol.declaration, &LineIndex::new(text)).ok_or_else(invalid_position)
}

fn references(params: &Value, uri: &str, text: &str) -> ProviderResult {
    let offset = position_offset(params, text)?;
    let include_declaration = params
        .get("context")
        .and_then(|context| context.get("includeDeclaration"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let analysis = analyze(text);
    let line_index = LineIndex::new(text);
    let locations = analysis
        .references_at(offset)
        .into_iter()
        .filter(|occurrence| include_declaration || !occurrence.is_declaration)
        .filter_map(|occurrence| location(uri, occurrence.span, &line_index))
        .collect();
    Ok(Value::Array(locations))
}

fn prepare_rename(params: &Value, text: &str) -> ProviderResult {
    let offset = position_offset(params, text)?;
    let analysis = analyze(text);
    if !analysis.is_complete {
        return Ok(Value::Null);
    }
    let Some(occurrence) = occurrence_at(&analysis, offset) else {
        return Ok(Value::Null);
    };
    let Some(symbol_id) = occurrence.symbol_id else {
        return Ok(Value::Null);
    };
    let Some(symbol) = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.id == symbol_id)
    else {
        return Ok(Value::Null);
    };
    if !symbol.renameable || (symbol.kind == SymbolKind::Function && symbol.name == "main") {
        return Ok(Value::Null);
    }
    let range = span_range(occurrence.span, &LineIndex::new(text)).ok_or_else(invalid_position)?;
    Ok(json!({"range": range, "placeholder": occurrence.name}))
}

fn rename(params: &Value, uri: &str, text: &str) -> ProviderResult {
    let offset = position_offset(params, text)?;
    let new_name = params
        .get("newName")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid_params("rename requires a string newName"))?;
    let analysis = analyze(text);
    let Some(occurrence) = occurrence_at(&analysis, offset) else {
        return Ok(Value::Null);
    };
    if occurrence.symbol_id.is_none() {
        return Ok(Value::Null);
    }
    let edits = analysis.rename(offset, new_name).map_err(rename_error)?;
    let line_index = LineIndex::new(text);
    let text_edits: Option<Vec<_>> = edits
        .iter()
        .map(|edit| {
            Some(json!({
                "range": span_range(edit.span, &line_index)?,
                "newText": edit.new_text
            }))
        })
        .collect();
    let text_edits = text_edits.ok_or_else(invalid_position)?;
    Ok(json!({"changes": {uri: text_edits}}))
}

fn document_symbols(analysis: &EditorAnalysis, line_index: &LineIndex) -> Vec<Value> {
    let mut declarations: Vec<_> = analysis
        .symbols
        .iter()
        .filter(|symbol| matches!(symbol.kind, SymbolKind::Function | SymbolKind::Struct))
        .collect();
    declarations.sort_by_key(|symbol| symbol.declaration.start.offset);
    declarations
        .into_iter()
        .filter_map(|symbol| document_symbol(symbol, analysis, line_index))
        .collect()
}

fn document_symbol(
    symbol: &EditorSymbol,
    analysis: &EditorAnalysis,
    line_index: &LineIndex,
) -> Option<Value> {
    let scope = match symbol.kind {
        SymbolKind::Struct => struct_scope_for_symbol(symbol, analysis),
        SymbolKind::Function => function_scope_for_symbol(symbol, analysis),
        _ => None,
    }?;
    let range = offsets_range(scope.span.start.offset, scope.span.end.offset, line_index)?;
    let selection_range = span_range(symbol.declaration, line_index)?;
    let mut result = json!({
        "name": symbol.name,
        "kind": lsp_symbol_kind(symbol.kind),
        "range": range,
        "selectionRange": selection_range
    });

    let mut children: Vec<(&EditorSymbol, Option<&'static str>)> = match symbol.kind {
        SymbolKind::Struct => analysis
            .symbols
            .iter()
            .filter(|child| child.kind == SymbolKind::Field && child.scope_id == scope.id)
            .map(|child| (child, None))
            .collect(),
        SymbolKind::Function => analysis
            .symbols
            .iter()
            .filter(|child| {
                matches!(child.kind, SymbolKind::Parameter | SymbolKind::Local)
                    && function_root_scope(child.scope_id, analysis)
                        .is_some_and(|root| root.id == scope.id)
            })
            .map(|child| {
                let detail = match child.kind {
                    SymbolKind::Parameter => "parameter",
                    SymbolKind::Local => "local",
                    _ => "",
                };
                (child, Some(detail))
            })
            .collect(),
        _ => Vec::new(),
    };
    children.sort_by_key(|(child, _)| child.declaration.start.offset);
    if !children.is_empty() {
        let serialized: Vec<_> = children
            .into_iter()
            .filter_map(|(child, detail)| {
                Some(json!({
                    "name": child.name,
                    "kind": lsp_symbol_kind(child.kind),
                    "range": span_range(child.declaration, line_index)?,
                    "selectionRange": span_range(child.declaration, line_index)?,
                    "detail": detail
                }))
            })
            .collect();
        result["children"] = json!(serialized);
    }
    Some(result)
}

fn struct_scope_for_symbol<'a>(
    symbol: &EditorSymbol,
    analysis: &'a EditorAnalysis,
) -> Option<&'a EditorScope> {
    analysis.scopes.iter().find(|scope| {
        scope.parent == Some(ScopeId(0))
            && scope.span.start.offset <= symbol.declaration.start.offset
            && symbol.declaration.end.offset <= scope.span.end.offset
            && analysis.symbols.iter().any(|candidate| {
                candidate.kind == SymbolKind::Struct
                    && candidate.id == symbol.id
                    && scope.span.start.offset <= candidate.declaration.start.offset
                    && candidate.declaration.end.offset <= scope.span.end.offset
            })
    })
}

fn function_scope_for_symbol<'a>(
    symbol: &EditorSymbol,
    analysis: &'a EditorAnalysis,
) -> Option<&'a EditorScope> {
    analysis
        .scopes
        .iter()
        .filter(|scope| {
            scope.parent == Some(ScopeId(0))
                && scope.span.start.offset <= symbol.declaration.start.offset
                && symbol.declaration.end.offset <= scope.span.end.offset
        })
        .filter(|scope| {
            function_for_scope(scope, analysis).is_some_and(|function| function.id == symbol.id)
        })
        .min_by_key(|scope| {
            scope
                .span
                .end
                .offset
                .saturating_sub(scope.span.start.offset)
        })
}

fn function_for_scope<'a>(
    scope: &EditorScope,
    analysis: &'a EditorAnalysis,
) -> Option<&'a EditorSymbol> {
    analysis
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.kind == SymbolKind::Function
                && scope.span.start.offset <= symbol.declaration.start.offset
                && symbol.declaration.end.offset <= scope.span.end.offset
        })
        .min_by_key(|symbol| symbol.declaration.start.offset)
}

fn function_root_scope(scope_id: ScopeId, analysis: &EditorAnalysis) -> Option<&EditorScope> {
    let mut scope = analysis.scopes.iter().find(|scope| scope.id == scope_id)?;
    loop {
        match scope.parent {
            Some(ScopeId(0)) => return Some(scope),
            Some(parent) => scope = analysis.scopes.iter().find(|scope| scope.id == parent)?,
            None => return None,
        }
    }
}

fn workspace_symbols(params: &Value, uri: &str, text: &str) -> ProviderResult {
    let query = params.get("query").and_then(Value::as_str).unwrap_or("");
    let query = query.to_lowercase();
    let analysis = analyze(text);
    let line_index = LineIndex::new(text);
    let symbols = analysis
        .symbols
        .iter()
        .filter(|symbol| matches!(symbol.kind, SymbolKind::Function | SymbolKind::Struct))
        .filter(|symbol| symbol.name.to_lowercase().contains(&query))
        .filter_map(|symbol| {
            Some(json!({
                "name": symbol.name,
                "kind": lsp_symbol_kind(symbol.kind),
                "location": location(uri, symbol.declaration, &line_index)?
            }))
        })
        .collect();
    Ok(Value::Array(symbols))
}

fn semantic_token_data(analysis: &EditorAnalysis, text: &str) -> Vec<u32> {
    let line_index = LineIndex::new(text);
    let symbols: std::collections::HashMap<SymbolId, &EditorSymbol> = analysis
        .symbols
        .iter()
        .map(|symbol| (symbol.id, symbol))
        .collect();
    let mut tokens = Vec::new();
    for occurrence in &analysis.occurrences {
        let (token_type, modifiers) = if occurrence.kind == EditorOccurrenceKind::Builtin {
            let token_type = if get_compiler_builtin(&occurrence.name).is_some()
                || is_contract_builtin(&occurrence.name)
            {
                TOKEN_TYPE_FUNCTION
            } else {
                TOKEN_TYPE_PROPERTY
            };
            (token_type, TOKEN_MODIFIER_DEFAULT_LIBRARY)
        } else {
            let Some(symbol) = occurrence
                .symbol_id
                .and_then(|symbol_id| symbols.get(&symbol_id).copied())
            else {
                continue;
            };
            let token_type = match symbol.kind {
                SymbolKind::Function => TOKEN_TYPE_FUNCTION,
                SymbolKind::Struct => TOKEN_TYPE_STRUCT,
                SymbolKind::Field => TOKEN_TYPE_PROPERTY,
                SymbolKind::Parameter => TOKEN_TYPE_PARAMETER,
                SymbolKind::Local => TOKEN_TYPE_VARIABLE,
            };
            let modifiers = if occurrence.is_declaration {
                TOKEN_MODIFIER_DECLARATION
            } else {
                0
            };
            (token_type, modifiers)
        };
        let Some(start) = line_index.position(occurrence.span.start.offset) else {
            continue;
        };
        let Some(end) = line_index.position(occurrence.span.end.offset) else {
            continue;
        };
        if start.line != end.line || end.character <= start.character {
            continue;
        }
        tokens.push((
            occurrence.span.start.offset,
            start.line,
            start.character,
            end.character - start.character,
            token_type,
            modifiers,
        ));
    }
    tokens.sort_by_key(|token| token.0);
    let mut data = Vec::with_capacity(tokens.len() * 5);
    let (mut previous_line, mut previous_character) = (0, 0);
    for (_, line, character, length, token_type, modifiers) in tokens {
        let delta_line = line.saturating_sub(previous_line);
        let delta_character = if delta_line == 0 {
            character.saturating_sub(previous_character)
        } else {
            character
        };
        let Ok(delta_line) = u32::try_from(delta_line) else {
            continue;
        };
        let Ok(delta_character) = u32::try_from(delta_character) else {
            continue;
        };
        let Ok(length) = u32::try_from(length) else {
            continue;
        };
        data.extend([delta_line, delta_character, length, token_type, modifiers]);
        previous_line = line;
        previous_character = character;
    }
    data
}

fn occurrence_at(analysis: &EditorAnalysis, offset: usize) -> Option<&EditorOccurrence> {
    analysis.occurrences.iter().find(|occurrence| {
        occurrence.span.start.offset <= offset && offset < occurrence.span.end.offset
    })
}

fn position_offset(params: &Value, text: &str) -> Result<usize, (i64, String)> {
    let position = params
        .get("position")
        .ok_or_else(|| invalid_params("request requires a position"))?;
    let line = position
        .get("line")
        .and_then(Value::as_u64)
        .and_then(|line| usize::try_from(line).ok())
        .ok_or_else(|| invalid_params("position.line must be a non-negative integer"))?;
    let character = position
        .get("character")
        .and_then(Value::as_u64)
        .and_then(|character| usize::try_from(character).ok())
        .ok_or_else(|| invalid_params("position.character must be a non-negative integer"))?;
    lsp_position_to_offset(text, line, character)
        .ok_or_else(|| invalid_params("position is outside the document"))
}

fn lsp_position_to_offset(text: &str, wanted_line: usize, character: usize) -> Option<usize> {
    let mut base: usize = 0;
    for (line, raw_line) in text.split('\n').enumerate() {
        let contents = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        if line == wanted_line {
            if character > contents.encode_utf16().count() {
                return None;
            }
            return base.checked_add(character);
        }
        base = base
            .checked_add(raw_line.encode_utf16().count())?
            .checked_add(1)?;
    }
    None
}

struct LineIndex {
    starts: Vec<usize>,
    text_len: usize,
}

impl LineIndex {
    fn new(text: &str) -> Self {
        let mut starts = vec![0];
        let mut offset = 0;
        for character in text.chars() {
            offset += character.len_utf16();
            if character == '\n' {
                starts.push(offset);
            }
        }
        Self {
            starts,
            text_len: offset,
        }
    }

    fn position(&self, offset: usize) -> Option<Position> {
        if offset > self.text_len {
            return None;
        }
        let line = self
            .starts
            .partition_point(|start| *start <= offset)
            .checked_sub(1)?;
        Some(Position {
            line,
            character: offset - self.starts[line],
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Position {
    line: usize,
    character: usize,
}

fn span_range(span: SourceSpan, line_index: &LineIndex) -> Option<Value> {
    offsets_range(span.start.offset, span.end.offset, line_index)
}

fn offsets_range(start: usize, end: usize, line_index: &LineIndex) -> Option<Value> {
    if end < start {
        return None;
    }
    Some(json!({
        "start": position_json(line_index.position(start)?),
        "end": position_json(line_index.position(end)?)
    }))
}

fn position_json(position: Position) -> Value {
    json!({"line": position.line, "character": position.character})
}

fn location(uri: &str, span: SourceSpan, line_index: &LineIndex) -> Option<Value> {
    Some(json!({"uri": uri, "range": span_range(span, line_index)?}))
}

fn lsp_symbol_kind(kind: SymbolKind) -> u32 {
    match kind {
        SymbolKind::Function => 12,
        SymbolKind::Struct => 23,
        SymbolKind::Field => 8,
        SymbolKind::Parameter | SymbolKind::Local => 13,
    }
}

fn rename_error(error: EditorRenameError) -> (i64, String) {
    invalid_params(error.message())
}

fn invalid_position() -> (i64, String) {
    invalid_params("source span is outside the document")
}

fn invalid_params(message: impl Into<String>) -> (i64, String) {
    (-32602, message.into())
}

fn is_contract_builtin(name: &str) -> bool {
    matches!(name, "aligned" | "multiple_of" | "noalias")
}
