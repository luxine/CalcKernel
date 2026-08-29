use std::{fs, process::Command};

use calckernel::{
    ContractFactSet, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel,
    KirOverflowMode, KirSanitizerMode, SourceFile, build_kir_module, check, emit_c_kir_header,
    emit_c_kir_module, emit_c_kir_module_with_contracts, import_contract_facts, lower_to_mir,
    run_kir_pass_pipeline,
};

use crate::generated::fixed_seed_kernel_program;
use crate::support::temp::temp_dir;

fn optimized_kir(
    source: &str,
    level: KirOptimizationLevel,
    overflow: KirOverflowMode,
    bounds: KirBoundsMode,
) -> calckernel::KirModule {
    optimized_kir_with_contracts(source, level, overflow, bounds).0
}

fn optimized_kir_with_contracts(
    source: &str,
    level: KirOptimizationLevel,
    overflow: KirOverflowMode,
    bounds: KirBoundsMode,
) -> (calckernel::KirModule, Option<ContractFactSet>) {
    let checked = check(&SourceFile::new("kir-c.ck", source));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::C,
            overflow_mode: overflow,
            bounds_mode: bounds,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let contracts = checked
        .checked_program
        .functions
        .iter()
        .any(|function| function.is_unsafe)
        .then(|| import_contract_facts(&kir, &checked.checked_program, 0).expect("contract facts"));
    let result = run_kir_pass_pipeline(kir, level, contracts.as_ref());
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    (
        result.artifact.expect("verified artifact"),
        result.contract_facts,
    )
}

fn compile_and_run(c: &str, harness: &str) {
    let temp = temp_dir("kir_c_backend");
    fs::create_dir_all(&temp).expect("create temp dir");
    let source = temp.join("case.c");
    let binary = temp.join("case");
    fs::write(&source, format!("{c}\n{harness}")).expect("write C");
    let output = Command::new("clang")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .expect("clang");
    assert!(
        output.status.success(),
        "clang failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let output = Command::new(binary).output().expect("run C");
    assert!(output.status.success(), "C harness exit: {output:?}");
    fs::remove_dir_all(temp).expect("remove temp dir");
}

const DIFFERENTIAL_SOURCE: &str = r#"
    struct Pair { x: i32; y: i32; }
    export fn scalar(a: i32, b: i32) -> i32 { return a * 3 + b; }
    export fn control(n: i32) -> i32 {
      let i: i32 = 0; let total: i32 = 0;
      while i < n { total = total + i; i = i + 1; }
      return total;
    }
    export fn write(out: ptr<i32>, value: i32) -> void { out[0] = value; }
    export fn slice_total(items: slice<i32>) -> i32 {
      let middle: slice<i32> = items[1..3];
      return middle[0] + middle[1];
    }
    export fn pair_total(pair: ptr<Pair>) -> i32 { return pair[0].x + pair[0].y; }
"#;

fn level(level: u8) -> KirOptimizationLevel {
    match level {
        0 => KirOptimizationLevel::O0,
        1 => KirOptimizationLevel::O1,
        2 => KirOptimizationLevel::O2,
        3 => KirOptimizationLevel::O3,
        _ => unreachable!("test optimization level"),
    }
}

#[test]
fn kir_c_unchecked_backend_should_compile_scalar_control_struct_and_memory() {
    let kir = optimized_kir(
        r#"
        struct Item { price: i64; qty: i64; }
        export fn sum(n: i64) -> i64 {
          let i: i64 = 0; let total: i64 = 0;
          while i < n { total = total + i; i = i + 1; }
          return total;
        }
        export fn calc(items: ptr<Item>) -> i64 { return items[0].price * items[0].qty; }
        "#,
        KirOptimizationLevel::O3,
        KirOverflowMode::Unchecked,
        KirBoundsMode::Unchecked,
    );
    let c = emit_c_kir_module(&kir).expect("KIR C");
    compile_and_run(
        &c,
        r#"
        int main(void) {
          Item item[1] = {{7, 6}};
          if (sum(5) != 10) return 1;
          if (calc(item) != 42) return 2;
          return 0;
        }
        "#,
    );
}

#[test]
fn kir_c_checked_backend_should_lower_only_explicit_guards_and_status_abi() {
    let checked = optimized_kir(
        "export fn add(a: i32, b: i32) -> i32 { return a + b; }",
        KirOptimizationLevel::O0,
        KirOverflowMode::Checked,
        KirBoundsMode::Checked,
    );
    let c = emit_c_kir_module(&checked).expect("checked KIR C");
    let header = emit_c_kir_header(&checked);
    assert!(header.contains("CK_Status add(int32_t a, int32_t b, int32_t* ck_return);"));
    assert_eq!(c.matches("return CK_ERR_OVERFLOW").count(), 1);
    compile_and_run(
        &c,
        r#"
        int main(void) {
          int32_t result = 0;
          if (add(20, 22, &result) != CK_OK || result != 42) return 1;
          if (add(INT32_MAX, 1, &result) != CK_ERR_OVERFLOW) return 2;
          return 0;
        }
        "#,
    );

    let proven = optimized_kir(
        "export fn answer() -> i32 { return 20 + 22; }",
        KirOptimizationLevel::O3,
        KirOverflowMode::Checked,
        KirBoundsMode::Checked,
    );
    let proven_c = emit_c_kir_module(&proven).expect("proven KIR C");
    assert_eq!(proven_c.matches("return CK_ERR_OVERFLOW").count(), 0);
    assert!(!proven_c.contains("__builtin_add_overflow"));
}

#[test]
fn kir_c_pairwise_noalias_third_root_should_not_emit_restrict() {
    let (kir, contracts) = optimized_kir_with_contracts(
        r#"
        export unsafe fn f(a: slice<i32>, b: slice<i32>, c: slice<i32>) -> i32
        contract { requires noalias(a, b); effects read(a), read(b), read(c); }
        { return a[0] + b[0] + c[0]; }
        "#,
        KirOptimizationLevel::O3,
        KirOverflowMode::Checked,
        KirBoundsMode::Checked,
    );
    let c = emit_c_kir_module_with_contracts(&kir, contracts.as_ref()).expect("KIR C");
    assert!(!c.contains("* CKC_RESTRICT"));
}

#[test]
fn kir_c_complete_noalias_and_alignment_facts_should_emit_portable_hints() {
    let (kir, contracts) = optimized_kir_with_contracts(
        r#"
        export unsafe fn add(a: slice<i32>, b: slice<i32>) -> i32
        contract {
          requires noalias(a, b);
          requires aligned(a.data, 16);
          effects read(a), read(b);
        }
        { return a[0] + b[0]; }
        "#,
        KirOptimizationLevel::O3,
        KirOverflowMode::Checked,
        KirBoundsMode::Checked,
    );
    let c = emit_c_kir_module_with_contracts(&kir, contracts.as_ref()).expect("fact-aware C");
    assert_eq!(c.matches("* CKC_RESTRICT").count(), 4, "{c}");
    assert!(c.contains("CKC_ASSUME_ALIGNED(a_data, 16)"), "{c}");
    compile_and_run(
        &c,
        r#"
        int main(void) {
          _Alignas(16) int32_t a[1] = {19};
          _Alignas(16) int32_t b[1] = {23};
          int32_t result = 0;
          if (add(a, 1, b, 1, &result) != CK_OK || result != 42) return 1;
          return 0;
        }
        "#,
    );
}

#[test]
fn kir_c_layout_should_order_struct_slices_and_disambiguate_generated_names() {
    let kir = optimized_kir(
        r#"
        struct Item { value: i32; }
        struct CK_Slice_Item { marker: i32; }
        export fn collide(
          items_data: i32,
          items: slice<Item>,
          ck_return: i32,
          ck_v0: i32
        ) -> i32 {
          return items_data + items[0].value + ck_return + ck_v0;
        }
        "#,
        KirOptimizationLevel::O0,
        KirOverflowMode::Checked,
        KirBoundsMode::Checked,
    );
    let c = emit_c_kir_module(&kir).expect("collision-safe KIR C");
    assert!(c.contains("typedef struct Item Item;"), "{c}");
    assert!(c.contains("typedef struct CK_Slice_Item_1"), "{c}");
    assert!(c.contains("Item* items_data_1"), "{c}");
    assert!(c.contains("int32_t* ck_return_1"), "{c}");
    compile_and_run(
        &c,
        r#"
        int main(void) {
          Item item[1] = {{36}};
          int32_t result = 0;
          if (collide(1, item, 1, 2, 3, &result) != CK_OK || result != 42) return 1;
          return 0;
        }
        "#,
    );
}

#[test]
fn kir_c_o0_through_o3_should_cover_supported_mode_matrix() {
    let unchecked_harness = r#"
        int main(void) {
          int32_t out = 0;
          int32_t items[4] = {10, 20, 22, 99};
          Pair pair[1] = {{19, 23}};
          if (scalar(10, 12) != 42) return 1;
          if (control(10) != 45) return 2;
          write(&out, 42); if (out != 42) return 3;
          if (slice_total(items, 4) != 42) return 4;
          if (pair_total(pair) != 42) return 5;
          return 0;
        }
    "#;
    let checked_harness = r#"
        int main(void) {
          int32_t out = 0; int32_t result = 0;
          int32_t items[4] = {10, 20, 22, 99};
          Pair pair[1] = {{19, 23}};
          if (scalar(10, 12, &result) != CK_OK || result != 42) return 1;
          if (control(10, &result) != CK_OK || result != 45) return 2;
          if (write(&out, 42) != CK_OK || out != 42) return 3;
          if (slice_total(items, 4, &result) != CK_OK || result != 42) return 4;
          if (pair_total(pair, &result) != CK_OK || result != 42) return 5;
          if (slice_total(items, 2, &result) != CK_ERR_OUT_OF_BOUNDS) return 6;
          return 0;
        }
    "#;

    for opt_level in 0..=3 {
        let unchecked = optimized_kir(
            DIFFERENTIAL_SOURCE,
            level(opt_level),
            KirOverflowMode::Unchecked,
            KirBoundsMode::Unchecked,
        );
        compile_and_run(
            &emit_c_kir_module(&unchecked).expect("unchecked KIR C"),
            unchecked_harness,
        );
        let checked = optimized_kir(
            DIFFERENTIAL_SOURCE,
            level(opt_level),
            KirOverflowMode::Checked,
            KirBoundsMode::Checked,
        );
        compile_and_run(
            &emit_c_kir_module(&checked).expect("checked KIR C"),
            checked_harness,
        );
    }
}

#[test]
fn generated_c_kernels_should_match_o0_at_o1_through_o3_in_every_supported_mode() {
    let generated = fixed_seed_kernel_program();

    for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
        for bounds in [KirBoundsMode::Unchecked, KirBoundsMode::Checked] {
            let checked_abi =
                overflow == KirOverflowMode::Checked || bounds == KirBoundsMode::Checked;
            let mut harness = String::from("int main(void) {\n");
            for (index, case) in generated.cases.iter().enumerate() {
                let values = case
                    .values
                    .iter()
                    .map(i32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                harness.push_str(&format!("  int32_t values_{index}[8] = {{{values}}};\n"));
                if checked_abi {
                    harness.push_str(&format!(
                        "  int32_t result_{index} = 0; if ({}(values_{index}, 8, {}, {}, &result_{index}) != CK_OK || result_{index} != {}) return {};\n",
                        case.function,
                        case.len,
                        case.bias,
                        case.expected,
                        index + 1,
                    ));
                } else {
                    harness.push_str(&format!(
                        "  if ({}(values_{index}, 8, {}, {}) != {}) return {};\n",
                        case.function,
                        case.len,
                        case.bias,
                        case.expected,
                        index + 1,
                    ));
                }
            }
            harness.push_str("  return 0;\n}\n");

            for level in [
                KirOptimizationLevel::O0,
                KirOptimizationLevel::O1,
                KirOptimizationLevel::O2,
                KirOptimizationLevel::O3,
            ] {
                let (kir, contracts) =
                    optimized_kir_with_contracts(&generated.source, level, overflow, bounds);
                let c = emit_c_kir_module_with_contracts(&kir, contracts.as_ref())
                    .expect("generated KIR C");
                compile_and_run(&c, &harness);
            }
        }
    }
}

#[test]
fn kir_c_canonical_checked_loop_should_preserve_kir_guard_elimination() {
    let source = r#"
        export unsafe fn sum(items: slice<i32>, len: u32) -> i32
        contract { requires len <= items.len; effects read(items); }
        {
          let i: u32 = 0; let total: i32 = 0;
          while i < len { total = total + items[i]; i = i + 1; }
          return total;
        }
    "#;
    for (level, expected_bounds_returns) in [
        (KirOptimizationLevel::O1, 1),
        (KirOptimizationLevel::O2, 0),
        (KirOptimizationLevel::O3, 0),
    ] {
        let kir = optimized_kir(
            source,
            level,
            KirOverflowMode::Checked,
            KirBoundsMode::Checked,
        );
        let c = emit_c_kir_module(&kir).expect("canonical checked loop C");
        assert_eq!(
            c.matches("return CK_ERR_OUT_OF_BOUNDS").count(),
            expected_bounds_returns,
            "{level:?}:\n{c}"
        );
    }
}
