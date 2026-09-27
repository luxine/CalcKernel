#![cfg(feature = "native-toolchain")]

use std::{fs, process::Command};

use calckernel::{
    BoundsMode, EmitLlvmOptions, KirConsumer, NativeContext, NativeOptimizationLevel, NativeTarget,
    OverflowMode, lower_native_kir_module,
};

use super::support::{
    command::clang_available, compiler::optimized_module, fixtures, temp::unique_id,
};

fn structural_llvm(source: &str, opt_level: u8) -> String {
    let kir = optimized_module(
        source,
        opt_level,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("host target");
    lower_native_kir_module(
        &context,
        &target,
        &kir,
        &EmitLlvmOptions {
            source_file_name: Some("test.ck".to_string()),
            target_triple: None,
        },
    )
    .expect("structural lowering")
    .verify()
    .expect("initial verification")
    .audit()
    .expect("pre-optimization fact audit")
    .optimize(
        &target,
        NativeOptimizationLevel::try_from(opt_level).expect("valid test level"),
    )
    .expect("PassBuilder and second verification")
    .to_ir_string()
    .expect("canonical LLVM print")
}

#[test]
fn llvm_structural_backend_should_verify_all_representative_fixtures_at_o0_through_o3() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for fixture in fixtures::ORACLE_EXAMPLES
        .iter()
        .chain(fixtures::BENCHMARK_FIXTURES)
        .map(|fixture| fixture.local)
        .chain(std::iter::once(fixtures::F64_EDGES.local))
    {
        let source = fs::read_to_string(root.join(fixture)).expect(fixture);
        for level in 0..=3 {
            let text = structural_llvm(&source, level);
            assert!(text.contains("target datalayout"), "{fixture} O{level}");
            assert!(text.contains("target triple"), "{fixture} O{level}");
        }
    }
}

#[test]
fn llvm_structural_backend_should_run_scalar_control_and_memory_oracle() {
    if !clang_available() {
        return;
    }
    let ir = structural_llvm(
        r#"
          struct Item { price: i64; qty: i64; }
          export fn add_i64(a: i64, b: i64) -> i64 { return a + b; }
          export fn sum_to_n(n: i64) -> i64 {
            let i: i64 = 0; let sum: i64 = 0;
            while i < n { sum = sum + i; i = i + 1; }
            return sum;
          }
          export fn calc(items: ptr<Item>, out: ptr<i64>) -> void {
            out[0] = items[0].price * items[0].qty;
          }
        "#,
        0,
    );
    let harness = r#"
      #include <stdint.h>
      typedef struct Item { int64_t price; int64_t qty; } Item;
      int64_t add_i64(int64_t, int64_t);
      int64_t sum_to_n(int64_t);
      void calc(Item*, int64_t*);
      int main(void) {
        Item item = {7, 6}; int64_t out = 0;
        calc(&item, &out);
        return add_i64(2, 3) == 5 && sum_to_n(5) == 10 && out == 42 ? 0 : 1;
      }
    "#;
    let dir = std::env::temp_dir().join(format!("ckc_structural_llvm_{}", unique_id()));
    fs::create_dir_all(&dir).expect("test dir");
    let ir_path = dir.join("module.ll");
    let harness_path = dir.join("harness.c");
    let binary = dir.join("harness");
    fs::write(&ir_path, ir).expect("IR");
    fs::write(&harness_path, harness).expect("harness");
    let compile = Command::new("clang")
        .args(["-Wno-override-module", "-O3"])
        .arg(&ir_path)
        .arg(&harness_path)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("Clang oracle");
    assert!(
        compile.status.success(),
        "{}",
        String::from_utf8_lossy(&compile.stderr)
    );
    assert!(Command::new(binary).status().expect("run oracle").success());
}

#[test]
fn llvm_structural_backend_should_cover_void_slice_index_and_subslice() {
    let text = structural_llvm(
        r#"
          struct Item { value: i32; }
          fn cut(items: slice<Item>, start: u32, end: u32) -> slice<Item> {
            items[start].value = items[start].value + 1;
            return items[start..end];
          }
          export fn touch(items: slice<Item>) -> void { let part: slice<Item> = cut(items, 0, items.len); }
        "#,
        0,
    );
    for needle in [
        "define internal { ptr, i32 } @cut",
        "define void @touch",
        "extractvalue { ptr, i32 }",
        "insertvalue { ptr, i32 }",
        "getelementptr %struct.Item",
        "ret void",
    ] {
        assert!(text.contains(needle), "missing {needle}:\n{text}");
    }
}

#[test]
fn llvm_structural_backend_should_reject_non_host_target_before_construction() {
    let kir = optimized_module(
        "export fn answer() -> i32 { return 42; }",
        0,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host().expect("target");
    let error = lower_native_kir_module(
        &context,
        &target,
        &kir,
        &EmitLlvmOptions {
            source_file_name: None,
            target_triple: Some("wasm32-unknown-unknown".to_string()),
        },
    )
    .expect_err("non-host target must fail");
    assert!(
        error.message.contains("does not match native target"),
        "{error}"
    );
}
