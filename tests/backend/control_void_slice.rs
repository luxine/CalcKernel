use std::{fs, path::Path, process::Command};

use calckernel::{
    BoundsMode, EmitCOptions, MirModule, MirPassBoundsMode, MirPassContext, MirPassOverflowMode,
    MirPassTargetBackend, OverflowMode, SourceFile, build_mir_optimization_pipeline, check,
    emit_c_module, lower_to_mir, run_mir_pass_pipeline,
};

#[cfg(feature = "native-toolchain")]
use calckernel::{EmitWasmOptions, emit_wasm_module_with_options};

#[cfg(feature = "native-toolchain")]
use calckernel::{
    EmitLlvmOptions, NativeContext, NativeOptimizationLevel, NativeTarget, lower_native_llvm_module,
};

use super::support::command::run_stdout;
use super::support::temp::unique_id;

const COMBINED_SOURCE: &str = r#"
struct Item {
  value: i32;
}

fn tail(items: slice<Item>, start: u32) -> slice<Item> {
  return items[start..items.len];
}

fn observe(items: slice<Item>) -> void {
    let len: u32 = items.len;
    if len == 0 {
        return;
    }
}

fn write_selected(items: slice<Item>, index: u32, delta: i32) -> void {
  items[index].value = items[index].value + delta;
  return;
}

export fn process(
  items: slice<Item>,
  start: u32,
  stop_value: i32,
  skip_value: i32,
  delta: i32
) -> void {
  observe(items);
  let view: slice<Item> = tail(items, start);
  let i: u32 = 0;
  while i < view.len {
    if view[i].value == stop_value {
      break;
    }
    if view[i].value == skip_value {
      i = i + 1;
      continue;
    }
    write_selected(view, i, delta);
    i = i + 1;
  }
  return;
}

export fn write_at(items: slice<Item>, index: u32, value: i32) -> void {
  items[index].value = value;
  return;
}

export fn write_offset(
  items: slice<Item>,
  base: u32,
  offset: u32,
  value: i32
) -> void {
  items[base + offset].value = value;
  return;
}
"#;

#[test]
#[cfg(feature = "native-toolchain")]
fn combined_control_void_slice_should_match_unchecked_backends_at_all_opt_levels() {
    let unique = unique_id();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_combined_unchecked_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    for opt_level in 0..=3 {
        let c_module = optimized_module(
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Unchecked,
            MirPassTargetBackend::C,
        );
        let c = emit_c_module(
            &c_module,
            EmitCOptions {
                overflow_mode: OverflowMode::Unchecked,
                bounds_mode: BoundsMode::Unchecked,
                opt_level,
            },
        );
        let c_harness = format!(
            r#"
{c}
#include <stdio.h>
int main(void) {{
  Item items[4] = {{{{1}}, {{2}}, {{3}}, {{4}}}};
  process(items, 4, 0, 4, 2, 10);
  printf("%d,%d,%d,%d\n", items[0].value, items[1].value, items[2].value, items[3].value);
  return 0;
}}
"#
        );
        let c_output =
            compile_and_run_c_text(&dir, &format!("combined_c_o{opt_level}"), &c_harness);

        let llvm_module = optimized_module(
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Unchecked,
            MirPassTargetBackend::Llvm,
        );
        let context = NativeContext::new().expect("native context");
        let target = NativeTarget::host().expect("native target");
        let object = lower_native_llvm_module(
            &context,
            &target,
            &llvm_module,
            &EmitLlvmOptions {
                source_file_name: Some("combined.ck".to_string()),
                target_triple: None,
            },
        )
        .expect("structural LLVM")
        .verify()
        .expect("verify structural LLVM")
        .optimize(
            &target,
            NativeOptimizationLevel::try_from(opt_level).expect("optimization level"),
        )
        .and_then(|module| target.emit_object(module))
        .expect("emit structural LLVM object");
        let object_path = dir.join(format!("combined_o{opt_level}.o"));
        let llvm_harness = dir.join(format!("combined_llvm_o{opt_level}.c"));
        let llvm_binary = dir.join(format!("combined_llvm_o{opt_level}"));
        fs::write(&object_path, object.as_bytes()).expect("write LLVM object");
        fs::write(
            &llvm_harness,
            r#"
#include <stdint.h>
#include <stdio.h>
typedef struct Item { int32_t value; } Item;
void process(Item* data, uint32_t len, uint32_t start, int32_t stop, int32_t skip, int32_t delta);
int main(void) {
  Item items[4] = {{1}, {2}, {3}, {4}};
  process(items, 4, 0, 4, 2, 10);
  printf("%d,%d,%d,%d\n", items[0].value, items[1].value, items[2].value, items[3].value);
  return 0;
}
"#,
        )
        .expect("write LLVM harness");
        compile_native(&[&object_path, &llvm_harness], &llvm_binary);
        let llvm_output = run_stdout(&llvm_binary);

        let wasm_module = optimized_module(
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Unchecked,
            MirPassTargetBackend::Wasm,
        );
        let wasm = emit_wasm_module_with_options(&wasm_module, EmitWasmOptions { opt_level })
            .expect("emit WASM");
        let wasm_path = dir.join(format!("combined_o{opt_level}.wasm"));
        fs::write(&wasm_path, wasm).expect("write WASM");
        let node = Command::new("node")
            .arg("-e")
            .arg(
                r#"
const fs = require("node:fs");
WebAssembly.instantiate(fs.readFileSync(process.argv[1])).then(({instance}) => {
  const view = new DataView(instance.exports.memory.buffer);
  [1, 2, 3, 4].forEach((value, index) => view.setInt32(index * 4, value, true));
  instance.exports.process(0, 4, 0, 4, 2, 10);
  console.log([0, 1, 2, 3].map((index) => view.getInt32(index * 4, true)).join(","));
}).catch((error) => { console.error(error); process.exit(1); });
"#,
            )
            .arg(&wasm_path)
            .output()
            .expect("run WASM");
        assert!(
            node.status.success(),
            "O{opt_level} node stderr:\n{}",
            String::from_utf8_lossy(&node.stderr)
        );
        let wasm_output = String::from_utf8(node.stdout).expect("WASM stdout UTF-8");

        assert_eq!(c_output, "11,2,13,4\n", "C O{opt_level}");
        assert_eq!(llvm_output, c_output, "LLVM O{opt_level}");
        assert_eq!(wasm_output, c_output, "WASM O{opt_level}");
    }
}

#[test]
fn combined_control_void_slice_should_pass_checked_c_and_preserve_failure_order() {
    let unique = unique_id();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_combined_checked_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    for opt_level in 0..=3 {
        let module = optimized_module(
            opt_level,
            MirPassOverflowMode::Checked,
            MirPassBoundsMode::Checked,
            MirPassTargetBackend::C,
        );
        let c = emit_c_module(
            &module,
            EmitCOptions {
                overflow_mode: OverflowMode::Checked,
                bounds_mode: BoundsMode::Checked,
                opt_level,
            },
        );
        let harness = format!(
            r#"
{c}
#include <string.h>
int main(void) {{
  Item items[4] = {{{{1}}, {{2}}, {{3}}, {{4}}}};
  if (process(items, 4, 0, 4, 2, 10) != CK_OK) return 1;
  Item snapshot[4];
  memcpy(snapshot, items, sizeof(items));
  if (write_at(items, 4, 4, 99) != CK_ERR_OUT_OF_BOUNDS) return 2;
  if (memcmp(snapshot, items, sizeof(items)) != 0) return 3;
  if (write_offset(items, 4, UINT32_MAX, 1, 99) != CK_ERR_OVERFLOW) return 4;
  if (memcmp(snapshot, items, sizeof(items)) != 0) return 5;
  return 0;
}}
"#
        );
        let output =
            compile_and_run_c_text(&dir, &format!("combined_checked_o{opt_level}"), &harness);
        assert_eq!(output, "", "checked C O{opt_level}");
    }
}

#[test]
fn combined_control_void_slice_should_reject_checked_bounds_on_wasm_backends() {
    let unique = unique_id();
    let dir = std::env::temp_dir().join(format!("rust_calckernel_combined_reject_{unique}"));
    fs::create_dir_all(&dir).expect("create temp dir");
    let source = dir.join("combined.ck");
    fs::write(&source, COMBINED_SOURCE).expect("write fixture");

    let cases = vec![
        ("emit-wat", "WASM", None),
        ("emit-wasm", "WASM", Some(dir.join("combined.wasm"))),
    ];
    for (command, backend, out) in cases {
        let mut args = vec![
            command.into(),
            source.as_os_str().to_owned(),
            "--bounds".into(),
            "checked".into(),
        ];
        if let Some(out) = out {
            args.push("--out".into());
            args.push(out.into_os_string());
        }
        let output = Command::new(env!("CARGO_BIN_EXE_ckc"))
            .args(args)
            .output()
            .expect("run ckc rejection case");
        assert_eq!(output.status.code(), Some(1), "{command}");
        let stderr = String::from_utf8(output.stderr).expect("stderr UTF-8");
        assert!(
            stderr.contains(&format!(
                "{backend} backend does not support --bounds checked yet."
            )),
            "{command}: {stderr}"
        );
    }
}

fn optimized_module(
    opt_level: u8,
    overflow_mode: MirPassOverflowMode,
    bounds_mode: MirPassBoundsMode,
    target_backend: MirPassTargetBackend,
) -> MirModule {
    let checked = check(&SourceFile::new("combined.ck", COMBINED_SOURCE));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("lower combined fixture");
    let pipeline = build_mir_optimization_pipeline(opt_level);
    let optimized = run_mir_pass_pipeline(
        mir,
        &pipeline,
        &MirPassContext {
            opt_level,
            overflow_mode,
            bounds_mode,
            target_backend,
            debug: Default::default(),
        },
    );
    assert_eq!(optimized.validation_errors, []);
    optimized.module
}

fn compile_and_run_c_text(dir: &Path, name: &str, source: &str) -> String {
    let source_path = dir.join(format!("{name}.c"));
    let binary = dir.join(name);
    fs::write(&source_path, source).expect("write C source");
    compile_native(&[&source_path], &binary);
    run_stdout(&binary)
}

fn compile_native(inputs: &[&Path], output: &Path) {
    let compile = Command::new("clang")
        .args(inputs)
        .arg("-std=c11")
        .arg("-Wall")
        .arg("-Wextra")
        .arg("-Werror")
        .arg("-Wno-override-module")
        .arg("-o")
        .arg(output)
        .output()
        .expect("run clang");
    assert!(
        compile.status.success(),
        "clang failed:\n{}",
        String::from_utf8_lossy(&compile.stderr)
    );
}
