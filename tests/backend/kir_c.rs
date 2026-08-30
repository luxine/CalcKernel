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
fn kir_c_cfg_forwarding_should_preserve_swapped_phi_arguments_and_memory_order() {
    let source = r#"
        export fn choose(out: ptr<i32>, flag: bool, a: i32, b: i32) -> i32 {
          let x: i32 = a; let y: i32 = b;
          out[0] = a;
          if flag { x = b; y = a; } else { x = a; y = b; }
          out[1] = x;
          return out[0] + y;
        }
        export fn constant(out: ptr<i32>) -> i32 {
          let x: i32 = 0;
          if (20 + 22) == 42 { out[0] = 7; x = 7; }
          else { out[0] = 99; x = 99; }
          return x;
        }
    "#;
    for optimization_level in 0..=3 {
        for overflow in [KirOverflowMode::Checked, KirOverflowMode::Unchecked] {
            let kir = optimized_kir(
                source,
                level(optimization_level),
                overflow,
                if overflow == KirOverflowMode::Checked {
                    KirBoundsMode::Checked
                } else {
                    KirBoundsMode::Unchecked
                },
            );
            let c = emit_c_kir_module(&kir).expect("CFG C");
            let harness = if overflow == KirOverflowMode::Checked {
                r#"
                int main(void) {
                  int32_t out[2] = {0, 0}; int32_t value = 0;
                  if (choose(out, true, 10, 20, &value) != CK_OK || value != 20 || out[0] != 10 || out[1] != 20) return 1;
                  if (choose(out, false, 10, 20, &value) != CK_OK || value != 30 || out[0] != 10 || out[1] != 10) return 2;
                  if (constant(out, &value) != CK_OK || value != 7 || out[0] != 7) return 3;
                  return 0;
                }
            "#
            } else {
                r#"
                int main(void) {
                  int32_t out[2] = {0, 0};
                  if (choose(out, true, 10, 20) != 20 || out[0] != 10 || out[1] != 20) return 1;
                  if (choose(out, false, 10, 20) != 30 || out[0] != 10 || out[1] != 10) return 2;
                  if (constant(out) != 7 || out[0] != 7) return 3;
                  return 0;
                }
            "#
            };
            compile_and_run(&c, harness);
        }
    }
}

#[test]
fn kir_c_boolean_propagation_should_preserve_loops_and_short_circuit_effects() {
    let source = r#"
        export fn same(flag: bool) -> bool {
          let selected: bool = false;
          if flag { selected = 20 < 22; } else { selected = true; }
          return selected;
        }
        export fn invariant(n: u32) -> bool {
          let i: u32 = 0; let value: bool = true;
          while i < n { value = true; i = i + 1; }
          return value;
        }
        export fn toggle(n: u32) -> bool {
          let i: u32 = 0; let value: bool = false;
          while i < n { value = !value; i = i + 1; }
          return value;
        }
        fn touch(out: ptr<i32>) -> bool { out[0] = 7; return true; }
        export fn short_and(out: ptr<i32>) -> bool { return false && touch(out); }
        export fn short_or(out: ptr<i32>) -> bool { return true || touch(out); }
        export fn maybe_touch(out: ptr<i32>, flag: bool) -> bool { return flag && touch(out); }
    "#;
    for optimization_level in 0..=3 {
        for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
            for bounds in [KirBoundsMode::Unchecked, KirBoundsMode::Checked] {
                let kir = optimized_kir(source, level(optimization_level), overflow, bounds);
                let checked_abi =
                    overflow == KirOverflowMode::Checked || bounds == KirBoundsMode::Checked;
                let harness = if checked_abi {
                    r#"
                    int main(void) {
                      bool result = false; int32_t out = 0;
                      if (same(false, &result) != CK_OK || !result) return 1;
                      if (same(true, &result) != CK_OK || !result) return 2;
                      for (uint32_t n = 0; n < 8; ++n) {
                        if (invariant(n, &result) != CK_OK || !result) return 3;
                        if (toggle(n, &result) != CK_OK || result != (n % 2 != 0)) return 4;
                      }
                      if (short_and(&out, &result) != CK_OK || result || out != 0) return 5;
                      if (short_or(&out, &result) != CK_OK || !result || out != 0) return 6;
                      if (maybe_touch(&out, false, &result) != CK_OK || result || out != 0) return 7;
                      if (maybe_touch(&out, true, &result) != CK_OK || !result || out != 7) return 8;
                      return 0;
                    }
                    "#
                } else {
                    r#"
                    int main(void) {
                      int32_t out = 0;
                      if (!same(false) || !same(true)) return 1;
                      for (uint32_t n = 0; n < 8; ++n) {
                        if (!invariant(n)) return 2;
                        if (toggle(n) != (n % 2 != 0)) return 3;
                      }
                      if (short_and(&out) || out != 0) return 4;
                      if (!short_or(&out) || out != 0) return 5;
                      if (maybe_touch(&out, false) || out != 0) return 6;
                      if (!maybe_touch(&out, true) || out != 7) return 7;
                      return 0;
                    }
                    "#
                };
                compile_and_run(
                    &emit_c_kir_module(&kir).expect("boolean propagation C"),
                    harness,
                );
            }
        }
    }
}

#[test]
fn kir_c_checked_propagation_should_preserve_first_failure_and_prior_writes() {
    let source = r#"
        export fn ordered(out: ptr<i32>, initial: u32, denominator: u32) -> bool {
          out[0] = 1;
          let x: u32 = initial + 1;
          out[0] = 2;
          let y: u32 = 42 % denominator;
          out[0] = 3;
          return (x == 1) && (y == 0);
        }
        export fn zero(out: ptr<i32>) -> bool {
          out[0] = 4;
          let y: u32 = 42 % 0;
          out[0] = 5;
          return y == 0;
        }
        export fn unreachable_failure() -> bool { return false && (42 % 0 == 0); }
        export fn bounded(n: u32) -> bool {
          if n < 8 { return (n + 1) < 9; }
          return (n + 1) < 9;
        }
    "#;
    for optimization_level in 0..=3 {
        let kir = optimized_kir(
            source,
            level(optimization_level),
            KirOverflowMode::Checked,
            KirBoundsMode::Checked,
        );
        compile_and_run(
            &emit_c_kir_module(&kir).expect("checked propagation C"),
            r#"
            int main(void) {
              int32_t out = 0; bool result = false;
              if (ordered(&out, UINT32_MAX, 0, &result) != CK_ERR_OVERFLOW || result || out != 1) return 1;
              if (ordered(&out, 0, 0, &result) != CK_ERR_DIV_BY_ZERO || result || out != 2) return 2;
              if (ordered(&out, 0, 7, &result) != CK_OK || !result || out != 3) return 3;
              result = false;
              if (zero(&out, &result) != CK_ERR_DIV_BY_ZERO || result || out != 4) return 4;
              if (unreachable_failure(&result) != CK_OK || result) return 5;
              for (uint32_t n = 0; n < 16; ++n) {
                if (bounded(n, &result) != CK_OK || result != (n < 8)) return 6;
              }
              result = true;
              if (bounded(UINT32_MAX, &result) != CK_ERR_OVERFLOW || !result) return 7;
              return 0;
            }
        "#,
        );
    }
}

#[test]
fn kir_c_constant_propagation_should_preserve_results_and_checked_wrap_at_every_level() {
    let source = r#"
        export fn same(flag: bool) -> i32 {
          let x: i32 = 0;
          if flag { x = 42; } else { x = 42; }
          return x + 1;
        }
        export fn different(flag: bool) -> i32 {
          let x: i32 = 0;
          if flag { x = 42; } else { x = 41; }
          return x + 1;
        }
        export fn compared() -> bool { return (20 + 22) >= 42; }
        export fn wrapped() -> u32 { return 4294967295 + 1; }
    "#;
    for optimization_level in 0..=3 {
        for checked in [false, true] {
            let kir = optimized_kir(
                source,
                level(optimization_level),
                if checked {
                    KirOverflowMode::Checked
                } else {
                    KirOverflowMode::Unchecked
                },
                KirBoundsMode::Unchecked,
            );
            let c = emit_c_kir_module(&kir).expect("constant propagation C");
            let harness = if checked {
                r#"
                int main(void) {
                  int32_t value = 0;
                  uint32_t wrapped_value = 7;
                  bool comparison = false;
                  if (same(false, &value) != CK_OK || value != 43) return 1;
                  if (same(true, &value) != CK_OK || value != 43) return 2;
                  if (different(false, &value) != CK_OK || value != 42) return 3;
                  if (different(true, &value) != CK_OK || value != 43) return 4;
                  if (compared(&comparison) != CK_OK || !comparison) return 5;
                  if (wrapped(&wrapped_value) != CK_ERR_OVERFLOW || wrapped_value != 7) return 6;
                  return 0;
                }
                "#
            } else {
                r#"
                int main(void) {
                  if (same(false) != 43 || same(true) != 43) return 1;
                  if (different(false) != 42 || different(true) != 43) return 2;
                  if (!compared()) return 3;
                  if (wrapped() != 0) return 4;
                  return 0;
                }
                "#
            };
            compile_and_run(&c, harness);
        }
    }
}

#[test]
fn kir_c_range_proofs_should_preserve_boundaries_and_failure_result_slots() {
    let source = r#"
        export fn bounded(n: u32) -> u32 {
          if n < 8 { return n + 8; }
          return n + 8;
        }
        export fn divide(n: u32) -> u32 {
          if n > 0 { return 40 / n; }
          return 40 / n;
        }
        export unsafe fn positive(a: i32, n: i32) -> i32
        contract { requires n > 0; }
        { return a / n; }
        export unsafe fn negative(a: i32, n: i32) -> i32
        contract { requires n < 0; }
        { return a / n; }
        export fn get(data: ptr<i32>, n: u32) -> i32 {
          if n < 8 { let items: slice<i32> = slice(data, 8); return items[n]; }
          let items: slice<i32> = slice(data, 8); return items[n];
        }
    "#;
    for optimization_level in 0..=3 {
        let (kir, contracts) = optimized_kir_with_contracts(
            source,
            level(optimization_level),
            KirOverflowMode::Checked,
            KirBoundsMode::Checked,
        );
        let c = emit_c_kir_module_with_contracts(&kir, contracts.as_ref()).expect("range C");
        compile_and_run(
            &c,
            r#"
            int main(void) {
              uint32_t u = 99;
              int32_t i = 99;
              int32_t data[8] = {0, 1, 2, 3, 4, 5, 6, 7};
              for (uint32_t n = 0; n < 32; ++n) {
                if (bounded(n, &u) != CK_OK || u != n + 8) return 1;
              }
              u = 99;
              if (bounded(UINT32_MAX, &u) != CK_ERR_OVERFLOW || u != 99) return 2;
              for (uint32_t n = 1; n < 32; ++n) {
                if (divide(n, &u) != CK_OK || u != 40 / n) return 3;
              }
              u = 99;
              if (divide(0, &u) != CK_ERR_DIV_BY_ZERO || u != 99) return 4;
              if (positive(INT32_MIN, 1, &i) != CK_OK || i != INT32_MIN) return 5;
              if (negative(INT32_MIN, -2, &i) != CK_OK || i != INT32_MIN / -2) return 6;
              i = 99;
              if (negative(INT32_MIN, -1, &i) != CK_ERR_OVERFLOW || i != 99) return 7;
              for (uint32_t n = 0; n < 8; ++n) {
                if (get(data, n, &i) != CK_OK || i != (int32_t)n) return 8;
              }
              i = 99;
              if (get(data, 8, &i) != CK_ERR_OUT_OF_BOUNDS || i != 99) return 9;
              return 0;
            }
        "#,
        );
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

#[test]
fn kir_c_loop_guard_rules_should_preserve_integer_limits_and_first_failure() {
    let source = r#"
        export fn up_i32(start: i32, stop: i32) -> i32 {
          let i: i32 = start; while i < stop { i = i + 1; } return i;
        }
        export fn up_u32(start: u32, stop: u32) -> u32 {
          let i: u32 = start; while stop > i { i = 1 + i; } return i;
        }
        export fn up_i64(start: i64, stop: i64) -> i64 {
          let i: i64 = start; while i < stop { i = i + 1; } return i;
        }
        export fn up_u64(start: u64, stop: u64) -> u64 {
          let i: u64 = start; while i < stop { i = i + 1; } return i;
        }
        export fn overshoot(out: ptr<u32>, stop: u32) -> u32 {
          let i: u32 = 4294967294;
          while i < stop { out[0] = 7; i = i + 2; out[0] = 9; break; }
          return i;
        }
        export fn shifted(items: slice<i32>, start: u32) -> i32 {
          let i: u32 = start;
          while i < items.len { i = i + 1; return items[i]; }
          return 7;
        }
    "#;
    for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
        for bounds in [KirBoundsMode::Unchecked, KirBoundsMode::Checked] {
            let checked_abi =
                overflow == KirOverflowMode::Checked || bounds == KirBoundsMode::Checked;
            let mut harness = String::from("int main(void) {\n");
            for (function, ty, limit) in [
                ("up_i32", "int32_t", "INT32_MAX"),
                ("up_u32", "uint32_t", "UINT32_MAX"),
                ("up_i64", "int64_t", "INT64_MAX"),
                ("up_u64", "uint64_t", "UINT64_MAX"),
            ] {
                if checked_abi {
                    harness.push_str(&format!("  {{ {ty} result = 0; if ({function}({limit} - 1, {limit}, &result) != CK_OK || result != {limit}) return 1; if ({function}({limit}, {limit}, &result) != CK_OK || result != {limit}) return 2; if ({function}(3, 0, &result) != CK_OK || result != 3) return 3; }}\n"));
                } else {
                    harness.push_str(&format!("  if ({function}({limit} - 1, {limit}) != {limit} || {function}({limit}, {limit}) != {limit} || {function}(3, 0) != 3) return 4;\n"));
                }
            }
            harness.push_str("  uint32_t out = 0; int32_t data[2] = {11, 42};\n");
            if checked_abi {
                harness.push_str("  uint32_t result = 99; int32_t loaded = 99;\n");
                if overflow == KirOverflowMode::Checked {
                    harness.push_str("  if (overshoot(&out, UINT32_MAX, &result) != CK_ERR_OVERFLOW || out != 7 || result != 99) return 5;\n");
                } else {
                    harness.push_str("  if (overshoot(&out, UINT32_MAX, &result) != CK_OK || out != 9 || result != 0) return 6;\n");
                }
                if bounds == KirBoundsMode::Checked {
                    harness.push_str("  if (shifted(data, 1, 0, &loaded) != CK_ERR_OUT_OF_BOUNDS || loaded != 99) return 7;\n");
                }
                harness.push_str(
                    "  if (shifted(data, 2, 0, &loaded) != CK_OK || loaded != 42) return 8;\n",
                );
            } else {
                harness.push_str("  if (overshoot(&out, UINT32_MAX) != 0 || out != 9) return 9; if (shifted(data, 2, 0) != 42) return 10;\n");
            }
            harness.push_str("  return 0;\n}\n");
            for optimization in 0..=3 {
                let kir = optimized_kir(source, level(optimization), overflow, bounds);
                compile_and_run(&emit_c_kir_module(&kir).expect("loop guard C"), &harness);
            }
        }
    }
}

#[test]
fn kir_c_induction_simplification_should_preserve_wrap_break_and_first_error() {
    let source = r#"
        export fn counters(start: u32, stop: u32) -> u32 {
          let i: u32 = start; let j: u32 = start;
          while i < stop { i = i + 1; j = j + 1; }
          return j;
        }
        export fn descend(start: i64, stop: i64) -> i64 {
          let i: i64 = start; let j: i64 = start;
          while i > stop { i = i - 1; j = j - 1; }
          return j;
        }
        export fn mid_break(start: u32, stop: u32, choose: bool) -> u32 {
          let i: u32 = start; let j: u32 = start;
          while i < stop { i = i + 1; if choose { break; } j = j + 1; }
          return j;
        }
        export fn ordered(out: ptr<u32>, start: u32, stop: u32) -> u32 {
          let i: u32 = start; let j: u32 = start;
          while i < stop {
            out[0] = 1; i = i + 2; out[0] = 2; j = j + 2; out[0] = 3;
            if i == 0 { break; }
          }
          return j;
        }
        export fn different(n: u32) -> u32 {
          let i: u32 = 0; let j: u32 = 1;
          while i < n { i = i + 1; j = j + 1; }
          return j;
        }
    "#;
    for overflow in [KirOverflowMode::Unchecked, KirOverflowMode::Checked] {
        for bounds in [KirBoundsMode::Unchecked, KirBoundsMode::Checked] {
            let checked_abi =
                overflow == KirOverflowMode::Checked || bounds == KirBoundsMode::Checked;
            let mut harness = String::from("int main(void) { uint32_t out = 0;\n");
            if checked_abi {
                harness.push_str("  uint32_t result = 99; int64_t signed_result = 99;\n");
                harness.push_str("  if (counters(UINT32_MAX - 1, UINT32_MAX, &result) != CK_OK || result != UINT32_MAX) return 1; if (counters(7, 0, &result) != CK_OK || result != 7) return 2;\n");
                harness.push_str("  if (descend(INT64_MIN + 1, INT64_MIN, &signed_result) != CK_OK || signed_result != INT64_MIN) return 3;\n");
                harness.push_str("  if (mid_break(3, 5, true, &result) != CK_OK || result != 3) return 4; if (mid_break(3, 5, false, &result) != CK_OK || result != 5) return 5;\n");
                harness.push_str(
                    "  if (different(3, &result) != CK_OK || result != 4) return 6; result = 99;\n",
                );
                if overflow == KirOverflowMode::Checked {
                    harness.push_str("  if (ordered(&out, UINT32_MAX - 1, UINT32_MAX, &result) != CK_ERR_OVERFLOW || out != 1 || result != 99) return 7;\n");
                } else {
                    harness.push_str("  if (ordered(&out, UINT32_MAX - 1, UINT32_MAX, &result) != CK_OK || out != 3 || result != 0) return 8;\n");
                }
                harness.push_str("  if (ordered(&out, 0, 2, &result) != CK_OK || out != 3 || result != 2) return 9;\n");
            } else {
                harness.push_str("  if (counters(UINT32_MAX - 1, UINT32_MAX) != UINT32_MAX || counters(7, 0) != 7) return 1;\n");
                harness
                    .push_str("  if (descend(INT64_MIN + 1, INT64_MIN) != INT64_MIN) return 2;\n");
                harness.push_str("  if (mid_break(3, 5, true) != 3 || mid_break(3, 5, false) != 5 || different(3) != 4) return 3;\n");
                harness.push_str("  if (ordered(&out, UINT32_MAX - 1, UINT32_MAX) != 0 || out != 3 || ordered(&out, 0, 2) != 2 || out != 3) return 4;\n");
            }
            harness.push_str("  return 0;\n}\n");
            for optimization in 0..=3 {
                let kir = optimized_kir(source, level(optimization), overflow, bounds);
                compile_and_run(&emit_c_kir_module(&kir).expect("induction C"), &harness);
            }
        }
    }
}
