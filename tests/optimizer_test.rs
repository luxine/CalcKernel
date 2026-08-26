use calckernel::{
    MirPassBoundsMode, MirPassContext, MirPassOverflowMode, MirPassTargetBackend, SourceFile,
    build_mir_optimization_pipeline, check, lower_to_mir, print_mir_module,
    print_mir_pass_pipeline, run_mir_pass_pipeline,
};

fn optimize(source_text: &str, opt_level: u8, overflow_mode: MirPassOverflowMode) -> String {
    optimize_with_bounds(
        source_text,
        opt_level,
        overflow_mode,
        MirPassBoundsMode::Unchecked,
    )
}

fn optimize_with_bounds(
    source_text: &str,
    opt_level: u8,
    overflow_mode: MirPassOverflowMode,
    bounds_mode: MirPassBoundsMode,
) -> String {
    let checked = check(&SourceFile::new("test.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    let pipeline = build_mir_optimization_pipeline(opt_level);
    let result = run_mir_pass_pipeline(
        mir,
        &pipeline,
        &MirPassContext {
            opt_level,
            overflow_mode,
            bounds_mode,
            target_backend: MirPassTargetBackend::C,
            debug: Default::default(),
        },
    );
    assert_eq!(result.validation_errors, []);
    print_mir_module(&result.module)
}

#[test]
fn optimizer_should_build_ts_compatible_pipeline_names() {
    assert_eq!(
        build_mir_optimization_pipeline(0)
            .passes
            .iter()
            .map(|pass| pass.name)
            .collect::<Vec<_>>(),
        Vec::<&str>::new()
    );
    assert_eq!(
        build_mir_optimization_pipeline(1)
            .passes
            .iter()
            .map(|pass| pass.name)
            .collect::<Vec<_>>(),
        vec![
            "constant-folding",
            "copy-propagation",
            "dead-code-elimination",
            "cfg-simplify"
        ]
    );
    assert_eq!(
        print_mir_pass_pipeline(&build_mir_optimization_pipeline(3)),
        "O3: constant-folding -> copy-propagation -> inline-small-functions -> constant-folding -> copy-propagation -> loop-analysis -> loop-invariant-code-motion -> induction-simplify -> constant-folding -> copy-propagation -> local-cse -> copy-propagation -> address-cse -> dead-code-elimination -> cfg-simplify -> dead-code-elimination"
    );
}

#[test]
fn optimizer_should_fold_safe_integer_bool_and_unary_constants() {
    let text = optimize(
        r#"
      export fn calc_i32() -> i32 {
        return 1 + 2 * 3;
      }

      export fn less() -> bool {
        return 1 < 2;
      }

      export fn neg() -> i32 {
        return -1;
      }

      export fn not_false() -> bool {
        return !false;
      }
    "#,
        1,
        MirPassOverflowMode::Unchecked,
    );

    assert!(text.contains("%t4: i32 = const_int 7"));
    assert!(text.contains("%t2: bool = const_bool true"));
    assert!(text.contains("%t1: i32 = const_int -1"));
    assert!(text.contains("%t1: bool = const_bool true"));
    assert!(!text.contains(" = mul "));
    assert!(!text.contains(" = add "));
    assert!(!text.contains(" = lt "));
    assert!(!text.contains(" = neg "));
    assert!(!text.contains(" = not "));
}

#[test]
fn optimizer_should_not_fold_checked_integer_or_f64_arithmetic() {
    let checked = optimize(
        "export fn calc() -> i64 { return 1 + 2; }",
        1,
        MirPassOverflowMode::Checked,
    );
    assert!(checked.contains("%t2: i64 = add %t0, %t1"));

    let f64_text = optimize(
        "export fn calc() -> f64 { return 1.0 + 2.0; }",
        1,
        MirPassOverflowMode::Unchecked,
    );
    assert!(f64_text.contains("const_float 1.0"));
    assert!(f64_text.contains("const_float 2.0"));
    assert!(f64_text.contains("add %t0, %t1"));
}

#[test]
fn optimizer_should_keep_overflow_and_division_by_zero_unfolded() {
    let text = optimize(
        r#"
      export fn add_overflow() -> i32 {
        return 2147483647 + 1;
      }

      export fn div_zero() -> i32 {
        return 1 / 0;
      }
    "#,
        1,
        MirPassOverflowMode::Unchecked,
    );

    assert!(text.contains("add %t0, %t1"));
    assert!(text.contains("div %t0, %t1"));
}

#[test]
fn optimizer_should_simplify_constant_branches_at_o2() {
    let text = optimize(
        r#"
      export fn choose() -> i32 {
        if true {
          return 1;
        } else {
          return 2;
        }
      }
    "#,
        2,
        MirPassOverflowMode::Unchecked,
    );

    assert!(!text.contains("branch"));
    assert!(!text.contains("bb2:"));
    assert!(text.contains("return %t1"));
}

#[test]
fn optimizer_should_apply_local_cse_for_repeated_integer_expressions_at_o2() {
    let text = optimize(
        r#"
      export fn repeat(a: i64, b: i64) -> i64 {
        let x: i64 = a + b;
        let y: i64 = b + a;
        return x + y;
      }
    "#,
        2,
        MirPassOverflowMode::Unchecked,
    );

    assert!(text.contains("%t0: i64 = add a, b"));
    assert!(!text.contains("add b, a"));
    assert!(text.contains("y: i64 = move %t0"));
}

#[test]
fn optimizer_should_apply_f64_local_cse_without_reordering_operands_at_o2() {
    let text = optimize(
        r#"
      export fn repeat_f64(a: f64, b: f64) -> f64 {
        let x: f64 = a + b;
        let y: f64 = b + a;
        let z: f64 = a + b;
        return x + y + z;
      }
    "#,
        2,
        MirPassOverflowMode::Unchecked,
    );

    assert!(text.contains("%t0: f64 = add a, b"));
    assert!(text.contains("%t1: f64 = add b, a"));
    assert!(text.contains("z: f64 = move %t0"));
}

#[test]
fn optimizer_should_apply_address_cse_for_repeated_indexed_places_at_o2() {
    let text = optimize(
        r#"
      struct Item {
        price: i64;
        qty: i64;
      }

      export fn calc(items: ptr<Item>, idx: i32) -> i64 {
        let price: i64 = items[idx].price;
        let qty: i64 = items[idx].qty;
        return price * qty;
      }
    "#,
        2,
        MirPassOverflowMode::Unchecked,
    );

    assert!(text.contains("%addr0: ptr<Item> = address index(items, idx)"));
    assert!(text.contains("load field(deref(%addr0), price)"));
    assert!(text.contains("load field(deref(%addr0), qty)"));
    assert!(!text.contains("load field(index(items, idx), price)"));
    assert!(!text.contains("load field(index(items, idx), qty)"));
}

#[test]
fn optimizer_should_inline_small_internal_helpers_at_o2() {
    let text = optimize(
        r#"
      fn add_one(x: i64) -> i64 {
        return x + 1;
      }

      export fn calc(a: i64) -> i64 {
        return add_one(a) * 2;
      }
    "#,
        2,
        MirPassOverflowMode::Unchecked,
    );

    assert!(!text.contains("call add_one"));
    assert!(!text.contains("fn add_one"));
    assert!(text.contains("export fn calc"));
    assert!(text.contains("add a,"));
    assert!(text.contains("mul"));
}

#[test]
fn optimizer_should_hoist_loop_invariant_integer_work_at_o3() {
    let text = optimize(
        r#"
      export fn calc(n: i64, a: i64, b: i64) -> i64 {
        let i: i64 = 0;
        let sum: i64 = 0;

        while i < n {
          sum = sum + (a * b + 7);
          i = i + 1;
        }

        return sum;
      }
    "#,
        3,
        MirPassOverflowMode::Unchecked,
    );

    let header_index = text.find("bb1:").expect("loop header");
    let product_index = text.find("mul a, b").expect("invariant product");
    let add_const_index = text.find("add %").expect("invariant add");

    assert!(product_index < header_index);
    assert!(add_const_index < header_index);
}

#[test]
fn optimizer_should_preserve_break_continue_cfg_at_all_opt_levels() {
    let source = r#"
      export fn control(n: u32) -> u32 {
        let i: u32 = 0;
        let sum: u32 = 0;
        while i < n {
          i = i + 1;
          if i == 2 {
            continue;
          }
          sum = sum + i;
          if sum > 10 {
            break;
          }
        }
        return sum;
      }
    "#;

    for opt_level in 0..=3 {
        let text = optimize(source, opt_level, MirPassOverflowMode::Unchecked);
        assert!(text.contains("export fn control"), "O{opt_level}:\n{text}");
        assert!(text.contains("return"), "O{opt_level}:\n{text}");
    }
}

#[test]
fn optimizer_should_keep_targetless_calls_as_side_effects_at_all_levels() {
    let source = r#"
      fn write_one(out: ptr<i32>) -> void { out[0] = 1; }
      export fn run(out: ptr<i32>) -> void { write_one(out); }
    "#;

    for opt_level in 0..=3 {
        let text = optimize(source, opt_level, MirPassOverflowMode::Unchecked);
        assert!(
            text.contains("call write_one(out)"),
            "O{opt_level}:\n{text}"
        );
    }
}

#[test]
fn optimizer_should_handle_valueless_returns_in_cfg_passes() {
    let source = r#"
      export fn stop(flag: bool) -> void {
        if flag { return; }
        while true { break; }
      }
    "#;

    for opt_level in 0..=3 {
        let text = optimize(source, opt_level, MirPassOverflowMode::Unchecked);
        assert!(text.contains("-> void"), "O{opt_level}:\n{text}");
        assert!(text.contains("return"), "O{opt_level}:\n{text}");
    }
}

#[test]
fn optimizer_should_not_require_a_synthetic_void_temp() {
    let text = optimize(
        r#"
      fn no_op() -> void {}
      export fn run() -> void { no_op(); }
    "#,
        3,
        MirPassOverflowMode::Unchecked,
    );

    assert!(text.contains("  call no_op()"), "{text}");
    assert!(!text.contains(": void ="), "{text}");
    assert!(!text.contains("local ik_void"), "{text}");
}

#[test]
fn optimizer_should_conservatively_keep_void_helpers_without_panicking() {
    let source = r#"
      fn helper(out: ptr<i32>) -> void { out[0] = 7; }
      export fn run(out: ptr<i32>) -> void { helper(out); }
    "#;

    for opt_level in 2..=3 {
        let text = optimize(source, opt_level, MirPassOverflowMode::Unchecked);
        assert!(text.contains("fn helper"), "O{opt_level}:\n{text}");
        assert!(text.contains("call helper(out)"), "O{opt_level}:\n{text}");
    }
}

#[test]
fn optimizer_should_track_slice_instruction_and_place_uses() {
    let source = r#"
      fn cut(items: slice<i32>, start: u32, end: u32) -> slice<i32> {
        let middle: slice<i32> = items[start..end];
        let first: i32 = middle[0];
        return middle;
      }
    "#;

    for opt_level in 0..=3 {
        let text = optimize_with_bounds(
            source,
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Unchecked,
        );
        assert!(
            text.contains("subslice items, start, end"),
            "O{opt_level}:\n{text}"
        );
        assert!(
            text.contains("slice_index(middle,"),
            "O{opt_level}:\n{text}"
        );
        assert!(text.contains("return middle"), "O{opt_level}:\n{text}");
    }
}

#[test]
fn optimizer_should_keep_checked_subslice_even_when_result_is_dead() {
    let source = r#"
      export fn observe(items: slice<i32>, len: u32) -> void {
        let unused: slice<i32> = items[0..len];
      }
    "#;

    for opt_level in 1..=3 {
        let text = optimize_with_bounds(
            source,
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Checked,
        );
        assert!(text.contains("subslice items,"), "O{opt_level}:\n{text}");
    }
}

#[test]
fn optimizer_should_keep_checked_slice_index_address_observable() {
    let source = r#"
      struct Item { left: i32; right: i32; }
      export fn read(items: slice<Item>, index: u32) -> i32 {
        return items[index].left + items[index].right;
      }
    "#;

    for opt_level in 1..=3 {
        let text = optimize_with_bounds(
            source,
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Checked,
        );
        assert_eq!(
            text.matches("slice_index(items, index)").count(),
            2,
            "O{opt_level}:\n{text}"
        );
        assert!(
            !text.contains("address slice_index"),
            "O{opt_level}:\n{text}"
        );
    }
}

#[test]
fn optimizer_should_not_cse_or_hoist_checked_slice_guards_at_o1_through_o3() {
    let source = r#"
      export fn loop_cut(items: slice<i32>, len: u32) -> u32 {
        let i: u32 = 0;
        while i < len {
          let middle: slice<i32> = items[i..len];
          let value: i32 = middle[0];
          i = i + 1;
        }
        return i;
      }
    "#;

    for opt_level in 1..=3 {
        let text = optimize_with_bounds(
            source,
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Checked,
        );
        let loop_header = text.find("bb1:").expect("loop header");
        let subslice = text.find("subslice items, i, len").expect("subslice");
        assert!(subslice > loop_header, "O{opt_level}:\n{text}");
        assert_eq!(text.matches("subslice items, i, len").count(), 1);
    }
}

#[test]
fn optimizer_should_preserve_slice_internal_calls_and_returns() {
    let source = r#"
      fn identity(items: slice<i32>) -> slice<i32> { return items; }
      fn forward(items: slice<i32>) -> slice<i32> { return identity(items); }
    "#;

    for opt_level in 0..=3 {
        let text = optimize_with_bounds(
            source,
            opt_level,
            MirPassOverflowMode::Unchecked,
            MirPassBoundsMode::Checked,
        );
        assert!(
            text.contains("call identity(items)"),
            "O{opt_level}:\n{text}"
        );
        assert!(text.contains("-> slice<i32>"), "O{opt_level}:\n{text}");
    }
}
