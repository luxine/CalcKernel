use super::support::compiler::optimized_module;
use calckernel::{
    BoundsMode, EmitLlvmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirFailureKind,
    KirInstructionKind, KirOptimizationLevel, KirOverflowMode, KirSanitizerMode, NativeContext,
    NativeOptimizationLevel, NativeStage, NativeTarget, OverflowMode, SourceFile, build_kir_module,
    check, lower_native_kir_module, lower_to_mir, run_kir_pass_pipeline,
    test_invalid_module_verification,
};

fn structural_llvm(source_text: &str) -> String {
    let consumer = if source_text.contains("fn main()") {
        KirConsumer::NativeExecutable
    } else {
        KirConsumer::NativeLibrary
    };
    let kir = optimized_module(
        source_text,
        0,
        consumer,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("host target");
    let module = lower_native_kir_module(
        &context,
        &target,
        &kir,
        &EmitLlvmOptions {
            source_file_name: Some("fixture.ck".to_string()),
            target_triple: None,
        },
    )
    .expect("structural LLVM lowering");
    module
        .verify()
        .expect("verify structural module")
        .to_ir_string()
        .expect("LLVM prints module")
}

fn checked_llvm(source_text: &str, overflow: OverflowMode, bounds: BoundsMode) -> String {
    let kir = optimized_module(source_text, 0, KirConsumer::NativeLibrary, overflow, bounds);
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("host target");
    lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("checked structural LLVM lowering")
        .verify()
        .expect("verify checked structural module")
        .to_ir_string()
        .expect("print checked structural module")
}

fn optimized_llvm(source_text: &str, level: NativeOptimizationLevel) -> String {
    let opt_level = match level {
        NativeOptimizationLevel::O0 => 0,
        NativeOptimizationLevel::O1 => 1,
        NativeOptimizationLevel::O2 => 2,
        NativeOptimizationLevel::O3 => 3,
    };
    let kir = optimized_module(
        source_text,
        opt_level,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("host target");
    lower_native_kir_module(&context, &target, &kir, &EmitLlvmOptions::default())
        .expect("structural LLVM lowering")
        .verify()
        .expect("initial verification")
        .audit()
        .expect("pre-optimization fact audit")
        .optimize(&target, level)
        .expect("PassBuilder and second verification")
        .to_ir_string()
        .expect("print optimized module")
}

#[test]
fn structural_llvm_should_use_host_target_triple_and_data_layout() {
    let target = NativeTarget::host().expect("host target");
    let triple = target.triple().expect("target triple");
    let data_layout = target.data_layout().expect("target data layout");
    assert!(!triple.is_empty());
    assert!(!data_layout.is_empty());

    let text = structural_llvm("export fn answer() -> i32 { return 42; }");
    assert!(
        text.contains(&format!("target triple = \"{triple}\"")),
        "{text}"
    );
    assert!(
        text.contains(&format!("target datalayout = \"{data_layout}\"")),
        "{text}"
    );
    assert!(text.contains("source_filename = \"fixture.ck\""), "{text}");
}

#[test]
fn structural_llvm_should_lower_constants_arithmetic_comparisons_calls_and_void() {
    let text = structural_llvm(
        r#"
      fn touch(out: ptr<i32>) -> void { out[0] = 7; }
      export fn calc(a: i32, b: i32, out: ptr<i32>) -> bool {
        touch(out);
        let value: i32 = a * b + 3;
        return value >= 10;
      }
    "#,
    );
    for needle in [
        "define internal void @touch",
        "@__ck_impl_calc(i32",
        "@calc(i32",
        "call void @touch",
        "mul i32",
        "add i32",
        "icmp sge i32",
        "ret void",
    ] {
        assert!(text.contains(needle), "missing {needle}:\n{text}");
    }
}

#[test]
fn structural_llvm_should_lower_branches_loops_and_short_circuit_blocks() {
    let text = structural_llvm(
        r#"
      export fn control(n: i32, enabled: bool) -> i32 {
        let i: i32 = 0;
        while i < n && enabled {
          if i == 3 { break; }
          i = i + 1;
        }
        return i;
      }
    "#,
    );
    assert!(text.matches("br i1").count() >= 2, "{text}");
    assert!(text.contains("br label"), "{text}");
    assert!(text.contains("bb1"), "{text}");
}

#[test]
fn structural_llvm_should_lower_struct_pointer_slice_index_and_subslice() {
    let text = structural_llvm(
        r#"
      struct Holder { values: slice<i64>; }
      fn cut(holder: ptr<Holder>, start: u32, end: u32) -> slice<i64> {
        let values: slice<i64> = holder[0].values;
        let first: i64 = values[start];
        holder[0].values = values[start..end];
        return holder[0].values;
      }
      export fn use_cut(holder: ptr<Holder>, start: u32, end: u32) -> i64 {
        let part: slice<i64> = cut(holder, start, end);
        return part[0];
      }
    "#,
    );
    assert!(
        text.contains("%struct.Holder = type { { ptr, i32 } }"),
        "{text}"
    );
    assert!(text.contains("getelementptr %struct.Holder"), "{text}");
    assert!(text.contains("extractvalue { ptr, i32 }"), "{text}");
    assert!(text.contains("insertvalue { ptr, i32 }"), "{text}");
    assert!(text.contains("getelementptr i64"), "{text}");
}

#[test]
fn structural_llvm_should_declare_and_call_typed_print_runtime_intrinsics() {
    let text = structural_llvm("fn main() -> void { print_i32(7); print_newline(); }");
    assert!(text.contains("declare void @__ck_print_i32(i32)"), "{text}");
    assert!(
        text.contains("declare void @__ck_print_newline()"),
        "{text}"
    );
    assert!(text.contains("call void @__ck_print_i32(i32"), "{text}");
    assert!(text.contains("call void @__ck_print_newline()"), "{text}");
}

#[test]
fn verifier_should_reject_an_unterminated_test_block_with_module_stage() {
    let error = test_invalid_module_verification();
    assert_eq!(error.stage, NativeStage::Module);
    assert!(
        error.message.contains("does not have terminator"),
        "{error}"
    );
}

#[test]
fn pass_builder_should_select_o0_through_o3_and_verify_each_result() {
    let source = "export fn fold(a: i64) -> i64 { return (a + 1) * 2; }";
    for level in [
        NativeOptimizationLevel::O0,
        NativeOptimizationLevel::O1,
        NativeOptimizationLevel::O2,
        NativeOptimizationLevel::O3,
    ] {
        let text = optimized_llvm(source, level);
        assert!(text.contains("@fold(i64"), "level={level:?}\n{text}");
    }
    let o0 = optimized_llvm(source, NativeOptimizationLevel::O0);
    let o3 = optimized_llvm(source, NativeOptimizationLevel::O3);
    assert!(o0.contains("alloca"), "{o0}");
    assert!(!o3.contains("alloca"), "{o3}");
}

#[test]
fn optimized_structural_control_flow_should_materialize_phi_values() {
    let text = optimized_llvm(
        r#"
          export fn sum(n: i64) -> i64 {
            let i: i64 = 0;
            let total: i64 = 0;
            while i < n { total = total + i; i = i + 1; }
            return total;
          }
        "#,
        NativeOptimizationLevel::O1,
    );
    assert!(text.contains(" phi i64 "), "{text}");
}

#[test]
fn o3_should_preserve_strict_floating_point_operations() {
    let text = optimized_llvm(
        "export fn strict(a: f64, b: f64, c: f64) -> f64 { return a * b + c; }",
        NativeOptimizationLevel::O3,
    );
    assert!(!text.contains(" fadd fast "), "{text}");
    assert!(!text.contains(" fmul fast "), "{text}");
    assert!(!text.contains("contract"), "{text}");
    assert!(text.contains("fmul double"), "{text}");
    assert!(text.contains("fadd double"), "{text}");
}

#[test]
fn checked_lowering_should_cover_all_four_overflow_and_bounds_combinations() {
    let source = r#"
      export fn compute(items: slice<i32>, index: u32, delta: i32) -> i32 {
        return items[index] + delta;
      }
    "#;
    for (overflow, bounds, has_overflow, has_bounds, has_status_result) in [
        (
            OverflowMode::Unchecked,
            BoundsMode::Unchecked,
            false,
            false,
            false,
        ),
        (
            OverflowMode::Checked,
            BoundsMode::Unchecked,
            true,
            false,
            true,
        ),
        (
            OverflowMode::Unchecked,
            BoundsMode::Checked,
            false,
            true,
            true,
        ),
        (OverflowMode::Checked, BoundsMode::Checked, true, true, true),
    ] {
        let text = checked_llvm(source, overflow, bounds);
        assert_eq!(
            text.contains("llvm.sadd.with.overflow.i32"),
            has_overflow,
            "{text}"
        );
        assert_eq!(text.contains("icmp uge i32"), has_bounds, "{text}");
        assert_eq!(text.contains("ptr %ck_return"), has_status_result, "{text}");
        if has_status_result {
            assert!(text.contains("icmp eq ptr %ck_return, null"), "{text}");
            assert!(text.contains("ret i32 3"), "{text}");
        }
    }
}

#[test]
fn checked_overflow_should_guard_division_and_modulo_without_traps() {
    let text = checked_llvm(
        r#"
          export fn divide(a: i64, b: i64) -> i64 { return a / b; }
          export fn modulo(a: i32, b: i32) -> i32 { return a % b; }
        "#,
        OverflowMode::Checked,
        BoundsMode::Unchecked,
    );
    assert!(text.matches("ret i32 2").count() >= 2, "{text}");
    assert!(text.matches("ret i32 1").count() >= 2, "{text}");
    assert!(text.contains("icmp eq i64"), "{text}");
    assert!(text.contains("-9223372036854775808"), "{text}");
    assert!(text.contains("sdiv i64"), "{text}");
    assert!(text.contains("srem i32"), "{text}");
    assert!(!text.contains("llvm.trap"), "{text}");
}

#[test]
fn checked_bounds_should_validate_subslice_before_pointer_advance() {
    let text = checked_llvm(
        "fn cut(items: slice<i64>, start: u32, end: u32) -> slice<i64> { return items[start..end]; } export fn first(items: slice<i64>, start: u32, end: u32) -> i64 { let part: slice<i64> = cut(items, start, end); return part[0]; }",
        OverflowMode::Unchecked,
        BoundsMode::Checked,
    );
    let start_end = text.find("icmp ugt i32").expect("start <= end guard");
    let second_guard = text[start_end + 1..]
        .find("icmp ugt i32")
        .map(|offset| start_end + 1 + offset)
        .expect("end <= len guard");
    let pointer_advance = text.find("getelementptr i64").expect("subslice GEP");
    assert!(
        start_end < second_guard && second_guard < pointer_advance,
        "{text}"
    );
    assert_eq!(text.matches("ret i32 4").count(), 2, "{text}");
}

#[test]
fn checked_calls_should_propagate_status_and_void_should_not_gain_result_pointer() {
    let text = checked_llvm(
        r#"
          fn touch(value: i32) -> void { let next: i32 = value + 1; }
          export fn run(value: i32) -> void { touch(value); }
        "#,
        OverflowMode::Checked,
        BoundsMode::Unchecked,
    );
    assert!(
        text.contains("define internal i32 @touch(i32 %value)"),
        "{text}"
    );
    assert!(text.contains("define i32 @run(i32"), "{text}");
    assert!(!text.contains("define i32 @run(i32 %arg0, ptr"), "{text}");
    assert!(text.contains("call i32 @touch(i32"), "{text}");
    assert!(text.contains("icmp ne i32"), "{text}");
}

#[test]
fn llvm_kir_lowering_should_consume_explicit_guards_without_reinferring_removed_checks() {
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("host target");
    for (source, level, expected_overflow_intrinsics) in [
        (
            "export fn add(a: i32, b: i32) -> i32 { return a + b; }",
            KirOptimizationLevel::O0,
            2,
        ),
        (
            "export fn answer() -> i32 { return 20 + 22; }",
            KirOptimizationLevel::O3,
            0,
        ),
    ] {
        let checked = check(&SourceFile::new("kir-native.ck", source));
        assert_eq!(checked.diagnostics, []);
        let mir = lower_to_mir(&checked.checked_program).expect("MIR");
        let kir = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::NativeLibrary,
                overflow_mode: KirOverflowMode::Checked,
                bounds_mode: KirBoundsMode::Checked,
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("KIR");
        let result = run_kir_pass_pipeline(kir, level, None);
        let text = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
            .expect("KIR LLVM lowering")
            .verify()
            .expect("LLVM verify")
            .to_ir_string()
            .expect("LLVM print");
        assert_eq!(
            text.matches("llvm.sadd.with.overflow.i32").count(),
            expected_overflow_intrinsics,
            "{level:?}:\n{text}"
        );
    }
}

#[test]
fn llvm_o3_canonical_checked_proof_loop_should_not_reintroduce_a_bounds_guard() {
    const SOURCE: &str = include_str!("../fixtures/performance/native/proof_loop.ck");
    let checked = check(&SourceFile::new("proof_loop.ck", SOURCE));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("checked proof-loop KIR");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let bounds_guards = result
        .artifact
        .as_ref()
        .expect("verified proof-loop artifact")
        .functions
        .iter()
        .flat_map(|function| &function.blocks)
        .flat_map(|block| &block.instructions)
        .filter(|instruction| {
            matches!(
                instruction.kind,
                KirInstructionKind::Guard {
                    failure: KirFailureKind::OutOfBounds,
                    ..
                }
            )
        })
        .count();
    assert_eq!(bounds_guards, 0, "KIR retained a proof-loop bounds guard");

    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("host target");
    let text = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower checked proof loop")
        .verify()
        .expect("verify checked proof loop")
        .audit()
        .expect("audit checked proof-loop facts")
        .optimize(&target, NativeOptimizationLevel::O3)
        .expect("optimize checked proof loop")
        .to_ir_string()
        .expect("print checked proof loop");

    assert!(!text.contains("ret i32 4"), "{text}");
    assert!(!text.contains("icmp uge i32"), "{text}");
}

#[test]
fn llvm_kir_executable_lowering_should_add_entry_wrapper_and_runtime_calls() {
    let checked = check(&SourceFile::new(
        "kir-executable.ck",
        "fn main() -> i32 { print_i32(42); print_newline(); return 0; }",
    ));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR");
    let kir = build_kir_module(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeExecutable,
            overflow_mode: KirOverflowMode::Checked,
            bounds_mode: KirBoundsMode::Checked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    let result = run_kir_pass_pipeline(kir, KirOptimizationLevel::O3, None);
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    let context = NativeContext::new().expect("context");
    let target = NativeTarget::host().expect("target");
    let text = lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
        .expect("lower executable KIR")
        .verify()
        .expect("verify")
        .to_ir_string()
        .expect("LLVM IR");

    assert!(
        text.contains("define internal i32 @__ck_user_main("),
        "{text}"
    );
    assert!(text.contains("define i32 @main()"), "{text}");
    assert!(text.contains("call void @__ck_print_i32"), "{text}");
    assert!(text.contains("call void @__ck_print_newline"), "{text}");
    assert!(
        text.contains("declare void @__ck_runtime_fail(i32)"),
        "{text}"
    );
}
