use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, OverflowMode, emit_wasm_kir_module,
    emit_wat_kir_module,
};

use super::support::compiler::{optimized_module, verified_artifact};
use super::support::{command::node_available, temp::temp_dir};
use std::{fs, path::Path, process::Command};

fn operators(bytes: &[u8]) -> Vec<Vec<String>> {
    wasmparser::Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| match payload.expect("valid Wasm payload") {
            wasmparser::Payload::CodeSectionEntry(body) => Some(
                body.get_operators_reader()
                    .expect("valid code body")
                    .into_iter()
                    .map(|operator| format!("{:?}", operator.expect("valid operator")))
                    .collect(),
            ),
            _ => None,
        })
        .collect()
}

fn exports(bytes: &[u8]) -> Vec<(String, String, u32)> {
    wasmparser::Parser::new(0)
        .parse_all(bytes)
        .filter_map(|payload| match payload.expect("valid Wasm payload") {
            wasmparser::Payload::ExportSection(exports) => Some(
                exports
                    .into_iter()
                    .map(|export| {
                        let export = export.expect("valid export");
                        (
                            export.name.to_string(),
                            format!("{:?}", export.kind),
                            export.index,
                        )
                    })
                    .collect::<Vec<_>>(),
            ),
            _ => None,
        })
        .flatten()
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct ModuleShape {
    sections: Vec<u8>,
    function_signatures: Vec<String>,
    memories: Vec<String>,
    globals: Vec<String>,
    metadata: Vec<Vec<u8>>,
}

fn module_shape(bytes: &[u8]) -> ModuleShape {
    let mut sections = Vec::new();
    let mut types = Vec::new();
    let mut function_type_indices = Vec::new();
    let mut memories = Vec::new();
    let mut globals = Vec::new();
    let mut metadata = Vec::new();
    for payload in wasmparser::Parser::new(0).parse_all(bytes) {
        match payload.expect("valid Wasm payload") {
            wasmparser::Payload::TypeSection(section) => {
                sections.push(1);
                types.extend(section.into_iter_err_on_gc_types().map(|ty| {
                    let ty = ty.expect("valid function type");
                    format!("{:?}->{:?}", ty.params(), ty.results())
                }));
            }
            wasmparser::Payload::FunctionSection(section) => {
                sections.push(3);
                function_type_indices
                    .extend(section.into_iter().map(|ty| ty.expect("valid type index")));
            }
            wasmparser::Payload::MemorySection(section) => {
                sections.push(5);
                memories.extend(
                    section
                        .into_iter()
                        .map(|memory| format!("{:?}", memory.expect("valid memory"))),
                );
            }
            wasmparser::Payload::GlobalSection(section) => {
                sections.push(6);
                globals.extend(section.into_iter().map(|global| {
                    let global = global.expect("valid global");
                    let init = global
                        .init_expr
                        .get_operators_reader()
                        .into_iter()
                        .map(|op| format!("{:?}", op.expect("valid global initializer")))
                        .collect::<Vec<_>>();
                    format!("{:?}:{init:?}", global.ty)
                }));
            }
            wasmparser::Payload::ExportSection(_) => sections.push(7),
            wasmparser::Payload::CodeSectionStart { .. } => sections.push(10),
            wasmparser::Payload::CustomSection(section) if section.name() == "ck.wasm.target" => {
                sections.push(0);
                metadata.push(section.data().to_vec());
            }
            wasmparser::Payload::CustomSection(section) if section.name() == "name" => {}
            wasmparser::Payload::Version { .. }
            | wasmparser::Payload::CodeSectionEntry(_)
            | wasmparser::Payload::End(_) => {}
            other => panic!("unexpected Wasm section in direct parity corpus: {other:?}"),
        }
    }
    ModuleShape {
        sections,
        function_signatures: function_type_indices
            .into_iter()
            .map(|index| types[index as usize].clone())
            .collect(),
        memories,
        globals,
        metadata,
    }
}

fn emit_cli(source: &str, features: &str, opt_level: u8, directory: &Path) -> (String, Vec<u8>) {
    fs::create_dir_all(directory).expect("create direct Wasm fixture directory");
    let source_path = directory.join("fixture.ck");
    fs::write(&source_path, source).expect("write CK fixture");
    let mut outputs = Vec::new();
    for (command, extension) in [("emit-wat", "wat"), ("emit-wasm", "wasm")] {
        let output_path = directory.join(format!("fixture.{extension}"));
        let result = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .arg(command)
            .arg(&source_path)
            .arg("--out")
            .arg(&output_path)
            .arg("--overflow")
            .arg("unchecked")
            .arg("--bounds")
            .arg("unchecked")
            .arg("--wasm-features")
            .arg(features)
            .arg("--opt-level")
            .arg(opt_level.to_string())
            .output()
            .expect("run ckc Wasm emission");
        assert!(
            result.status.success(),
            "{command} failed: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        outputs.push(fs::read(output_path).expect("read emitted artifact"));
    }
    (
        String::from_utf8(outputs.remove(0)).expect("WAT is UTF-8"),
        outputs.remove(0),
    )
}

#[test]
fn direct_binary_should_match_wat_oracle_for_deterministic_scalar_corpus() {
    let corpus = [
        r#"export fn add(a: i32, b: i32) -> i32 { return a + b; }"#,
        r#"export fn length(items: slice<i32>) -> u32 { return items.len; }"#,
        r#"
            export fn helper(a: i64) -> i64 { return a * 3; }
            export fn caller(a: i64) -> i64 { return helper(a) + 1; }
        "#,
        r#"
            export fn choose(a: i32, b: i32, pick: bool) -> i32 {
                if pick { return a; }
                return b;
            }
        "#,
        r#"
            fn used(value: i32) -> i32 { return value + 1; }
            fn dead(value: i32) -> i32 { return value - 1; }
            export fn live(value: i32) -> i32 { return used(value); }
        "#,
        r#"fn unused(value: i32) -> i32 { return value + 1; }"#,
        r#"
            fn dead_output() -> void { print_i32(7); }
            export fn live(value: i32) -> i32 { return value + 1; }
        "#,
        include_str!("../../tests/fixtures/performance/f64_edges.ck"),
        include_str!("../../examples/wasm/cursor_copy.ck"),
        include_str!("../../examples/wasm/field_offset.ck"),
    ];
    for source in corpus {
        for opt_level in [0, 1, 2, 3] {
            let optimized = optimized_module(
                source,
                opt_level,
                KirConsumer::WebAssembly,
                OverflowMode::Unchecked,
                BoundsMode::Unchecked,
            );
            let module = verified_artifact(&optimized);
            let options = EmitWasmOptions { opt_level };
            let wat = emit_wat_kir_module(module, options).expect("WAT emission");
            let direct = emit_wasm_kir_module(module, options).expect("direct binary emission");
            let parsed = wat::parse_str(&wat).expect("WAT oracle must parse");
            assert_eq!(operators(&direct), operators(&parsed), "{wat}");
            assert_eq!(exports(&direct), exports(&parsed), "{wat}");
            assert_eq!(module_shape(&direct), module_shape(&parsed), "{wat}");
            assert_eq!(
                direct,
                emit_wasm_kir_module(module, options).expect("deterministic binary"),
            );
        }
    }
}

#[test]
fn direct_binary_should_match_wat_oracle_for_simd_bulk_and_profile_matrix() {
    let cases = [
        (
            "simd-map-with-dead-output",
            format!(
                "{}\nfn dead_output() -> void {{ print_i32(7); }}\n",
                include_str!("../../examples/wasm/f64_map.ck")
            ),
        ),
        (
            "bulk-copy",
            include_str!("../../examples/wasm/cursor_copy.ck").to_string(),
        ),
        (
            "bulk-fill",
            include_str!("../../examples/wasm/bulk_fill.ck").to_string(),
        ),
    ];
    let root = temp_dir("ck-wasm-direct-parity");
    for (case, source) in cases {
        for features in ["baseline", "simd128"] {
            for opt_level in [0, 3] {
                let directory = root.join(format!("{case}-{features}-O{opt_level}"));
                let (wat, direct) = emit_cli(&source, features, opt_level, &directory);
                let parsed = wat::parse_str(&wat).expect("WAT oracle parses");
                assert_eq!(
                    operators(&direct),
                    operators(&parsed),
                    "{case} {features} O{opt_level}"
                );
                assert_eq!(
                    exports(&direct),
                    exports(&parsed),
                    "{case} {features} O{opt_level}"
                );
                assert_eq!(
                    module_shape(&direct),
                    module_shape(&parsed),
                    "{case} {features} O{opt_level}"
                );
                if case == "simd-map-with-dead-output" && features == "simd128" && opt_level == 3 {
                    assert!(
                        wat.contains("v128"),
                        "checked SIMD path should be exercised"
                    );
                    assert!(
                        !wat.contains("dead_output"),
                        "dead runtime function must be pruned"
                    );
                }
                if case == "bulk-copy" && opt_level == 3 {
                    assert!(wat.contains("memory.copy"));
                }
                if case == "bulk-fill" && opt_level == 3 {
                    assert!(wat.contains("memory.fill"));
                }
            }
        }
    }
    fs::remove_dir_all(root).expect("remove direct parity fixtures");
}

#[test]
fn direct_binary_should_match_wat_oracle_on_trap_visible_memory() {
    if !node_available() {
        return;
    }
    let root = temp_dir("ck-wasm-direct-trap");
    let (wat, direct) = emit_cli(
        include_str!("../../examples/wasm/cursor_copy.ck"),
        "baseline",
        3,
        &root,
    );
    let parsed = wat::parse_str(&wat).expect("WAT oracle parses");
    let direct_path = root.join("direct.wasm");
    let parsed_path = root.join("parsed.wasm");
    fs::write(&direct_path, direct).expect("write direct binary");
    fs::write(&parsed_path, parsed).expect("write WAT oracle binary");
    let script = r#"
const fs = require('node:fs');
function run(path) {
  const instance = new WebAssembly.Instance(new WebAssembly.Module(fs.readFileSync(path)));
  const bytes = new Uint8Array(instance.exports.memory.buffer);
  const words = new DataView(bytes.buffer);
  words.setUint32(0, 0x12345678, true);
  words.setUint32(4, 0x9abcdef0, true);
  let trapped = false;
  try { instance.exports.copy_u32(65532, 0, 0, 2); }
  catch (error) { trapped = error instanceof WebAssembly.RuntimeError; }
  if (!trapped || words.getUint32(65532, true) !== 0x12345678) {
    throw new Error('expected trap after the first visible write');
  }
  return Buffer.from(bytes);
}
if (!run(process.argv[1]).equals(run(process.argv[2]))) {
  throw new Error('direct and WAT oracle differ after a partial-write trap');
}
"#;
    let output = Command::new("node")
        .arg("-e")
        .arg(script)
        .arg(&direct_path)
        .arg(&parsed_path)
        .output()
        .expect("run Node direct-vs-WAT trap comparison");
    assert!(
        output.status.success(),
        "Node trap comparison failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_dir_all(root).expect("remove direct trap fixtures");
}
