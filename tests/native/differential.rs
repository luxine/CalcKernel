use std::{fs, path::Path, process::Command};

use calckernel::{
    EmitLlvmOptions, KirBoundsMode, KirBuildConfig, KirConsumer, KirOptimizationLevel,
    KirOverflowMode, KirSanitizerMode, NativeContext, NativeCpu, NativeOptimizationLevel,
    NativeTarget, SourceFile, build_kir_module, build_kir_module_with_profile, check,
    emit_c_kir_module_with_contracts, import_contract_facts, link_native_dynamic_library,
    lower_native_kir_module, lower_to_mir, run_kir_pass_pipeline,
};

use crate::generated::{GeneratedKernelCase, fixed_seed_kernel_program};

const SOURCE: &str = r#"
struct Pair {
  left: i64;
  right: i64;
}

fn add_inner(a: i64, b: i64) -> i64 { return a + b; }

export fn scalar(a: i64, b: i64) -> i64 { return add_inner(a, b); }
export fn control(value: i64, choose_left: i32) -> i64 {
  if choose_left != 0 { return value + 3; }
  return value - 7;
}
export fn touch(value: ptr<i64>) -> void {
  value[0] = add_inner(value[0], 5);
  return;
}
export fn echo_pair(value: Pair) -> Pair { return value; }
export fn pointer_value(value: ptr<i64>) -> i64 { return value[0]; }
export fn slice_read(items: slice<i64>, index: u32) -> i64 { return items[index]; }
export fn checked_order(items: slice<i64>, index: u32, value: i64) -> i64 {
  return items[index] + value;
}
export fn quotient(value: i64, divisor: i64) -> i64 { return value / divisor; }
"#;

const VECTOR_SOURCE: &str = r#"
export unsafe fn map_u32(a: slice<u32>, b: slice<u32>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 7; i = i + 1; }
}

export fn copy_unknown(a: slice<u32>, b: slice<u32>, n: u32) -> void {
  let i: u32 = 0;
  while i < n { b[i] = a[i] + 1; i = i + 1; }
}

export unsafe fn map_f64(a: slice<f64>, b: slice<f64>, n: u32, factor: f64) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = a[i] * factor; i = i + 1; }
}

export unsafe fn map_f64_div(
  a: slice<f64>, b: slice<f64>, n: u32, divisor: f64
) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = -a[i] / divisor; i = i + 1; }
}

export unsafe fn map_cast(a: slice<u32>, b: slice<f64>, n: u32) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n { b[i] = u32_to_f64(a[i]); i = i + 1; }
}

export unsafe fn map_diamond(
  a: slice<u32>, b: slice<u32>, n: u32, pivot: u32
) -> void
contract { requires noalias(a, b); effects read(a), write(b); }
{
  let i: u32 = 0;
  while i < n {
    let x: u32 = a[i];
    let selected: u32 = 0;
    if x < pivot { selected = x + 1; } else { selected = x - 1; }
    b[i] = selected;
    i = i + 1;
  }
}

export fn sum_u32(a: slice<u32>, n: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 0;
  while i < n { total = total + a[i]; i = i + 1; }
  return total;
}

export fn product_u32(a: slice<u32>, n: u32) -> u32 {
  let i: u32 = 0;
  let total: u32 = 1;
  while i < n { total = total * a[i]; i = i + 1; }
  return total;
}
"#;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pair {
    left: i64,
    right: i64,
}

#[test]
fn differential_native_exports_should_match_pinned_clang_c_libraries_at_o0_through_o3() {
    let Some(clang) = super::support::oracle::clang_oracle_22() else {
        return;
    };
    let root = std::env::temp_dir().join(format!("ckc-native-differential-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).expect("create differential directory");

    for checked in [false, true] {
        let checked_program = check(&SourceFile::new("differential.ck", SOURCE));
        assert_eq!(checked_program.diagnostics, []);
        let mir = lower_to_mir(&checked_program.checked_program).expect("lower differential MIR");
        let exports = mir
            .functions
            .iter()
            .filter(|function| function.exported)
            .map(|function| function.name.clone())
            .collect::<Vec<_>>();
        let c_kir = build_kir_module(
            &mir,
            KirBuildConfig {
                consumer: KirConsumer::C,
                overflow_mode: if checked {
                    KirOverflowMode::Checked
                } else {
                    KirOverflowMode::Unchecked
                },
                bounds_mode: if checked {
                    KirBoundsMode::Checked
                } else {
                    KirBoundsMode::Unchecked
                },
                sanitizer_mode: KirSanitizerMode::Disabled,
            },
        )
        .expect("build C oracle KIR");
        let c_contracts = import_contract_facts(&c_kir, &checked_program.checked_program, 0)
            .expect("import C oracle facts");
        let c_result = run_kir_pass_pipeline(c_kir, KirOptimizationLevel::O3, Some(&c_contracts));
        let c_source = emit_c_kir_module_with_contracts(
            c_result.artifact.as_ref().expect("verified C oracle KIR"),
            c_result.contract_facts.as_ref(),
        )
        .expect("emit C oracle source");
        let suffix = if checked { "checked" } else { "unchecked" };
        let c_path = root.join(format!("oracle-{suffix}.c"));
        let oracle_path = root.join(format!("oracle-{suffix}{}", dynamic_suffix()));
        fs::write(&c_path, c_source).expect("write C oracle source");
        compile_oracle_library(&clang, &c_path, &oracle_path, &exports);

        let oracle = DynamicLibrary::open(&oracle_path);
        for (kir_level, native_level, level_name) in [
            (KirOptimizationLevel::O0, NativeOptimizationLevel::O0, "o0"),
            (KirOptimizationLevel::O1, NativeOptimizationLevel::O1, "o1"),
            (KirOptimizationLevel::O2, NativeOptimizationLevel::O2, "o2"),
            (KirOptimizationLevel::O3, NativeOptimizationLevel::O3, "o3"),
        ] {
            let kir = build_kir_module(
                &mir,
                KirBuildConfig {
                    consumer: KirConsumer::NativeLibrary,
                    overflow_mode: if checked {
                        KirOverflowMode::Checked
                    } else {
                        KirOverflowMode::Unchecked
                    },
                    bounds_mode: if checked {
                        KirBoundsMode::Checked
                    } else {
                        KirBoundsMode::Unchecked
                    },
                    sanitizer_mode: KirSanitizerMode::Disabled,
                },
            )
            .expect("build native differential KIR");
            let result = run_kir_pass_pipeline(kir, kir_level, None);
            assert!(
                result.errors.is_empty(),
                "{kir_level:?}: {:?}",
                result.errors
            );
            let context = NativeContext::new().expect("native context");
            let target = NativeTarget::host().expect("native target");
            let optimized =
                lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
                    .expect("lower native differential KIR")
                    .verify()
                    .expect("verify differential module")
                    .audit()
                    .expect("audit differential module facts")
                    .optimize(&target, native_level)
                    .expect("optimize differential module");
            let object = target
                .emit_object(optimized)
                .expect("emit differential object");
            let native = link_native_dynamic_library(&object, &exports)
                .expect("link native differential library");
            let native_path =
                root.join(format!("native-{suffix}-{level_name}{}", dynamic_suffix()));
            fs::write(&native_path, native.as_bytes()).expect("write native differential library");
            let native = DynamicLibrary::open(&native_path);
            if checked {
                unsafe { compare_checked(&oracle, &native) };
            } else {
                unsafe { compare_unchecked(&oracle, &native) };
            }
        }
    }
    fs::remove_dir_all(root).expect("remove differential directory");
}

#[test]
fn generated_native_kernels_should_match_o0_at_o1_through_o3_in_every_supported_mode() {
    let generated = fixed_seed_kernel_program();
    let checked_program = check(&SourceFile::new(
        "generated-differential.ck",
        &generated.source,
    ));
    assert_eq!(checked_program.diagnostics, []);
    let mir = lower_to_mir(&checked_program.checked_program).expect("lower generated MIR");
    let root = std::env::temp_dir().join(format!(
        "ckc-native-generated-differential-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).expect("create generated differential directory");

    for (overflow, overflow_name) in [
        (KirOverflowMode::Unchecked, "unchecked"),
        (KirOverflowMode::Checked, "checked"),
    ] {
        for (bounds, bounds_name) in [
            (KirBoundsMode::Unchecked, "unchecked"),
            (KirBoundsMode::Checked, "checked"),
        ] {
            let checked_abi =
                overflow == KirOverflowMode::Checked || bounds == KirBoundsMode::Checked;
            let mut o0_observations = None;
            for (kir_level, native_level, level_name) in [
                (KirOptimizationLevel::O0, NativeOptimizationLevel::O0, "o0"),
                (KirOptimizationLevel::O1, NativeOptimizationLevel::O1, "o1"),
                (KirOptimizationLevel::O2, NativeOptimizationLevel::O2, "o2"),
                (KirOptimizationLevel::O3, NativeOptimizationLevel::O3, "o3"),
            ] {
                let kir = build_kir_module(
                    &mir,
                    KirBuildConfig {
                        consumer: KirConsumer::NativeLibrary,
                        overflow_mode: overflow,
                        bounds_mode: bounds,
                        sanitizer_mode: KirSanitizerMode::Disabled,
                    },
                )
                .expect("build generated native KIR");
                let contracts = import_contract_facts(&kir, &checked_program.checked_program, 0)
                    .expect("import generated contract facts");
                let result = run_kir_pass_pipeline(kir, kir_level, Some(&contracts));
                assert!(
                    result.errors.is_empty(),
                    "{overflow:?}/{bounds:?}/{kir_level:?}: {:?}",
                    result.errors
                );
                let context = NativeContext::new().expect("native context");
                let target = NativeTarget::host().expect("native target");
                let optimized = lower_native_kir_module(
                    &context,
                    &target,
                    &result,
                    &EmitLlvmOptions::default(),
                )
                .expect("lower generated native KIR")
                .verify()
                .expect("verify generated native module")
                .audit()
                .expect("audit generated native facts")
                .optimize(&target, native_level)
                .expect("optimize generated native module");
                let object = target
                    .emit_object(optimized)
                    .expect("emit generated native object");
                let exports = generated
                    .cases
                    .iter()
                    .map(|case| case.function.clone())
                    .collect::<Vec<_>>();
                let library = link_native_dynamic_library(&object, &exports)
                    .expect("link generated native library");
                let path = root.join(format!(
                    "generated-{}-{}-{level_name}{}",
                    overflow_name,
                    bounds_name,
                    dynamic_suffix()
                ));
                fs::write(&path, library.as_bytes()).expect("write generated native library");
                let library = DynamicLibrary::open(&path);
                let observations =
                    unsafe { observe_generated(&library, &generated.cases, checked_abi) };
                assert_eq!(
                    observations,
                    generated
                        .cases
                        .iter()
                        .map(|case| case.expected)
                        .collect::<Vec<_>>(),
                    "{overflow:?}/{bounds:?}/{kir_level:?}"
                );
                if let Some(o0) = &o0_observations {
                    assert_eq!(
                        &observations, o0,
                        "{overflow:?}/{bounds:?}: {level_name} diverged from O0"
                    );
                } else {
                    o0_observations = Some(observations);
                }
            }
        }
    }

    fs::remove_dir_all(root).expect("remove generated differential directory");
}

#[test]
fn differential_vector_loop_should_match_o0_for_zero_short_exact_remainder_and_overlap_fallback() {
    let root = std::env::temp_dir().join(format!(
        "ckc-native-vector-differential-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir(&root).expect("create vector differential directory");
    let o0_path = compile_vector_library(
        &root,
        KirOptimizationLevel::O0,
        NativeOptimizationLevel::O0,
        "o0",
    );
    let o3_path = compile_vector_library(
        &root,
        KirOptimizationLevel::O3,
        NativeOptimizationLevel::O3,
        "o3",
    );
    let o0 = DynamicLibrary::open(&o0_path);
    let o3 = DynamicLibrary::open(&o3_path);
    type Map = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, u32);
    for len in [0_u32, 3, 8, 9, 16, 19, 31] {
        let input = std::array::from_fn::<_, 32, _>(|index| {
            u32::try_from(index)
                .unwrap()
                .wrapping_mul(17)
                .wrapping_add(5)
        });
        let mut baseline = [0_u32; 32];
        let mut vector = [0_u32; 32];
        let baseline_map: Map = unsafe { o0.symbol("map_u32") };
        let vector_map: Map = unsafe { o3.symbol("map_u32") };
        unsafe {
            baseline_map(
                input.as_ptr().cast_mut(),
                input.len() as u32,
                baseline.as_mut_ptr(),
                baseline.len() as u32,
                len,
            );
            vector_map(
                input.as_ptr().cast_mut(),
                input.len() as u32,
                vector.as_mut_ptr(),
                vector.len() as u32,
                len,
            );
        }
        assert_eq!(vector, baseline, "len={len}");
        for index in 0..usize::try_from(len).unwrap() {
            assert_eq!(vector[index], input[index].wrapping_add(7), "len={len}");
        }
    }

    let mut baseline_overlap = std::array::from_fn::<_, 16, _>(|index| index as u32 + 1);
    let mut vector_overlap = baseline_overlap;
    let baseline_copy: Map = unsafe { o0.symbol("copy_unknown") };
    let vector_copy: Map = unsafe { o3.symbol("copy_unknown") };
    let disjoint_input = std::array::from_fn::<_, 16, _>(|index| index as u32 + 11);
    let mut baseline_disjoint = [0_u32; 16];
    let mut vector_disjoint = [0_u32; 16];
    unsafe {
        baseline_copy(
            disjoint_input.as_ptr().cast_mut(),
            16,
            baseline_disjoint.as_mut_ptr(),
            16,
            16,
        );
        vector_copy(
            disjoint_input.as_ptr().cast_mut(),
            16,
            vector_disjoint.as_mut_ptr(),
            16,
            16,
        );
    }
    assert_eq!(vector_disjoint, baseline_disjoint);
    unsafe {
        baseline_copy(
            baseline_overlap.as_mut_ptr(),
            15,
            baseline_overlap.as_mut_ptr().add(1),
            15,
            12,
        );
        vector_copy(
            vector_overlap.as_mut_ptr(),
            15,
            vector_overlap.as_mut_ptr().add(1),
            15,
            12,
        );
    }
    assert_eq!(vector_overlap, baseline_overlap);

    type MapF64 = unsafe extern "C" fn(*mut f64, u32, *mut f64, u32, u32, f64);
    let baseline_f64: MapF64 = unsafe { o0.symbol("map_f64") };
    let vector_f64: MapF64 = unsafe { o3.symbol("map_f64") };
    let baseline_f64_div: MapF64 = unsafe { o0.symbol("map_f64_div") };
    let vector_f64_div: MapF64 = unsafe { o3.symbol("map_f64_div") };
    for len in [0_u32, 3, 4, 5, 16, 19, 31] {
        let input = std::array::from_fn::<_, 32, _>(|index| index as f64 * 0.25 - 2.75);
        let mut baseline = [0.0_f64; 32];
        let mut vector = [0.0_f64; 32];
        unsafe {
            baseline_f64(
                input.as_ptr().cast_mut(),
                32,
                baseline.as_mut_ptr(),
                32,
                len,
                1.75,
            );
            vector_f64(
                input.as_ptr().cast_mut(),
                32,
                vector.as_mut_ptr(),
                32,
                len,
                1.75,
            );
        }
        assert_eq!(
            vector.map(f64::to_bits),
            baseline.map(f64::to_bits),
            "strict f64 len={len}"
        );
        let mut baseline_div = [0.0_f64; 32];
        let mut vector_div = [0.0_f64; 32];
        unsafe {
            baseline_f64_div(
                input.as_ptr().cast_mut(),
                32,
                baseline_div.as_mut_ptr(),
                32,
                len,
                1.75,
            );
            vector_f64_div(
                input.as_ptr().cast_mut(),
                32,
                vector_div.as_mut_ptr(),
                32,
                len,
                1.75,
            );
        }
        assert_eq!(
            vector_div.map(f64::to_bits),
            baseline_div.map(f64::to_bits),
            "strict f64 unary/divide len={len}"
        );
    }

    type MapCast = unsafe extern "C" fn(*mut u32, u32, *mut f64, u32, u32);
    let baseline_cast: MapCast = unsafe { o0.symbol("map_cast") };
    let vector_cast: MapCast = unsafe { o3.symbol("map_cast") };
    type MapDiamond = unsafe extern "C" fn(*mut u32, u32, *mut u32, u32, u32, u32);
    let baseline_diamond: MapDiamond = unsafe { o0.symbol("map_diamond") };
    let vector_diamond: MapDiamond = unsafe { o3.symbol("map_diamond") };
    for len in [0_u32, 3, 8, 9, 16, 19, 31] {
        let input = std::array::from_fn::<_, 32, _>(|index| {
            u32::try_from(index)
                .unwrap()
                .wrapping_mul(17)
                .wrapping_add(5)
        });
        let mut baseline_cast_output = [0.0_f64; 32];
        let mut vector_cast_output = [0.0_f64; 32];
        let mut baseline_diamond_output = [0_u32; 32];
        let mut vector_diamond_output = [0_u32; 32];
        unsafe {
            baseline_cast(
                input.as_ptr().cast_mut(),
                32,
                baseline_cast_output.as_mut_ptr(),
                32,
                len,
            );
            vector_cast(
                input.as_ptr().cast_mut(),
                32,
                vector_cast_output.as_mut_ptr(),
                32,
                len,
            );
            baseline_diamond(
                input.as_ptr().cast_mut(),
                32,
                baseline_diamond_output.as_mut_ptr(),
                32,
                len,
                200,
            );
            vector_diamond(
                input.as_ptr().cast_mut(),
                32,
                vector_diamond_output.as_mut_ptr(),
                32,
                len,
                200,
            );
        }
        assert_eq!(
            vector_cast_output.map(f64::to_bits),
            baseline_cast_output.map(f64::to_bits),
            "cast len={len}"
        );
        assert_eq!(
            vector_diamond_output, baseline_diamond_output,
            "diamond len={len}"
        );
    }

    type Reduce = unsafe extern "C" fn(*mut u32, u32, u32) -> u32;
    let baseline_sum: Reduce = unsafe { o0.symbol("sum_u32") };
    let vector_sum: Reduce = unsafe { o3.symbol("sum_u32") };
    let baseline_product: Reduce = unsafe { o0.symbol("product_u32") };
    let vector_product: Reduce = unsafe { o3.symbol("product_u32") };
    let reduction_input = std::array::from_fn::<_, 32, _>(|index| {
        u32::try_from(index)
            .unwrap()
            .wrapping_mul(3)
            .wrapping_add(1)
    });
    for len in [0_u32, 3, 8, 9, 16, 19, 31] {
        let baseline_sum_value =
            unsafe { baseline_sum(reduction_input.as_ptr().cast_mut(), 32, len) };
        let vector_sum_value = unsafe { vector_sum(reduction_input.as_ptr().cast_mut(), 32, len) };
        assert_eq!(vector_sum_value, baseline_sum_value, "sum len={len}");
        let baseline_product_value =
            unsafe { baseline_product(reduction_input.as_ptr().cast_mut(), 32, len) };
        let vector_product_value =
            unsafe { vector_product(reduction_input.as_ptr().cast_mut(), 32, len) };
        assert_eq!(
            vector_product_value, baseline_product_value,
            "product len={len}"
        );
    }
    drop(o3);
    drop(o0);
    fs::remove_dir_all(root).expect("remove vector differential directory");
}

fn compile_vector_library(
    root: &Path,
    kir_level: KirOptimizationLevel,
    native_level: NativeOptimizationLevel,
    name: &str,
) -> std::path::PathBuf {
    let checked = check(&SourceFile::new("vector-differential.ck", VECTOR_SOURCE));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("vector differential MIR");
    let target = NativeTarget::host_with_cpu(NativeCpu::Baseline).expect("baseline target");
    let profile = target
        .kir_profile(KirConsumer::NativeLibrary)
        .expect("vector target profile");
    let kir = build_kir_module_with_profile(
        &mir,
        KirBuildConfig {
            consumer: KirConsumer::NativeLibrary,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        profile,
    )
    .expect("vector differential KIR");
    let contracts = import_contract_facts(&kir, &checked.checked_program, 0)
        .expect("vector differential facts");
    let result = run_kir_pass_pipeline(kir, kir_level, Some(&contracts));
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    if kir_level == KirOptimizationLevel::O3 {
        // The frozen target profile, not the source surface alone, decides the
        // profitable subset. Baseline x86-64 conservatively rejects the
        // strict-f64 divide and horizontal multiply-reduction loops at the
        // unchanged 20% threshold; AArch64 accepts the complete corpus.
        let expected_vectorized_loops = if cfg!(target_arch = "x86_64") { 6 } else { 8 };
        assert_eq!(
            result.stats.vectorized_loops, expected_vectorized_loops,
            "{:?}",
            result.analysis_fallbacks
        );
    }
    let context = NativeContext::new().expect("native context");
    let optimized =
        lower_native_kir_module(&context, &target, &result, &EmitLlvmOptions::default())
            .expect("lower vector differential")
            .verify()
            .expect("verify vector differential")
            .audit()
            .expect("audit vector differential")
            .optimize(&target, native_level)
            .expect("optimize vector differential");
    let object = target
        .emit_object(optimized)
        .expect("emit vector differential object");
    let library = link_native_dynamic_library(
        &object,
        &[
            "map_u32".to_string(),
            "copy_unknown".to_string(),
            "map_f64".to_string(),
            "map_f64_div".to_string(),
            "map_cast".to_string(),
            "map_diamond".to_string(),
            "sum_u32".to_string(),
            "product_u32".to_string(),
        ],
    )
    .expect("link vector differential library");
    let path = root.join(format!("vector-{name}{}", dynamic_suffix()));
    fs::write(&path, library.as_bytes()).expect("write vector differential library");
    path
}

unsafe fn observe_generated(
    library: &DynamicLibrary,
    cases: &[GeneratedKernelCase],
    checked_abi: bool,
) -> Vec<i32> {
    type Unchecked = unsafe extern "C" fn(*mut i32, u32, u32, i32) -> i32;
    type Checked = unsafe extern "C" fn(*mut i32, u32, u32, i32, *mut i32) -> i32;
    cases
        .iter()
        .map(|case| {
            let mut values = case.values;
            assert!(case.len <= values.len() as u32);
            if checked_abi {
                let function: Checked = unsafe { library.symbol(&case.function) };
                let mut result = 0;
                assert_eq!(
                    unsafe {
                        function(
                            values.as_mut_ptr(),
                            values.len() as u32,
                            case.len,
                            case.bias,
                            &mut result,
                        )
                    },
                    0,
                    "generated contract-domain call must succeed"
                );
                result
            } else {
                let function: Unchecked = unsafe { library.symbol(&case.function) };
                unsafe {
                    function(
                        values.as_mut_ptr(),
                        values.len() as u32,
                        case.len,
                        case.bias,
                    )
                }
            }
        })
        .collect()
}

fn compile_oracle_library(clang: &Path, source: &Path, output: &Path, exports: &[String]) {
    let mut command = Command::new(clang);
    command.args([
        "-std=c11",
        "-O3",
        "-fno-fast-math",
        "-fuse-ld=lld",
        "-nostdlib",
    ]);
    if cfg!(target_os = "macos") {
        command.args([
            "-dynamiclib",
            "-Wl,-platform_version,macos,11.0,11.0",
            "-Wl,-adhoc_codesign",
        ]);
    } else if cfg!(target_os = "windows") {
        command.args(["-shared", "-Wl,/noentry"]);
        for export in exports {
            command.arg(format!("-Wl,/export:{export}"));
        }
    } else {
        command.args(["-shared", "-fPIC", "-Wl,--no-undefined"]);
    }
    let result = command
        .arg(source)
        .arg("-o")
        .arg(output)
        .output()
        .expect("run Clang oracle");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
}

const fn dynamic_suffix() -> &'static str {
    if cfg!(target_os = "windows") {
        ".dll"
    } else if cfg!(target_os = "macos") {
        ".dylib"
    } else {
        ".so"
    }
}

unsafe fn compare_unchecked(oracle: &DynamicLibrary, native: &DynamicLibrary) {
    type Scalar = unsafe extern "C" fn(i64, i64) -> i64;
    type Control = unsafe extern "C" fn(i64, i32) -> i64;
    type Touch = unsafe extern "C" fn(*mut i64);
    type EchoPair = unsafe extern "C" fn(Pair) -> Pair;
    type Pointer = unsafe extern "C" fn(*mut i64) -> i64;
    type Slice = unsafe extern "C" fn(*mut i64, u32, u32) -> i64;
    for library in [oracle, native] {
        let scalar: Scalar = unsafe { library.symbol("scalar") };
        let control: Control = unsafe { library.symbol("control") };
        let touch: Touch = unsafe { library.symbol("touch") };
        let echo_pair: EchoPair = unsafe { library.symbol("echo_pair") };
        let pointer: Pointer = unsafe { library.symbol("pointer_value") };
        let slice: Slice = unsafe { library.symbol("slice_read") };
        assert_eq!(unsafe { scalar(12, 30) }, 42);
        assert_eq!(unsafe { control(20, 1) }, 23);
        assert_eq!(unsafe { control(20, 0) }, 13);
        let mut value = 10i64;
        unsafe { touch(&mut value) };
        assert_eq!(value, 15);
        assert_eq!(
            unsafe { echo_pair(Pair { left: 7, right: 9 }) },
            Pair { left: 7, right: 9 }
        );
        assert_eq!(unsafe { pointer(&mut value) }, 15);
        let mut values = [4i64, 8, 15, 16];
        assert_eq!(
            unsafe { slice(values.as_mut_ptr(), values.len() as u32, 2) },
            15
        );
    }
}

unsafe fn compare_checked(oracle: &DynamicLibrary, native: &DynamicLibrary) {
    type Scalar = unsafe extern "C" fn(i64, i64, *mut i64) -> i32;
    type Control = unsafe extern "C" fn(i64, i32, *mut i64) -> i32;
    type Touch = unsafe extern "C" fn(*mut i64) -> i32;
    type EchoPair = unsafe extern "C" fn(Pair, *mut Pair) -> i32;
    type Pointer = unsafe extern "C" fn(*mut i64, *mut i64) -> i32;
    type Slice = unsafe extern "C" fn(*mut i64, u32, u32, *mut i64) -> i32;
    type CheckedOrder = unsafe extern "C" fn(*mut i64, u32, u32, i64, *mut i64) -> i32;
    type Quotient = unsafe extern "C" fn(i64, i64, *mut i64) -> i32;
    for library in [oracle, native] {
        let scalar: Scalar = unsafe { library.symbol("scalar") };
        let control: Control = unsafe { library.symbol("control") };
        let touch: Touch = unsafe { library.symbol("touch") };
        let echo_pair: EchoPair = unsafe { library.symbol("echo_pair") };
        let pointer: Pointer = unsafe { library.symbol("pointer_value") };
        let slice: Slice = unsafe { library.symbol("slice_read") };
        let checked_order: CheckedOrder = unsafe { library.symbol("checked_order") };
        let quotient: Quotient = unsafe { library.symbol("quotient") };
        let mut result = 0i64;
        assert_eq!(unsafe { scalar(12, 30, &mut result) }, 0);
        assert_eq!(result, 42);
        assert_eq!(unsafe { control(20, 1, &mut result) }, 0);
        assert_eq!(result, 23);
        let mut value = 10i64;
        assert_eq!(unsafe { touch(&mut value) }, 0);
        assert_eq!(value, 15);
        let mut pair = Pair { left: 0, right: 0 };
        assert_eq!(
            unsafe { echo_pair(Pair { left: 7, right: 9 }, &mut pair) },
            0
        );
        assert_eq!(pair, Pair { left: 7, right: 9 });
        assert_eq!(unsafe { pointer(&mut value, &mut result) }, 0);
        assert_eq!(result, 15);
        let mut values = [1i64, 8];
        assert_eq!(unsafe { slice(values.as_mut_ptr(), 2, 1, &mut result) }, 0);
        assert_eq!(result, 8);
        assert_eq!(
            unsafe { checked_order(values.as_mut_ptr(), 2, 7, i64::MAX, &mut result) },
            4
        );
        assert_eq!(
            unsafe { checked_order(values.as_mut_ptr(), 2, 0, i64::MAX, &mut result) },
            1
        );
        assert_eq!(unsafe { quotient(10, 0, &mut result) }, 2);
    }
}

struct DynamicLibrary {
    handle: *mut std::ffi::c_void,
}

impl DynamicLibrary {
    fn open(path: &Path) -> Self {
        platform_loader::open(path)
    }

    unsafe fn symbol<T: Copy>(&self, name: &str) -> T {
        let address = platform_loader::symbol(self.handle, name);
        assert_eq!(std::mem::size_of::<T>(), std::mem::size_of_val(&address));
        unsafe { std::mem::transmute_copy(&address) }
    }
}

impl Drop for DynamicLibrary {
    fn drop(&mut self) {
        platform_loader::close(self.handle);
    }
}

#[cfg(unix)]
mod platform_loader {
    use std::{ffi::CString, path::Path};

    pub(super) fn open(path: &Path) -> super::DynamicLibrary {
        let path = CString::new(path.to_string_lossy().as_bytes()).expect("library path");
        // SAFETY: The path is NUL-terminated and remains live for the call.
        let handle = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(
            !handle.is_null(),
            "dlopen failed for {}",
            path.to_string_lossy()
        );
        super::DynamicLibrary { handle }
    }

    pub(super) fn symbol(handle: *mut std::ffi::c_void, name: &str) -> *mut std::ffi::c_void {
        let name = CString::new(name).expect("symbol name");
        // SAFETY: The handle is live and the symbol name is NUL-terminated.
        let address = unsafe { dlsym(handle, name.as_ptr()) };
        assert!(
            !address.is_null(),
            "missing symbol {}",
            name.to_string_lossy()
        );
        address
    }

    pub(super) fn close(handle: *mut std::ffi::c_void) {
        // SAFETY: `DynamicLibrary` owns one live handle and closes it once.
        assert_eq!(unsafe { dlclose(handle) }, 0);
    }

    unsafe extern "C" {
        fn dlopen(path: *const std::ffi::c_char, mode: std::ffi::c_int) -> *mut std::ffi::c_void;
        fn dlsym(
            handle: *mut std::ffi::c_void,
            name: *const std::ffi::c_char,
        ) -> *mut std::ffi::c_void;
        fn dlclose(handle: *mut std::ffi::c_void) -> std::ffi::c_int;
    }
}

#[cfg(windows)]
mod platform_loader {
    use std::{ffi::CString, os::windows::ffi::OsStrExt, path::Path};

    pub(super) fn open(path: &Path) -> super::DynamicLibrary {
        let mut wide = path.as_os_str().encode_wide().collect::<Vec<_>>();
        wide.push(0);
        // SAFETY: The path is NUL-terminated and remains live for the call.
        let handle = unsafe { LoadLibraryW(wide.as_ptr()) };
        assert!(
            !handle.is_null(),
            "LoadLibraryW failed for {}",
            path.display()
        );
        super::DynamicLibrary { handle }
    }

    pub(super) fn symbol(handle: *mut std::ffi::c_void, name: &str) -> *mut std::ffi::c_void {
        let name = CString::new(name).expect("symbol name");
        // SAFETY: The handle is live and the symbol name is NUL-terminated.
        let address = unsafe { GetProcAddress(handle, name.as_ptr()) };
        assert!(
            !address.is_null(),
            "missing symbol {}",
            name.to_string_lossy()
        );
        address
    }

    pub(super) fn close(handle: *mut std::ffi::c_void) {
        // SAFETY: `DynamicLibrary` owns one live handle and closes it once.
        assert_ne!(unsafe { FreeLibrary(handle) }, 0);
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryW(path: *const u16) -> *mut std::ffi::c_void;
        fn GetProcAddress(handle: *mut std::ffi::c_void, name: *const i8) -> *mut std::ffi::c_void;
        fn FreeLibrary(handle: *mut std::ffi::c_void) -> i32;
    }
}
