use calckernel::{
    BoundsMode, KirConsumer, NativeHeaderMode, OverflowMode, SourceFile, annotate_unsafe_contracts,
    check, emit_native_header,
};

use super::support::compiler::{optimized_module, verified_artifact};

#[test]
fn header_unsafe_contract_comments_should_normalize_flattened_slice_fields_without_abi_drift() {
    let source_text = r#"
        export unsafe fn kernel(x: slice<i32>, y: slice<i32>, n: u32) -> void
        contract {
          requires n + 2 <= x.len;
          requires multiple_of(n, 4);
          requires noalias(x, y);
          requires aligned(y.data, 32);
          effects read(x), write(y);
        }
        { return; }
        "#;
    let checked = check(&SourceFile::new("header.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let kir = optimized_module(
        source_text,
        3,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let plain = emit_native_header(verified_artifact(&kir), NativeHeaderMode::StaticOrObject);
    let annotated = annotate_unsafe_contracts(&plain, &checked.checked_program);

    for line in [
        "requires n + 2 <= x_len",
        "requires multiple_of(n, 4)",
        "requires noalias(x_data[0..x_len], y_data[0..y_len])",
        "requires aligned(y_data, 32)",
        "effects read(x_data[0..x_len]), write(y_data[0..y_len])",
    ] {
        assert!(annotated.contains(line), "missing {line}:\n{annotated}");
    }
    let without_comments = annotated
        .lines()
        .filter(|line| {
            !line.starts_with("/* CK unsafe ") && !line.starts_with(" * ") && *line != " */"
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(without_comments, plain.trim_end());
}
