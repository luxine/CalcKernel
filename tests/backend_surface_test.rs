use calckernel::{
    BoundsMode, EmitCOptions, EmitLlvmOptions, EmitWasmOptions, OverflowMode, SourceFile, check,
    emit_c_header, emit_c_module, emit_c_module_with_header, emit_llvm_module, emit_wasm_module,
    emit_wasm_module_with_options, emit_wat_module, emit_wat_module_with_options, lower_to_mir,
};

#[test]
fn backend_public_surface_and_defaults_should_remain_stable() {
    let c_options = EmitCOptions::default();
    assert_eq!(c_options.overflow_mode, OverflowMode::Unchecked);
    assert_eq!(c_options.bounds_mode, BoundsMode::Unchecked);
    assert_eq!(c_options.opt_level, 0);
    assert_eq!(EmitWasmOptions::default().opt_level, 0);
    assert_eq!(EmitLlvmOptions::default().source_file_name, None);
    assert_eq!(EmitLlvmOptions::default().target_triple, None);

    let checked = check(&SourceFile::new(
        "surface.ck",
        "export fn identity(value: i64) -> i64 { return value; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");

    let c = emit_c_module(&mir, c_options);
    let header = emit_c_header(&mir, c_options);
    let c_with_header = emit_c_module_with_header(&mir, c_options, "surface.h");
    assert!(c.contains("identity"));
    assert!(header.contains("CK_API"));
    assert!(c_with_header.starts_with("#include \"surface.h\""));

    let wat = emit_wat_module(&mir);
    assert_eq!(
        wat,
        emit_wat_module_with_options(&mir, EmitWasmOptions::default())
    );
    let wasm = emit_wasm_module(&mir).expect("minimal WAT should assemble");
    assert_eq!(
        wasm,
        emit_wasm_module_with_options(&mir, EmitWasmOptions::default())
            .expect("minimal WAT should assemble with explicit defaults")
    );
    assert_eq!(&wasm[..4], b"\0asm");

    let llvm = emit_llvm_module(&mir, &EmitLlvmOptions::default());
    assert!(llvm.contains("define i64 @identity"));
}
