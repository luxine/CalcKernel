use calckernel::{EditorOccurrenceKind, EditorRenameError, SourceFile, SymbolKind, analyze_editor};

fn analyze(text: &str) -> calckernel::EditorAnalysis {
    analyze_editor(&SourceFile::new("editor.ck", text))
}

fn occurrence<'a>(
    analysis: &'a calckernel::EditorAnalysis,
    text: &str,
    name: &str,
    declaration: bool,
) -> &'a calckernel::EditorOccurrence {
    let start = text.find(name).expect("name occurs in source");
    let offset = text[..start].encode_utf16().count();
    analysis
        .occurrences
        .iter()
        .find(|occurrence| occurrence.span.start.offset == offset && occurrence.name == name)
        .filter(|occurrence| occurrence.is_declaration == declaration)
        .expect("matching editor occurrence")
}

#[test]
fn call_binding_uses_global_function_even_when_a_local_has_the_same_name() {
    let text = "fn target() -> i32 { return 1; }\nfn caller() -> i32 { let target: i32 = 2; return target(); }";
    let analysis = analyze(text);
    let function_decl = occurrence(&analysis, text, "target", true);
    let call_offset = text.rfind("target()").expect("call");
    let call_offset = text[..call_offset].encode_utf16().count();
    let call = analysis
        .occurrences
        .iter()
        .find(|occurrence| occurrence.span.start.offset == call_offset)
        .expect("call callee occurrence");
    let local_start = text.find("let target").expect("local declaration") + "let ".len();
    let local_start = text[..local_start].encode_utf16().count();
    let local_decl = analysis
        .occurrences
        .iter()
        .find(|occurrence| occurrence.span.start.offset == local_start)
        .expect("local declaration");

    assert_eq!(call.symbol_id, function_decl.symbol_id);
    assert_ne!(call.symbol_id, local_decl.symbol_id);
}

#[test]
fn parameters_share_function_body_scope_and_nested_blocks_get_a_child_scope() {
    let text = "fn f(a: i32) -> i32 { let b: i32 = a; { let a: i32 = 2; b = a; } return a; }";
    let analysis = analyze(text);
    let parameter = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.name == "a" && symbol.kind == SymbolKind::Parameter)
        .expect("parameter symbol");
    let outer_local = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.name == "b")
        .expect("body local");
    let inner_local = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.name == "a" && symbol.kind == SymbolKind::Local)
        .expect("nested local");

    assert_eq!(parameter.scope_id, outer_local.scope_id);
    assert_ne!(parameter.scope_id, inner_local.scope_id);
    let inner_use = text.find("b = a").expect("inner use") + "b = ".len();
    let inner_use = text[..inner_use].encode_utf16().count();
    assert_eq!(
        analysis
            .references_at(inner_use)
            .first()
            .and_then(|occurrence| occurrence.symbol_id),
        Some(inner_local.id)
    );
}

#[test]
fn let_initializer_resolves_after_the_declaration_is_added_to_scope() {
    let text = "fn f() -> i32 { let value: i32 = value; return value; }";
    let analysis = analyze(text);
    let declaration = occurrence(&analysis, text, "value", true);
    let initializer_start = text.find("= value").expect("initializer") + 2;
    let initializer_offset = text[..initializer_start].encode_utf16().count();
    let initializer = analysis
        .occurrences
        .iter()
        .find(|occurrence| occurrence.span.start.offset == initializer_offset)
        .expect("initializer reference");

    assert_eq!(initializer.symbol_id, declaration.symbol_id);
}

#[test]
fn same_named_fields_bind_to_the_receiver_struct_field() {
    let text = "struct Left { value: i32; } struct Right { value: i32; } fn f(l: Left, r: Right) -> i32 { return l.value + r.value; }";
    let analysis = analyze(text);
    let fields: Vec<_> = analysis
        .symbols
        .iter()
        .filter(|symbol| symbol.name == "value" && symbol.kind == SymbolKind::Field)
        .collect();
    assert_eq!(fields.len(), 2);
    assert_ne!(fields[0].id, fields[1].id);

    let left_access = text.find("l.value").expect("left field") + 2;
    let right_access = text.find("r.value").expect("right field") + 2;
    let left_access = text[..left_access].encode_utf16().count();
    let right_access = text[..right_access].encode_utf16().count();
    assert_eq!(
        analysis.definition_at(left_access).map(|symbol| symbol.id),
        Some(fields[0].id)
    );
    assert_eq!(
        analysis.definition_at(right_access).map(|symbol| symbol.id),
        Some(fields[1].id)
    );
}

#[test]
fn unary_operations_do_not_keep_a_struct_receiver_binding() {
    let text = "struct Item { value: i32; } fn f(item: Item) -> i32 { return (-item).value; }";
    let analysis = analyze(text);
    let field_declaration = occurrence(&analysis, text, "value", true);
    let access = text.rfind("value").expect("field access");
    let access_offset = text[..access].encode_utf16().count();

    assert_eq!(analysis.definition_at(access_offset), None);
    let edits = analysis
        .rename(field_declaration.span.start.offset, "renamed")
        .expect("declaration rename remains safe");
    assert_eq!(edits.len(), 1);
}

#[test]
fn contract_requirements_and_effect_targets_reference_parameter_symbols() {
    let text = "unsafe fn bounded(items: slice<i32>, n: u32) -> i32 contract { requires n >= items.len; requires aligned(items.data, 4); effects read(items); } { return n; }";
    let analysis = analyze(text);
    let item_parameter = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.name == "items" && symbol.kind == SymbolKind::Parameter)
        .expect("items parameter");
    let n_parameter = analysis
        .symbols
        .iter()
        .find(|symbol| symbol.name == "n" && symbol.kind == SymbolKind::Parameter)
        .expect("n parameter");
    let function_scope = analysis
        .scopes
        .iter()
        .find(|scope| scope.id == item_parameter.scope_id)
        .expect("function scope");
    let effects = text.find("read(items)").expect("effect") + "read(".len();
    let effects_offset = text[..effects].encode_utf16().count();
    let requirement = text.find("requires n").expect("requirement") + "requires ".len();
    let requirement_offset = text[..requirement].encode_utf16().count();
    let slice_projection = text.find("items.len").expect("slice len") + "items.".len();
    let slice_projection_offset = text[..slice_projection].encode_utf16().count();
    let slice_data = text.find("items.data").expect("slice data") + "items.".len();
    let slice_data_offset = text[..slice_data].encode_utf16().count();
    let contract_builtin = text.find("aligned(").expect("contract builtin");
    let contract_builtin_offset = text[..contract_builtin].encode_utf16().count();

    assert!(function_scope.span.start.offset <= item_parameter.declaration.start.offset);
    assert!(function_scope.span.start.offset <= requirement_offset);
    assert!(function_scope.span.end.offset >= effects_offset);

    assert_eq!(
        analysis
            .definition_at(effects_offset)
            .map(|symbol| symbol.id),
        Some(item_parameter.id)
    );
    assert_eq!(
        analysis
            .definition_at(requirement_offset)
            .map(|symbol| symbol.id),
        Some(n_parameter.id)
    );
    assert_eq!(
        analysis
            .occurrences
            .iter()
            .find(|occurrence| occurrence.span.start.offset == slice_projection_offset)
            .map(|occurrence| occurrence.kind),
        Some(EditorOccurrenceKind::Builtin)
    );
    assert_eq!(
        analysis
            .occurrences
            .iter()
            .find(|occurrence| occurrence.span.start.offset == slice_data_offset)
            .map(|occurrence| occurrence.kind),
        Some(EditorOccurrenceKind::Builtin)
    );
    assert_eq!(
        analysis
            .occurrences
            .iter()
            .find(|occurrence| occurrence.span.start.offset == contract_builtin_offset)
            .map(|occurrence| occurrence.kind),
        Some(EditorOccurrenceKind::Builtin)
    );
    assert!(analysis.rename(effects_offset, "input").is_ok());
    assert_eq!(
        analysis.rename(slice_projection_offset, "length"),
        Err(EditorRenameError::NotRenameable)
    );
}

#[test]
fn utf16_offsets_and_rename_edits_remain_correct_after_astral_text() {
    let text = "// 😀\nfn f(value: i32) -> i32 { return value; }";
    let analysis = analyze(text);
    let declaration_start = text.find("value").expect("declaration");
    let declaration_offset = text[..declaration_start].encode_utf16().count();
    let value = occurrence(&analysis, text, "value", true);

    assert_eq!(value.span.start.offset, declaration_offset);
    let edits = analysis
        .rename(declaration_offset, "amount")
        .expect("rename");
    assert_eq!(edits.len(), 2);
    assert!(edits.iter().all(|edit| edit.new_text == "amount"));
}

#[test]
fn rename_rejects_invalid_names_entry_builtin_and_binding_conflicts() {
    let main = "fn main() -> i32 { return 0; }";
    let main_analysis = analyze(main);
    let main_decl = occurrence(&main_analysis, main, "main", true);
    assert_eq!(
        main_analysis.rename(main_decl.span.start.offset, "run"),
        Err(EditorRenameError::Main)
    );

    let source =
        "fn f(value: i32, other: i32) -> i32 { let local: i32 = value; return local + other; }";
    let analysis = analyze(source);
    let value = occurrence(&analysis, source, "value", true);
    assert_eq!(
        analysis.rename(value.span.start.offset, "not valid"),
        Err(EditorRenameError::InvalidName)
    );
    assert_eq!(
        analysis.rename(value.span.start.offset, "print_i32"),
        Err(EditorRenameError::ReservedName)
    );
    assert_eq!(
        analysis.rename(value.span.start.offset, "other"),
        Err(EditorRenameError::Conflict)
    );
}

#[test]
fn rename_rejects_a_name_that_would_capture_an_outer_reference() {
    let source = "fn f(value: i32) -> i32 { { let inner: i32 = 2; inner = value; } return value; }";
    let analysis = analyze(source);
    let value = occurrence(&analysis, source, "value", true);

    assert_eq!(
        analysis.rename(value.span.start.offset, "inner"),
        Err(EditorRenameError::BindingChanged)
    );
}

#[test]
fn builtin_calls_cannot_be_renamed_and_malformed_ast_fails_closed() {
    let text = "fn f() -> void { print_i32(1); }";
    let analysis = analyze(text);
    let builtin_start = text.find("print_i32").expect("builtin");
    let builtin_offset = text[..builtin_start].encode_utf16().count();
    assert_eq!(
        analysis
            .occurrences
            .iter()
            .find(|occurrence| occurrence.span.start.offset == builtin_offset)
            .map(|occurrence| occurrence.kind),
        Some(EditorOccurrenceKind::Builtin)
    );
    assert_eq!(
        analysis.rename(builtin_offset, "write_i32"),
        Err(EditorRenameError::NotRenameable)
    );

    let malformed = analyze("fn broken(value: i32) -> i32 { return value;");
    assert_eq!(
        malformed.rename(0, "changed"),
        Err(EditorRenameError::IncompleteAnalysis)
    );
}
