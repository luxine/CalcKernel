use std::{fs, process::Command};

use super::support::compiler::{optimized_module, verified_artifact};
use calckernel::{
    BoundsMode, EmitLlvmOptions, KirConsumer, MirFunction, MirParam,
    MirPrimitiveTypeName as Primitive, MirStruct, MirStructField, MirType, NativeAbiArgumentRole,
    NativeAbiClassifier, NativeAbiExtension, NativeAbiPassMode, NativeAbiRegister,
    NativeAbiRegisterClass as RegisterClass, NativeAbiTarget, NativeContext, NativeHeaderMode,
    NativeTarget, OverflowMode, emit_c_kir_header, emit_native_header, lower_native_kir_module,
};

fn primitive(name: Primitive) -> MirType {
    MirType::Primitive(name)
}

fn field(name: &str, type_node: MirType) -> MirStructField {
    MirStructField {
        name: name.to_string(),
        type_node,
    }
}

fn structure(name: &str, fields: Vec<MirStructField>) -> MirStruct {
    MirStruct {
        name: name.to_string(),
        fields,
    }
}

fn fixture_structs() -> Vec<MirStruct> {
    vec![
        structure("B1", vec![field("value", primitive(Primitive::Bool))]),
        structure("I4", vec![field("value", primitive(Primitive::I32))]),
        structure("I8", vec![field("value", primitive(Primitive::I64))]),
        structure(
            "I16",
            vec![
                field("a", primitive(Primitive::I64)),
                field("b", primitive(Primitive::I64)),
            ],
        ),
        structure(
            "I24",
            vec![
                field("a", primitive(Primitive::I64)),
                field("b", primitive(Primitive::I64)),
                field("c", primitive(Primitive::I64)),
            ],
        ),
        structure(
            "DI",
            vec![
                field("a", primitive(Primitive::F64)),
                field("b", primitive(Primitive::I32)),
            ],
        ),
        structure(
            "H2",
            vec![
                field("a", primitive(Primitive::F64)),
                field("b", primitive(Primitive::F64)),
            ],
        ),
        structure(
            "H4",
            (0..4)
                .map(|index| field(&format!("v{index}"), primitive(Primitive::F64)))
                .collect(),
        ),
        structure(
            "H5",
            (0..5)
                .map(|index| field(&format!("v{index}"), primitive(Primitive::F64)))
                .collect(),
        ),
        structure(
            "WithSlice",
            vec![field(
                "items",
                MirType::Slice(Box::new(primitive(Primitive::I32))),
            )],
        ),
    ]
}

fn classifier(target: NativeAbiTarget) -> NativeAbiClassifier {
    NativeAbiClassifier::new(target, &fixture_structs()).expect("valid ABI fixture structs")
}

fn direct_registers(mode: &NativeAbiPassMode) -> &[NativeAbiRegister] {
    match mode {
        NativeAbiPassMode::Direct { registers } => registers,
        NativeAbiPassMode::Indirect { .. } => panic!("expected direct ABI mode"),
    }
}

#[test]
fn abi_target_should_parse_all_six_release_triples() {
    for (triple, expected) in [
        ("x86_64-unknown-linux-gnu", NativeAbiTarget::SysvX86_64),
        ("x86_64-apple-darwin", NativeAbiTarget::DarwinX86_64),
        ("aarch64-unknown-linux-gnu", NativeAbiTarget::Aapcs64Linux),
        ("arm64-apple-darwin", NativeAbiTarget::Aapcs64Darwin),
        ("x86_64-pc-windows-msvc", NativeAbiTarget::WindowsX86_64),
        ("aarch64-pc-windows-msvc", NativeAbiTarget::WindowsArm64),
    ] {
        assert_eq!(
            NativeAbiTarget::from_triple(triple),
            Ok(expected),
            "{triple}"
        );
    }
    assert!(NativeAbiTarget::from_triple("wasm32-unknown-unknown").is_err());
}

#[test]
fn abi_layout_should_cover_primitives_pointers_slices_and_struct_padding() {
    for target in NativeAbiTarget::ALL {
        let classifier = classifier(target);
        assert_eq!(
            classifier.layout(&primitive(Primitive::Bool)).unwrap().size,
            1
        );
        assert_eq!(
            classifier.layout(&primitive(Primitive::I32)).unwrap().size,
            4
        );
        assert_eq!(
            classifier
                .layout(&primitive(Primitive::I64))
                .unwrap()
                .alignment,
            8
        );
        assert_eq!(
            classifier.layout(&primitive(Primitive::F64)).unwrap().size,
            8
        );
        assert_eq!(
            classifier
                .layout(&MirType::Pointer(Box::new(primitive(Primitive::I32))))
                .unwrap()
                .size,
            8
        );
        assert_eq!(
            classifier
                .layout(&MirType::Slice(Box::new(primitive(Primitive::I32))))
                .unwrap(),
            calckernel::NativeAbiLayout {
                size: 16,
                alignment: 8
            }
        );
        assert_eq!(
            classifier.layout(&MirType::Struct("DI".into())).unwrap(),
            calckernel::NativeAbiLayout {
                size: 16,
                alignment: 8
            }
        );
    }
}

#[test]
fn sysv_and_darwin_x64_should_classify_eightbytes_and_memory_boundaries() {
    for target in [NativeAbiTarget::SysvX86_64, NativeAbiTarget::DarwinX86_64] {
        let classifier = classifier(target);
        let i16 = classifier
            .classify_parameter(&MirType::Struct("I16".into()))
            .unwrap();
        assert_eq!(
            direct_registers(&i16.mode),
            [
                NativeAbiRegister::integer(64),
                NativeAbiRegister::integer(64)
            ]
        );
        let mixed = classifier
            .classify_parameter(&MirType::Struct("DI".into()))
            .unwrap();
        assert_eq!(
            direct_registers(&mixed.mode),
            [
                NativeAbiRegister::floating(64),
                NativeAbiRegister::integer(32)
            ]
        );
        assert_eq!(
            classifier
                .classify_parameter(&MirType::Struct("I24".into()))
                .unwrap()
                .mode,
            NativeAbiPassMode::Indirect {
                by_value: true,
                alignment: 8
            }
        );
    }
}

#[test]
fn aapcs64_variants_should_use_integer_chunks_hfa_and_indirect_boundaries() {
    for target in [
        NativeAbiTarget::Aapcs64Linux,
        NativeAbiTarget::Aapcs64Darwin,
        NativeAbiTarget::WindowsArm64,
    ] {
        let classifier = classifier(target);
        let i4 = classifier
            .classify_parameter(&MirType::Struct("I4".into()))
            .unwrap();
        assert_eq!(direct_registers(&i4.mode), [NativeAbiRegister::integer(64)]);
        let h4 = classifier
            .classify_parameter(&MirType::Struct("H4".into()))
            .unwrap();
        assert_eq!(
            direct_registers(&h4.mode),
            [NativeAbiRegister::floating(64); 4]
        );
        assert!(matches!(
            classifier
                .classify_parameter(&MirType::Struct("H5".into()))
                .unwrap()
                .mode,
            NativeAbiPassMode::Indirect {
                by_value: false,
                alignment: 8
            }
        ));
    }
}

#[test]
fn windows_x64_should_pass_only_power_of_two_aggregates_up_to_64_bits_directly() {
    let classifier = classifier(NativeAbiTarget::WindowsX86_64);
    for (name, bits) in [("B1", 8), ("I4", 32), ("I8", 64)] {
        let value = classifier
            .classify_parameter(&MirType::Struct(name.into()))
            .unwrap();
        assert_eq!(
            direct_registers(&value.mode),
            [NativeAbiRegister::integer(bits)]
        );
    }
    for name in ["I16", "DI", "H2", "I24"] {
        assert!(matches!(
            classifier
                .classify_parameter(&MirType::Struct(name.into()))
                .unwrap()
                .mode,
            NativeAbiPassMode::Indirect {
                by_value: false,
                alignment: 8
            }
        ));
    }
}

#[test]
fn scalar_bool_extension_should_match_each_target_family() {
    for target in NativeAbiTarget::ALL {
        let value = classifier(target)
            .classify_parameter(&primitive(Primitive::Bool))
            .unwrap();
        assert_eq!(
            value.extension,
            if matches!(
                target,
                NativeAbiTarget::SysvX86_64
                    | NativeAbiTarget::DarwinX86_64
                    | NativeAbiTarget::Aapcs64Darwin
                    | NativeAbiTarget::WindowsX86_64
            ) {
                NativeAbiExtension::Zero
            } else {
                NativeAbiExtension::None
            },
            "{target:?}"
        );
    }
}

#[test]
fn function_classifier_should_flatten_slices_and_model_checked_result_pointer() {
    let function = MirFunction {
        name: "transform".into(),
        exported: true,
        params: vec![
            MirParam {
                name: "items".into(),
                type_node: MirType::Slice(Box::new(primitive(Primitive::I32))),
            },
            MirParam {
                name: "large".into(),
                type_node: MirType::Struct("I24".into()),
            },
        ],
        return_type: MirType::Struct("I24".into()),
        locals: vec![],
        blocks: vec![],
    };
    let classifier = classifier(NativeAbiTarget::SysvX86_64);

    let unchecked = classifier
        .classify_function(&function, false)
        .expect("unchecked ABI");
    assert!(unchecked.hidden_result.is_some());
    assert_eq!(
        unchecked
            .parameters
            .iter()
            .map(|argument| argument.role)
            .collect::<Vec<_>>(),
        vec![
            NativeAbiArgumentRole::SliceData(0),
            NativeAbiArgumentRole::SliceLength(0),
            NativeAbiArgumentRole::Source(1),
        ]
    );

    let checked = classifier
        .classify_function(&function, true)
        .expect("checked ABI");
    assert!(checked.hidden_result.is_none());
    assert_eq!(
        direct_registers(&checked.return_value.mode),
        [NativeAbiRegister {
            class: RegisterClass::Integer,
            bits: 32
        }]
    );
    assert_eq!(
        checked.parameters.last().unwrap().role,
        NativeAbiArgumentRole::CheckedResult
    );
}

#[test]
fn abi_development_oracle_fixture_should_cover_all_classifier_shapes() {
    let fixture = include_str!("../fixtures/native/abi/shapes.c");
    for symbol in ["ck_bool", "ck_i4", "ck_i16", "ck_i24", "ck_di", "ck_h4"] {
        assert!(fixture.contains(symbol), "missing {symbol}");
    }
}

#[test]
fn pinned_clang_22_should_confirm_all_six_classifier_calling_sequences() {
    let Some(clang) = super::support::oracle::clang_oracle_22() else {
        return;
    };
    for (target, triple) in [
        (NativeAbiTarget::SysvX86_64, "x86_64-unknown-linux-gnu"),
        (NativeAbiTarget::DarwinX86_64, "x86_64-apple-macos11"),
        (NativeAbiTarget::Aapcs64Linux, "aarch64-unknown-linux-gnu"),
        (NativeAbiTarget::Aapcs64Darwin, "arm64-apple-macos11"),
        (NativeAbiTarget::WindowsX86_64, "x86_64-pc-windows-msvc"),
        (NativeAbiTarget::WindowsArm64, "aarch64-pc-windows-msvc"),
    ] {
        let output = Command::new(&clang)
            .args([
                "-target",
                triple,
                "-S",
                "-emit-llvm",
                "-O0",
                "-ffreestanding",
                "-nostdinc",
                "tests/fixtures/native/abi/shapes.c",
                "-o",
                "-",
            ])
            .output()
            .expect("run pinned Clang ABI oracle");
        assert!(
            output.status.success(),
            "{triple}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let ir = String::from_utf8(output.stdout).expect("Clang LLVM IR UTF-8");
        let bool_is_extended = matches!(
            target,
            NativeAbiTarget::SysvX86_64
                | NativeAbiTarget::DarwinX86_64
                | NativeAbiTarget::Aapcs64Darwin
                | NativeAbiTarget::WindowsX86_64
        );
        let bool_line = definition(&ir, "ck_bool");
        assert_eq!(
            bool_line.contains("zeroext i1"),
            bool_is_extended,
            "{triple}: {bool_line}"
        );

        let i24 = definition(&ir, "ck_i24");
        assert!(
            i24.contains("sret(%struct.CK_I24) align 8"),
            "{triple}: {i24}"
        );
        assert_eq!(
            i24.contains("byval(%struct.CK_I24) align 8"),
            matches!(
                target,
                NativeAbiTarget::SysvX86_64 | NativeAbiTarget::DarwinX86_64
            ),
            "{triple}: {i24}"
        );
        let h4 = definition(&ir, "ck_h4");
        let hfa = matches!(
            target,
            NativeAbiTarget::Aapcs64Linux
                | NativeAbiTarget::Aapcs64Darwin
                | NativeAbiTarget::WindowsArm64
        );
        assert_eq!(h4.contains("[4 x double]"), hfa, "{triple}: {h4}");
        assert_eq!(h4.contains("sret(%struct.CK_H4)"), !hfa, "{triple}: {h4}");
        let i16 = definition(&ir, "ck_i16");
        assert_eq!(
            i16.contains("sret(%struct.CK_I16)"),
            target == NativeAbiTarget::WindowsX86_64,
            "{triple}: {i16}"
        );
    }
}

fn definition<'ir>(ir: &'ir str, symbol: &str) -> &'ir str {
    ir.lines()
        .find(|line| line.starts_with("define ") && line.contains(&format!("@{symbol}(")))
        .unwrap_or_else(|| panic!("missing Clang definition for {symbol}:\n{ir}"))
}

fn export_fixture_kir(
    overflow_mode: OverflowMode,
    bounds_mode: BoundsMode,
) -> calckernel::KirPassManagerResult {
    optimized_module(
        include_str!("../fixtures/native/abi/export_shapes.ck"),
        0,
        KirConsumer::NativeLibrary,
        overflow_mode,
        bounds_mode,
    )
}

fn native_abi_ir(overflow_mode: OverflowMode, bounds_mode: BoundsMode) -> String {
    let kir = export_fixture_kir(overflow_mode, bounds_mode);
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("lower native ABI thunks")
        .verify()
        .expect("verify native ABI thunks")
        .to_ir_string()
        .expect("print native ABI thunks")
}

#[test]
fn native_header_should_share_c_types_and_select_artifact_export_mode() {
    let kir = export_fixture_kir(OverflowMode::Unchecked, BoundsMode::Unchecked);
    let artifact = verified_artifact(&kir);
    let c_header = emit_c_kir_header(artifact);
    let dynamic = emit_native_header(artifact, NativeHeaderMode::Dynamic);
    let static_header = emit_native_header(artifact, NativeHeaderMode::StaticOrObject);
    for declaration in [
        "typedef struct Small",
        "typedef struct Big",
        "CK_API Small echo_small(Small value);",
        "CK_API uint32_t slice_count(int32_t* items_data, uint32_t items_len);",
    ] {
        assert!(c_header.contains(declaration), "{c_header}");
        assert!(dynamic.contains(declaration), "{dynamic}");
        assert!(static_header.contains(declaration), "{static_header}");
    }
    assert!(dynamic.contains("__declspec(dllimport)"), "{dynamic}");
    assert!(!static_header.contains("dllimport"), "{static_header}");
    assert!(!static_header.contains("dllexport"), "{static_header}");
}

#[test]
fn generated_native_header_should_compile_as_strict_c11_with_pinned_clang() {
    let Some(clang) = super::support::oracle::clang_oracle_22() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("ckc-header-oracle-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).expect("create header oracle directory");
    let header = root.join("exports.h");
    let harness = root.join("harness.c");
    fs::write(
        &header,
        emit_native_header(
            verified_artifact(&export_fixture_kir(
                OverflowMode::Unchecked,
                BoundsMode::Unchecked,
            )),
            NativeHeaderMode::Dynamic,
        ),
    )
    .expect("write generated header");
    fs::write(&harness, "#include \"exports.h\"\n").expect("write header harness");
    let output = Command::new(clang)
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror", "-fsyntax-only"])
        .arg(&harness)
        .arg("-I")
        .arg(&root)
        .output()
        .expect("compile generated header");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(root).expect("remove header oracle directory");
}

#[test]
fn native_llvm_should_hide_internal_signatures_behind_host_c_abi_thunks() {
    let text = native_abi_ir(OverflowMode::Unchecked, BoundsMode::Unchecked);
    for symbol in ["echo_bool", "echo_small", "echo_big", "slice_count"] {
        assert!(
            text.contains(&format!("@{symbol}(")),
            "missing export {symbol}:\n{text}"
        );
        assert!(
            text.contains(&format!("@__ck_impl_{symbol}(")),
            "missing internal implementation {symbol}:\n{text}"
        );
    }
    assert!(!text.contains("define %struct.Small @echo_small"), "{text}");
    assert!(!text.contains("define %struct.Big @echo_big"), "{text}");
    if cfg!(target_arch = "aarch64") {
        assert!(
            text.contains("define [2 x i64] @echo_small([2 x i64]"),
            "{text}"
        );
        assert!(text.contains("sret(%struct.Big)"), "{text}");
    }
}

#[test]
fn checked_native_thunks_should_return_status_and_keep_result_pointer_explicit() {
    let text = native_abi_ir(OverflowMode::Checked, BoundsMode::Checked);
    assert!(
        text.lines()
            .any(|line| line.starts_with("define ") && line.contains(" i32 @echo_big(")),
        "{text}"
    );
    assert!(
        !text
            .lines()
            .any(|line| line.starts_with("define ") && line.contains(" void @echo_big(")),
        "{text}"
    );
    assert!(!text.contains("sret(%struct.Big)"), "{text}");
    assert!(text.contains("@__ck_impl_echo_big(%struct.Big"), "{text}");
}
